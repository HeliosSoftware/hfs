//! Which scans read their resource document after a row fan-out.
//!
//! A compiled view reads the scanned resource's JSON document through IR roots
//! equal to [`RESOURCE_DOCUMENT_ROOT`]. After a row-multiplying unnest
//! (`forEach`, `forEachOrNull`) or a `repeat:` traversal, every read of that
//! root runs once per produced row, not once per resource. On PostgreSQL each
//! such read decodes (and, for large documents, de-TOASTs) the document again.
//!
//! [`mark_document_reads_after_fan_out`] runs once at the end of
//! [`build_plan`](crate::sof::compile_view::build_plan) and stores the answer on
//! each [`PlanNode::Scan`] as `document_read_after_fan_out`. The emitter decides
//! from that fact alone; it never inspects rendered SQL text.
//!
//! # Rule
//!
//! For a select statement (a `Project` over `Filter` / `LateralUnnest` layers
//! over the `Scan`), with the unnests numbered from the scan upward, a
//! *multiplying* unnest is one without `flat_index` whose source is not a
//! `ChainArray` with an `offset` (both are indexed picks, which yield at most
//! one row: the second is how SQL targets lower a trailing `[N]` / `first()`
//! with a nested `select`). The scan is marked iff
//!
//! - (a) an unnest **source** reads the document after an earlier multiplying
//!   unnest, or
//! - (b) an unnest **`on_filter`** reads the document and that unnest, or an
//!   earlier one, multiplies, or
//! - (c) a **projected column** reads the document and some unnest multiplies.
//!
//! `Filter` predicates never count: they only reference the resource row, so
//! PostgreSQL evaluates them once per resource wherever the node sits.
//!
//! Each `unionAll` branch is its own statement over its own `Scan` clone and is
//! marked independently.
//!
//! A statement over a [`PlanNode::Recurse`] (a `repeat:`) is never marked: its
//! flag, which would describe the rejoin to `resources`, stays false even when
//! the rejoin reads the document. The traversal does multiply the scan's rows,
//! but the measured rejoin plan bound the document once per traversal row and
//! saved no decode, so a bound rejoin was pure cost. The recursive seeds are not
//! IR nodes and are never marked either: a seed reads the document only in its
//! first unnest source.
//!
//! # Documented false negatives
//!
//! [`SqlExpr::ScalarFromChain`] and [`SqlExpr::ChainArray`] carry pre-rendered
//! SQL text, so a document read inside them is not visible here and is never
//! counted. Subquery variants are never built by the compiler and are refused
//! by the emitter. A false negative leaves the scan unmarked, which keeps its
//! SQL exactly as it was.

#[cfg(test)]
use super::{Column, JsonPath};
use super::{PlanNode, SqlExpr};

/// IR root of the scanned resource's JSON document.
///
/// The one root string this analysis compares against.
pub(crate) const RESOURCE_DOCUMENT_ROOT: &str = "r.data";

/// Whether an IR root names the scanned resource's document.
pub(crate) fn is_resource_document_root(root: &str) -> bool {
    root == RESOURCE_DOCUMENT_ROOT
}

/// Whether `expr`, or any expression nested in it, reads the scanned
/// resource's document.
///
/// The match is exhaustive on purpose: a new [`SqlExpr`] variant must decide
/// here whether it reads the document.
pub(crate) fn reads_resource_document(expr: &SqlExpr) -> bool {
    match expr {
        SqlExpr::JsonPath { root, .. } | SqlExpr::CollectionAgg { root, .. } => {
            is_resource_document_root(root)
        }
        SqlExpr::Cast { inner, .. }
        | SqlExpr::UnaryOp { inner, .. }
        | SqlExpr::AsJson(inner)
        | SqlExpr::Alias { inner, .. } => reads_resource_document(inner),
        SqlExpr::BinOp { lhs, rhs, .. } => {
            reads_resource_document(lhs) || reads_resource_document(rhs)
        }
        SqlExpr::Case { arms, else_ } => {
            arms.iter()
                .any(|(when, then)| reads_resource_document(when) || reads_resource_document(then))
                || else_.as_deref().is_some_and(reads_resource_document)
        }
        SqlExpr::Coalesce(parts) => parts.iter().any(reads_resource_document),
        SqlExpr::NullIf(a, b) => reads_resource_document(a) || reads_resource_document(b),
        SqlExpr::ReferenceKey { reference, .. } => reads_resource_document(reference),
        SqlExpr::Boundary { source, .. } => reads_resource_document(source),
        SqlExpr::WhereExists {
            focus, predicate, ..
        } => reads_resource_document(focus) || reads_resource_document(predicate),
        SqlExpr::WhereScalar {
            focus,
            predicate,
            projection,
            ..
        } => {
            reads_resource_document(focus)
                || reads_resource_document(predicate)
                || reads_resource_document(projection)
        }
        SqlExpr::JoinAggregate { outer_focus, .. } => reads_resource_document(outer_focus),
        // Pre-rendered SQL text: opaque, never counted (module docs).
        SqlExpr::ScalarFromChain { .. } | SqlExpr::ChainArray { .. } => false,
        // Never built by the compiler; the emitter refuses them.
        SqlExpr::JsonAgg(_) | SqlExpr::Scalar(_) | SqlExpr::Exists(_) | SqlExpr::CountSub(_) => {
            false
        }
        SqlExpr::Lit(_) | SqlExpr::Param(_) | SqlExpr::ColRef(_) | SqlExpr::RowIndex(_) => false,
    }
}

/// Sets `document_read_after_fan_out` on every [`PlanNode::Scan`] of `plan`
/// (module docs). Called once, at the end of `build_plan`.
pub(crate) fn mark_document_reads_after_fan_out(plan: &mut PlanNode) {
    #[cfg(test)]
    let enabled = !test_switch::binding_disabled();
    #[cfg(not(test))]
    let enabled = true;
    mark(plan, enabled);
}

fn mark(node: &mut PlanNode, enabled: bool) {
    match node {
        PlanNode::Union(branches) => {
            for branch in branches {
                mark(branch, enabled);
            }
        }
        PlanNode::Project { parent, columns } => {
            let (unnests, over_recurse) = statement_unnests(parent);
            // A `repeat:` rejoin is never marked (module docs).
            let fact = enabled
                && !over_recurse
                && reads_after_fan_out(false, &unnests, columns.iter().map(|column| &column.expr));
            if let Some(flag) = scan_flag_mut(parent) {
                *flag = fact;
            }
        }
        // Not a statement root; the emitter refuses such plans.
        PlanNode::Scan { .. }
        | PlanNode::LateralUnnest { .. }
        | PlanNode::Filter { .. }
        | PlanNode::Recurse { .. } => {}
    }
}

/// The document reads of one unnest layer, in scan-upward order.
struct UnnestReads<'a> {
    source: &'a SqlExpr,
    on_filter: Option<&'a SqlExpr>,
    multiplies: bool,
}

/// The unnest layers of a statement body from the scan upward, and whether the
/// body sits on a `Recurse` (whose traversal multiplies the scan's rows).
fn statement_unnests(body: &PlanNode) -> (Vec<UnnestReads<'_>>, bool) {
    let mut unnests = Vec::new();
    let mut node = body;
    let over_recurse = loop {
        match node {
            PlanNode::LateralUnnest {
                parent,
                source,
                on_filter,
                flat_index,
                ..
            } => {
                unnests.push(UnnestReads {
                    source,
                    on_filter: on_filter.as_ref(),
                    multiplies: flat_index.is_none() && !is_single_element_source(source),
                });
                node = parent;
            }
            // Filter predicates never count (module docs).
            PlanNode::Filter { parent, .. } => node = parent,
            PlanNode::Recurse { .. } => break true,
            PlanNode::Scan { .. } | PlanNode::Project { .. } | PlanNode::Union(_) => break false,
        }
    };
    unnests.reverse();
    (unnests, over_recurse)
}

/// Whether an unnest source is a one-element array: the `ChainArray` with an
/// `offset` that a trailing `[N]` / `first()` with a nested `select` unnests
/// on SQL targets. Like a `flat_index` pick, it yields at most one row.
fn is_single_element_source(source: &SqlExpr) -> bool {
    matches!(
        source,
        SqlExpr::ChainArray {
            offset: Some(_),
            ..
        }
    )
}

/// The rule of the module docs over one statement: `fanned` says whether the
/// rows are already multiplied below the first unnest (a `Recurse`).
fn reads_after_fan_out<'a>(
    mut fanned: bool,
    unnests: &[UnnestReads<'_>],
    mut projections: impl Iterator<Item = &'a SqlExpr>,
) -> bool {
    for unnest in unnests {
        // (a) A source is evaluated once per row of the joins before it.
        if fanned && reads_resource_document(unnest.source) {
            return true;
        }
        // (b) An ON filter is evaluated once per element of its own unnest.
        if (fanned || unnest.multiplies) && unnest.on_filter.is_some_and(reads_resource_document) {
            return true;
        }
        fanned |= unnest.multiplies;
    }
    // (c) Projections are evaluated once per final row.
    fanned && projections.any(reads_resource_document)
}

/// Whether the rejoin of a statement over a `Recurse` reads the document at
/// all (the traversal has already fanned the rows out), which is when the
/// rejoin is listed as a scan site.
#[cfg(test)]
fn rejoin_reads_document(columns: &[Column], body: &PlanNode) -> bool {
    let (unnests, _) = statement_unnests(body);
    reads_after_fan_out(true, &unnests, columns.iter().map(|column| &column.expr))
}

/// The scan flag a statement body (or a `Recurse` parent) ends in.
fn scan_flag_mut(node: &mut PlanNode) -> Option<&mut bool> {
    match node {
        PlanNode::Scan {
            document_read_after_fan_out,
            ..
        } => Some(document_read_after_fan_out),
        PlanNode::LateralUnnest { parent, .. }
        | PlanNode::Filter { parent, .. }
        | PlanNode::Recurse { parent, .. } => scan_flag_mut(parent),
        PlanNode::Project { .. } | PlanNode::Union(_) => None,
    }
}

#[cfg(test)]
fn scan_flag(node: &PlanNode) -> Option<bool> {
    match node {
        PlanNode::Scan {
            document_read_after_fan_out,
            ..
        } => Some(*document_read_after_fan_out),
        PlanNode::LateralUnnest { parent, .. }
        | PlanNode::Filter { parent, .. }
        | PlanNode::Recurse { parent, .. } => scan_flag(parent),
        PlanNode::Project { .. } | PlanNode::Union(_) => None,
    }
}

/// The step paths of the `Recurse` a statement body sits on.
#[cfg(test)]
fn recurse_step_paths(node: &PlanNode) -> Option<&[JsonPath]> {
    match node {
        PlanNode::Recurse { step_paths, .. } => Some(step_paths),
        PlanNode::LateralUnnest { parent, .. } | PlanNode::Filter { parent, .. } => {
            recurse_step_paths(parent)
        }
        PlanNode::Scan { .. } | PlanNode::Project { .. } | PlanNode::Union(_) => None,
    }
}

/// The fact of one recursive seed, derived by the select rule on the seed's
/// statement shape: one multiplying unnest whose source is the step path off
/// the document, projecting the resource id and the unnest value.
#[cfg(test)]
fn seed_reads_document_after_fan_out(step_path: &JsonPath) -> bool {
    let source = SqlExpr::JsonPath {
        root: RESOURCE_DOCUMENT_ROOT.to_string(),
        path: step_path.clone(),
    };
    let unnests = [UnnestReads {
        source: &source,
        on_filter: None,
        multiplies: true,
    }];
    let projections = [
        SqlExpr::ColRef("r.id".to_string()),
        SqlExpr::JsonPath {
            root: "je.value".to_string(),
            path: JsonPath::new(),
        },
    ];
    reads_after_fan_out(false, &unnests, projections.iter())
}

/// One SQL scan of `resources` that a plan emits.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScanSite {
    /// The single select statement of a plan without `unionAll`.
    Select,
    /// The select statement of the `unionAll` branch at this index.
    UnionBranch(usize),
    /// The seed of a `repeat:` for the step path at `step`, in the `unionAll`
    /// branch at `branch` when there is one.
    RepeatSeed {
        /// `unionAll` branch index, if any.
        branch: Option<usize>,
        /// Index into the `Recurse` step paths.
        step: usize,
    },
    /// The rejoin of a `repeat:` traversal to `resources`, listed only when the
    /// analysis finds a document read there. Its fact is always false: the
    /// rejoin is never marked (module docs).
    RepeatRejoin {
        /// `unionAll` branch index, if any.
        branch: Option<usize>,
    },
}

/// Every SQL scan site of `plan` with its fact, in emission order.
///
/// Select, branch and rejoin facts are the stored scan flags (a rejoin's is
/// never set). Seed facts are
/// derived by the select rule on the seed's statement shape (seeds have no IR
/// node to store a flag on, and the emitter never consults one).
#[cfg(test)]
pub(crate) fn scan_sites(plan: &PlanNode) -> Vec<(ScanSite, bool)> {
    let mut sites = Vec::new();
    collect_sites(plan, None, &mut sites);
    sites
}

#[cfg(test)]
fn collect_sites(node: &PlanNode, branch: Option<usize>, sites: &mut Vec<(ScanSite, bool)>) {
    match node {
        PlanNode::Union(branches) => {
            for (index, branch_plan) in branches.iter().enumerate() {
                collect_sites(branch_plan, Some(index), sites);
            }
        }
        PlanNode::Project { parent, columns } => {
            let stored = scan_flag(parent).unwrap_or(false);
            if let Some(step_paths) = recurse_step_paths(parent) {
                for (step, path) in step_paths.iter().enumerate() {
                    sites.push((
                        ScanSite::RepeatSeed { branch, step },
                        seed_reads_document_after_fan_out(path),
                    ));
                }
                if rejoin_reads_document(columns, parent) {
                    sites.push((ScanSite::RepeatRejoin { branch }, stored));
                }
            } else {
                let site = branch.map_or(ScanSite::Select, ScanSite::UnionBranch);
                sites.push((site, stored));
            }
        }
        PlanNode::Scan { .. }
        | PlanNode::LateralUnnest { .. }
        | PlanNode::Filter { .. }
        | PlanNode::Recurse { .. } => {}
    }
}

/// Test-only override that turns marking off, so a test can build the
/// unmarked (main-branch) form of a plan through the production compile path.
///
/// The override is a thread-local read only inside
/// [`mark_document_reads_after_fan_out`], that is, while `build_plan` runs.
/// Release builds contain no override.
#[cfg(test)]
mod test_switch {
    use std::cell::Cell;
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    thread_local! {
        static DISABLED: Cell<bool> = const { Cell::new(false) };
    }

    pub(super) fn binding_disabled() -> bool {
        DISABLED.with(Cell::get)
    }

    /// Restores the previous override value when dropped, also on unwind.
    struct Restore(bool);

    impl Drop for Restore {
        fn drop(&mut self) {
            DISABLED.with(|cell| cell.set(self.0));
        }
    }

    fn disable() -> Restore {
        Restore(DISABLED.with(|cell| cell.replace(true)))
    }

    /// Runs a **synchronous** compilation with marking off.
    ///
    /// Do not wrap the construction of a future (for example an `async fn`
    /// call) in this: `build_plan` runs on the future's first poll, after the
    /// override is already restored, so the plan would be marked. Use
    /// [`without_document_binding_in`] for futures.
    pub(crate) fn without_document_binding<R>(compile: impl FnOnce() -> R) -> R {
        let _restore = disable();
        compile()
    }

    /// Wraps a future so marking is off during each of its polls.
    ///
    /// The override is set at the start of every poll and restored when that
    /// poll returns or unwinds, so nothing stays set between polls: other
    /// tasks on the thread are unaffected, and a poll that moves to another
    /// worker thread still sees it.
    pub(crate) fn without_document_binding_in<F: Future>(future: F) -> WithoutDocumentBinding<F> {
        WithoutDocumentBinding {
            inner: Box::pin(future),
        }
    }

    /// Future returned by [`without_document_binding_in`].
    pub(crate) struct WithoutDocumentBinding<F> {
        inner: Pin<Box<F>>,
    }

    impl<F: Future> Future for WithoutDocumentBinding<F> {
        type Output = F::Output;

        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<F::Output> {
            let _restore = disable();
            self.inner.as_mut().poll(cx)
        }
    }
}

#[cfg(test)]
pub(crate) use test_switch::{without_document_binding, without_document_binding_in};

#[cfg(all(test, feature = "R4"))]
mod tests {
    use std::cell::Cell;
    use std::panic::AssertUnwindSafe;
    use std::pin::Pin;

    use helios_fhir::FhirVersion;
    use serde_json::{Value, json};

    use super::super::{Column, LitValue, PathStep, SqlType};
    use super::*;
    use crate::sof::compile_view::build_plan;
    use crate::sof::compiler::CompileTarget;
    use crate::sof::decode::ColumnDecode;
    use crate::sof::dialect::PgDialect;

    use ScanSite::{RepeatRejoin, RepeatSeed, Select, UnionBranch};

    fn view(resource: &str, select: Value) -> Value {
        json!({"resourceType":"ViewDefinition","resource":resource,"status":"active","select":select})
    }

    fn plan_of(view: &Value) -> PlanNode {
        build_plan(view, &PgDialect, CompileTarget::Postgres, FhirVersion::R4)
            .unwrap_or_else(|e| panic!("build_plan failed for {view}: {e:?}"))
            .0
    }

    fn sites_of(view: &Value) -> Vec<(ScanSite, bool)> {
        scan_sites(&plan_of(view))
    }

    fn col(path: &str, name: &str) -> Value {
        json!({"path": path, "name": name})
    }

    fn seed(step: usize) -> ScanSite {
        RepeatSeed { branch: None, step }
    }

    /// The golden `repeat` view: root columns next to a two-path `repeat`.
    fn repeat_view() -> Value {
        view(
            "QuestionnaireResponse",
            json!([
                {"column":[col("id","id"),{"path":"item.linkId","name":"roots","collection":true}]},
                {"repeat":["item","answer.item"],"column":[col("linkId","link"),
                    {"path":"answer.valueString","name":"answers","collection":true},
                    {"path":"answer.valueString.join('|')","name":"joined","type":"string"},
                    {"path":"answer.where(valueString != 'skip').first().valueString","name":"picked","type":"string"}]}
            ]),
        )
    }

    /// The bench `observation_expanded` view.
    fn observation_expanded_view() -> Value {
        view(
            "Observation",
            json!([
                {"column":[{"name":"id","path":"getResourceKey()","type":"id"},
                           {"name":"patient_id","path":"subject.getReferenceKey(Patient)","type":"id"}]},
                {"forEach":"component","column":[
                    {"name":"component_code","path":"code.coding.first().code","type":"string"},
                    {"name":"component_name","path":"code.text","type":"string"}]}
            ]),
        )
    }

    /// A matrix row: case name, view, expected sites with their facts.
    type FactCase = (&'static str, Value, Vec<(ScanSite, bool)>);

    fn fact_table() -> Vec<FactCase> {
        let patient = |select: Value| view("Patient", select);
        let mut flat = patient(json!([{"column":[col("id","id"),col("name.family","family")]}]));
        flat["where"] = json!([{"path":"active = true"}]);
        let mut foreach_where =
            patient(json!([{"forEach":"name","column":[col("family","family")]}]));
        foreach_where["where"] = json!([{"path":"active = true"}]);
        vec![
            ("flat + where", flat, vec![(Select, false)]),
            (
                "single forEach, no root column",
                patient(json!([{"forEach":"name","column":[col("family","family")]}])),
                vec![(Select, false)],
            ),
            (
                "single forEach + where (Filter never counts)",
                foreach_where,
                vec![(Select, false)],
            ),
            (
                "single forEach + root column (golden foreach)",
                patient(
                    json!([{"column":[col("id","r")]},{"forEach":"name","column":[col("family","u0")]}]),
                ),
                vec![(Select, true)],
            ),
            (
                "observation_expanded",
                observation_expanded_view(),
                vec![(Select, true)],
            ),
            (
                "nested (golden nested)",
                patient(
                    json!([{"forEach":"name","select":[{"column":[col("family","family")]},
                    {"forEach":"given","column":[{"path":"$this","name":"given","type":"string"}]}]}]),
                ),
                vec![(Select, false)],
            ),
            (
                "chained (golden chained)",
                patient(
                    json!([{"forEach":"name.given","column":[{"path":"$this","name":"given","type":"string"}]}]),
                ),
                vec![(Select, false)],
            ),
            (
                "cartesian (golden cartesian)",
                patient(json!([{"forEach":"name","column":[col("family","family")]},
                    {"forEach":"address","column":[col("city","city")]}])),
                vec![(Select, true)],
            ),
            (
                "forEachOrNull single step, no root column (golden nullable)",
                patient(
                    json!([{"forEachOrNull":"name.where(use = 'official')","column":[col("family","family")]}]),
                ),
                vec![(Select, false)],
            ),
            (
                "forEachOrNull single step + root column",
                patient(
                    json!([{"column":[col("id","id")]},{"forEachOrNull":"name","column":[col("family","family")]}]),
                ),
                vec![(Select, true)],
            ),
            (
                "forEachOrNull multi-step, no root column",
                patient(
                    json!([{"forEachOrNull":"contact.telecom","column":[col("value","value")]}]),
                ),
                vec![(Select, false)],
            ),
            (
                "forEachOrNull multi-step + root column",
                patient(json!([{"column":[col("id","id")]},
                    {"forEachOrNull":"contact.telecom","column":[col("value","value")]}])),
                vec![(Select, true)],
            ),
            (
                "limitation: forEach then forEachOrNull multi-step (opaque ChainArray)",
                patient(json!([{"forEach":"name","column":[col("family","family")]},
                    {"forEachOrNull":"contact.telecom","column":[col("value","value")]}])),
                vec![(Select, false)],
            ),
            (
                "limitation: forEach then forEach telecom[0] (opaque ScalarFromChain + Filter)",
                patient(json!([{"forEach":"name","column":[col("family","family")]},
                    {"forEach":"telecom[0]","column":[col("value","value")]}])),
                vec![(Select, false)],
            ),
            (
                "indexed (golden indexed)",
                patient(json!([{"forEachOrNull":"name[1]","column":[col("family","family")]}])),
                vec![(Select, false)],
            ),
            // A trailing `[N]` / `first()` with a nested `select` unnests a
            // one-element `ChainArray` (`offset`): at most one row, like a
            // `flat_index` pick, so a root column alone does not mark.
            (
                "indexed forEach + nested column-only select + root column (IndexedForEachNestedColumns)",
                patient(json!([{"column":[col("id","id")]},
                    {"forEach":"name[0]","select":[{"column":[col("use","u")]}]}])),
                vec![(Select, false)],
            ),
            (
                "indexed forEachOrNull + nested column-only select + root column",
                patient(json!([{"column":[col("id","id")]},
                    {"forEachOrNull":"name[1]","select":[{"column":[col("family","fam")]}]}])),
                vec![(Select, false)],
            ),
            (
                "filtered first() forEach + nested column-only select + root column",
                patient(json!([{"column":[col("id","id")]},
                    {"forEach":"name.where(use = 'official').first()",
                     "select":[{"column":[col("family","fam")]}]}])),
                vec![(Select, false)],
            ),
            (
                "union: indexed nested column-only branch, expanding branch + shared root column",
                patient(json!([{"column":[col("id","id")],"unionAll":[
                    {"forEach":"name[0]","select":[{"column":[col("family","value")]}]},
                    {"forEach":"telecom","column":[col("value","value")]}]}])),
                vec![(UnionBranch(0), false), (UnionBranch(1), true)],
            ),
            (
                "control: indexed forEach + nested expanding forEach + root column",
                patient(json!([{"column":[col("id","id")]},
                    {"forEach":"name[0]","select":[{"column":[col("family","fam")]},
                        {"forEach":"given","column":[{"path":"$this","name":"g","type":"string"}]}]}])),
                vec![(Select, true)],
            ),
            (
                "%rowIndex (golden row-index-legacy)",
                patient(json!([{"forEach":"name","column":[col("family","family"),
                    {"path":"%rowIndex","name":"index","type":"integer"}]}])),
                vec![(Select, false)],
            ),
            (
                "foreach-cells (golden foreach-cells)",
                patient(json!([{"forEach":"contact","column":[
                    {"path":"telecom","name":"objects","collection":true},
                    {"path":"telecom.value","name":"values","collection":true},
                    {"path":"telecom.value.join('|')","name":"joined","type":"string"},
                    {"path":"telecom.where(system = 'phone').first().value","name":"pick","type":"string"}]}])),
                vec![(Select, false)],
            ),
            (
                "union (golden union)",
                patient(json!([{"unionAll":[{"column":[col("id","value")]},
                    {"forEach":"name","column":[col("family","value")]}]}])),
                vec![(UnionBranch(0), false), (UnionBranch(1), false)],
            ),
            (
                "union + shared root column",
                patient(json!([{"column":[col("id","id")],"unionAll":[
                    {"forEach":"telecom","column":[col("value","value")]},
                    {"column":[col("gender","value")]}]}])),
                vec![(UnionBranch(0), true), (UnionBranch(1), false)],
            ),
            (
                "union + shared root column, two marked branches (patient_union_rooted)",
                patient(
                    json!([{"column":[{"name":"id","path":"getResourceKey()","type":"id"}],"unionAll":[
                    {"forEach":"name","column":[{"name":"value","path":"family","type":"string"}]},
                    {"forEach":"address","column":[{"name":"value","path":"city","type":"string"}]}]}]),
                ),
                vec![(UnionBranch(0), true), (UnionBranch(1), true)],
            ),
            // Repeat rows: a rejoin that reads the document is listed, but it
            // is never marked (module docs), whatever reads it.
            (
                "repeat + root columns (golden repeat)",
                repeat_view(),
                vec![
                    (seed(0), false),
                    (seed(1), false),
                    (RepeatRejoin { branch: None }, false),
                ],
            ),
            (
                "repeat without root column",
                view(
                    "QuestionnaireResponse",
                    json!([{"repeat":["item"],"column":[col("linkId","link")]}]),
                ),
                vec![(seed(0), false)],
            ),
            (
                "repeat + nested forEachOrNull over the node + root column",
                view(
                    "QuestionnaireResponse",
                    json!([{"column":[col("id","id")]},
                    {"repeat":["item"],"column":[col("linkId","link")],
                     "select":[{"forEachOrNull":"answer","column":[col("valueString","answer")]}]}]),
                ),
                vec![(seed(0), false), (RepeatRejoin { branch: None }, false)],
            ),
            (
                "union with a repeat branch + root column",
                view(
                    "QuestionnaireResponse",
                    json!([{"column":[col("id","id")],"unionAll":[
                    {"repeat":["item"],"column":[col("linkId","value")]},
                    {"forEach":"item","column":[col("linkId","value")]}]}]),
                ),
                vec![
                    (
                        RepeatSeed {
                            branch: Some(0),
                            step: 0,
                        },
                        false,
                    ),
                    (RepeatRejoin { branch: Some(0) }, false),
                    (UnionBranch(1), true),
                ],
            ),
            (
                // BD1: the emitter's `column_refers_to_resource` misses
                // `WhereExists`; the analysis counts it, so the rejoin is
                // listed, and still not marked.
                "BD1: repeat + sibling root column item.where(..).exists()",
                view(
                    "QuestionnaireResponse",
                    json!([
                    {"column":[{"path":"item.where(linkId = 'a').exists()","name":"has_a","type":"boolean"}]},
                    {"repeat":["item"],"column":[col("linkId","link")]}]),
                ),
                vec![(seed(0), false), (RepeatRejoin { branch: None }, false)],
            ),
            (
                // BD2: the sibling forEach sits below the traversal (one
                // keyed `repeat:` statement over `resources r`), so there is
                // no rejoin to list; the seed is listed, not marked.
                "BD2: repeat + sibling forEach rooted at the document",
                view(
                    "QuestionnaireResponse",
                    json!([
                    {"forEach":"item","column":[col("linkId","top")]},
                    {"repeat":["item"],"column":[col("linkId","link")]}]),
                ),
                vec![(seed(0), false)],
            ),
        ]
    }

    #[test]
    fn fact_per_scan_site() {
        let mut failures = Vec::new();
        for (name, view, expected) in fact_table() {
            let got = sites_of(&view);
            println!("{name}: {got:?}");
            if got != expected {
                failures.push(format!("{name}: expected {expected:?}, got {got:?}"));
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    fn column(expr: SqlExpr) -> Column {
        Column {
            name: "c".to_string(),
            expr,
            collection: false,
            ty: SqlType::Text,
            decode: ColumnDecode::Text,
        }
    }

    fn path(root: &str, field: &str) -> SqlExpr {
        SqlExpr::JsonPath {
            root: root.to_string(),
            path: JsonPath(vec![PathStep::Field(field.to_string())]),
        }
    }

    fn scan() -> PlanNode {
        PlanNode::Scan {
            alias: "r".to_string(),
            resource_type: "Patient".to_string(),
            filter: Default::default(),
            document_read_after_fan_out: false,
        }
    }

    fn unnest(
        parent: PlanNode,
        source: SqlExpr,
        on_filter: Option<SqlExpr>,
        flat_index: Option<i64>,
    ) -> PlanNode {
        PlanNode::LateralUnnest {
            parent: Box::new(parent),
            source,
            out_alias: "fe1".to_string(),
            left_join: false,
            on_filter,
            flat_index,
        }
    }

    fn marked(mut plan: PlanNode) -> Vec<(ScanSite, bool)> {
        mark_document_reads_after_fan_out(&mut plan);
        scan_sites(&plan)
    }

    /// Rule (b), not reachable from today's compiler: an ON filter of the
    /// first unnest reads the document once per element of that unnest.
    #[test]
    fn on_filter_reading_the_document_on_the_first_unnest_marks() {
        let on_filter = SqlExpr::BinOp {
            op: super::super::BinOp::Eq,
            lhs: Box::new(path("r.data", "gender")),
            rhs: Box::new(SqlExpr::Lit(LitValue::Str("male".to_string()))),
        };
        let plan = PlanNode::Project {
            parent: Box::new(unnest(
                scan(),
                path("r.data", "name"),
                Some(on_filter),
                None,
            )),
            columns: vec![column(path("fe1.value", "family"))],
        };
        assert_eq!(marked(plan), vec![(Select, true)]);
    }

    /// An indexed pick yields at most one row, so a root projection after it
    /// still reads the document once per resource.
    #[test]
    fn flat_index_unnest_does_not_multiply() {
        let plan = PlanNode::Project {
            parent: Box::new(unnest(scan(), path("r.data", "name"), None, Some(0))),
            columns: vec![
                column(path("r.data", "id")),
                column(path("fe1.value", "family")),
            ],
        };
        assert_eq!(marked(plan), vec![(Select, false)]);
    }

    /// A `ChainArray` with an `offset` is a one-element array (the indexed
    /// pick of a trailing `[N]` with a nested `select`), so it does not
    /// multiply either; without an `offset` it is the whole flattened chain
    /// and does.
    #[test]
    fn offset_chain_array_unnest_does_not_multiply() {
        let chain = |offset| SqlExpr::ChainArray {
            chain_sql: "jsonb_array_elements(r.data->'name') AS c0(value)".to_string(),
            leaf_alias: "c0".to_string(),
            filter: None,
            offset,
            scope_root: RESOURCE_DOCUMENT_ROOT.to_string(),
        };
        let plan = |offset| PlanNode::Project {
            parent: Box::new(unnest(scan(), chain(offset), None, None)),
            columns: vec![
                column(path("r.data", "id")),
                column(path("fe1.value", "family")),
            ],
        };
        assert_eq!(marked(plan(Some(0))), vec![(Select, false)]);
        assert_eq!(marked(plan(None)), vec![(Select, true)]);
    }

    /// A rejoin is never marked, even when every read kind hits the document
    /// after the traversal: a projection, an unnest source and its ON filter.
    #[test]
    fn rejoin_is_never_marked() {
        // The flag starts set, so the pass has to clear it.
        let mut stale = scan();
        if let PlanNode::Scan {
            document_read_after_fan_out,
            ..
        } = &mut stale
        {
            *document_read_after_fan_out = true;
        }
        let recurse = PlanNode::Recurse {
            parent: Box::new(stale),
            seed_root: "r.data".to_string(),
            step_paths: vec![JsonPath(vec![PathStep::Field("item".to_string())])],
            out_alias: "rec_1".to_string(),
        };
        let on_filter = SqlExpr::BinOp {
            op: super::super::BinOp::Eq,
            lhs: Box::new(path("r.data", "status")),
            rhs: Box::new(SqlExpr::Lit(LitValue::Str("completed".to_string()))),
        };
        let plan = PlanNode::Project {
            parent: Box::new(unnest(
                recurse,
                path("r.data", "item"),
                Some(on_filter),
                None,
            )),
            columns: vec![
                column(path("r.data", "id")),
                column(path("rec_1", "linkId")),
            ],
        };
        assert_eq!(
            marked(plan),
            vec![(seed(0), false), (RepeatRejoin { branch: None }, false)]
        );
    }

    /// String literals that spell the document root are not reads.
    #[test]
    fn literals_spelling_the_root_are_not_reads() {
        let literal = SqlExpr::Lit(LitValue::Str("r.data".to_string()));
        assert!(!reads_resource_document(&literal));
        let plan = PlanNode::Project {
            parent: Box::new(unnest(scan(), path("r.data", "name"), None, None)),
            columns: vec![column(literal)],
        };
        assert_eq!(marked(plan), vec![(Select, false)]);
        assert!(is_resource_document_root(RESOURCE_DOCUMENT_ROOT));
        assert!(!is_resource_document_root("fe1.value"));
    }

    #[cfg(feature = "mongodb")]
    #[test]
    fn mongo_target_build_has_the_same_fact() {
        use crate::sof::dialect::SqliteDialect;
        let cartesian = view(
            "Patient",
            json!([{"forEach":"name","column":[col("family","family")]},
                {"forEach":"address","column":[col("city","city")]}]),
        );
        let (plan, _) = build_plan(
            &cartesian,
            &SqliteDialect,
            CompileTarget::Mongo,
            FhirVersion::R4,
        )
        .unwrap();
        assert_eq!(scan_sites(&plan), vec![(Select, true)]);
    }

    /// Every stored flag of `plan`, for the switch tests.
    fn stored_facts(view: &Value) -> Vec<bool> {
        sites_of(view)
            .into_iter()
            .filter(|(site, _)| !matches!(site, RepeatSeed { .. }))
            .map(|(_, fact)| fact)
            .collect()
    }

    fn marked_views() -> Vec<Value> {
        let table = fact_table();
        table
            .into_iter()
            .filter(|(_, _, expected)| {
                expected
                    .iter()
                    .any(|(site, fact)| *fact && !matches!(site, RepeatSeed { .. }))
            })
            .map(|(_, view, _)| view)
            .collect()
    }

    fn facts_of_plan(plan: &PlanNode) -> Vec<bool> {
        scan_sites(plan)
            .into_iter()
            .filter(|(site, _)| !matches!(site, RepeatSeed { .. }))
            .map(|(_, fact)| fact)
            .collect()
    }

    /// Builds the foreach view after yielding to the scheduler, so the
    /// compile runs on a later poll than the future's construction.
    async fn compile_after_yield(view: Value) -> PlanNode {
        tokio::task::yield_now().await;
        plan_of(&view)
    }

    fn foreach_view() -> Value {
        view(
            "Patient",
            json!([{"column":[col("id","r")]},{"forEach":"name","column":[col("family","u0")]}]),
        )
    }

    #[test]
    fn switch_sync_unmarks_inside_and_restores_outside() {
        let views = marked_views();
        assert!(views.len() >= 8, "fact table lost its marked views");
        for view in &views {
            let inside = without_document_binding(|| stored_facts(view));
            assert!(inside.iter().all(|fact| !fact), "{view}: {inside:?}");
            let outside = stored_facts(view);
            assert!(outside.iter().any(|fact| *fact), "{view}: {outside:?}");
        }
        // Nesting restores the outer value, not "off".
        without_document_binding(|| {
            without_document_binding(|| assert!(test_switch::binding_disabled()));
            assert!(test_switch::binding_disabled());
            assert_eq!(facts_of_plan(&plan_of(&foreach_view())), vec![false]);
        });
        assert!(!test_switch::binding_disabled());
        assert_eq!(facts_of_plan(&plan_of(&foreach_view())), vec![true]);
    }

    #[tokio::test]
    async fn switch_async_wrapper_survives_a_yield_and_the_sync_form_does_not() {
        let wrapped = without_document_binding_in(compile_after_yield(foreach_view())).await;
        assert_eq!(facts_of_plan(&wrapped), vec![false]);
        // The forbidden form: the override is gone before the first poll.
        let hazard = without_document_binding(|| compile_after_yield(foreach_view())).await;
        assert_eq!(facts_of_plan(&hazard), vec![true]);
        assert!(!test_switch::binding_disabled());
    }

    /// Two compiles that each yield between steps, interleaved on one task.
    async fn interleaved() -> (Vec<Vec<bool>>, Vec<Vec<bool>>) {
        async fn twice(view: Value) -> Vec<Vec<bool>> {
            let first = compile_after_yield(view.clone()).await;
            let second = compile_after_yield(view).await;
            vec![facts_of_plan(&first), facts_of_plan(&second)]
        }
        tokio::join!(
            twice(foreach_view()),
            without_document_binding_in(twice(foreach_view()))
        )
    }

    #[tokio::test(flavor = "current_thread")]
    async fn switch_is_poll_scoped_on_a_current_thread_runtime() {
        let (bound, unbound) = interleaved().await;
        assert_eq!(bound, vec![vec![true], vec![true]]);
        assert_eq!(unbound, vec![vec![false], vec![false]]);
        assert!(!test_switch::binding_disabled());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn switch_is_poll_scoped_on_a_multi_thread_runtime() {
        let mut handles = Vec::new();
        for _ in 0..8 {
            handles.push(tokio::spawn(interleaved()));
        }
        for handle in handles {
            let (bound, unbound) = handle.await.unwrap();
            assert_eq!(bound, vec![vec![true], vec![true]]);
            assert_eq!(unbound, vec![vec![false], vec![false]]);
        }
    }

    fn poll_once<F: std::future::Future>(future: Pin<&mut F>) -> std::task::Poll<F::Output> {
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        future.poll(&mut cx)
    }

    #[test]
    fn switch_is_off_after_a_panic_inside_the_wrapped_future() {
        let seen = Cell::new(false);
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let mut future = without_document_binding_in(async {
                seen.set(test_switch::binding_disabled());
                panic!("compile panicked");
            });
            let _ = poll_once(Pin::new(&mut future));
        }));
        assert!(result.is_err());
        assert!(seen.get(), "the override was not on during the poll");
        assert!(!test_switch::binding_disabled());
    }

    #[test]
    fn switch_is_off_while_a_wrapped_future_is_pending_and_after_it_is_dropped() {
        let seen = Cell::new(false);
        let mut future = without_document_binding_in(async {
            seen.set(test_switch::binding_disabled());
            std::future::pending::<()>().await;
        });
        assert!(poll_once(Pin::new(&mut future)).is_pending());
        assert!(seen.get(), "the override was not on during the poll");
        assert!(!test_switch::binding_disabled());
        drop(future);
        assert!(!test_switch::binding_disabled());
    }
}
