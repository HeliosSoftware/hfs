//! Discriminator → slice-match translation.
//!
//! Discriminator paths are the FHIR "restricted FHIRPath" subset: dotted
//! element selections, `$this`, `extension('url')`, `ofType(Type)`, and
//! `resolve()`. All but `resolve()` are parsed and compiled here;
//! `resolve()` needs the instance graph and stays unsupported (slice kept
//! without match/min).
//!
//! A `value`/`pattern` discriminator at path `P` becomes a partial-match
//! pattern built from the `fixed[x]`/`pattern[x]` constant found at `P`
//! inside the slice's subtree (`$this` meaning the item root). Paths that
//! traverse `extension('url')` compile to a dedicated `extension` matcher
//! instead (array containment is not expressible as a plain pattern).
//!
//! `exists` discriminators compile to an `exists` matcher: expected
//! presence/absence per path, read from the slice differential (`min >= 1`
//! ⇒ must exist, `max = 0` ⇒ must be absent).
//!
//! `type`, `profile`, and `binding` discriminators become typed [`Match`]
//! values evaluated at runtime by the engine.

use super::EdDiscriminator;
use super::tree::{SliceNode, capitalize, finalize};
use crate::schema::{Match, Slice, Slicing};
use indexmap::IndexMap;
use serde_json::{Map, Value, json};
use std::sync::Arc;

pub(super) fn build_slicing(
    slices: IndexMap<String, SliceNode>,
    discriminators: &[EdDiscriminator],
    rules: Option<String>,
    ordered: Option<bool>,
    warnings: &mut Vec<String>,
) -> Option<Slicing> {
    let mut out: IndexMap<String, Slice> = IndexMap::new();
    for (name, slice_node) in slices {
        let SliceNode {
            node,
            min,
            max,
            extension_profile,
            reslice,
            slice_is_constraining,
        } = slice_node;

        let match_ = build_match(&node, discriminators, extension_profile.as_deref());
        // Constraining slices inherit the parent's matcher; without a match
        // (and without reslice/constraining) nothing can ever match → drop min.
        let can_match =
            match_.is_some() || reslice.is_some() || slice_is_constraining == Some(true);
        if match_.is_none() && !can_match {
            warnings.push(format!(
                "slice '{name}': discriminator(s) {:?} not translatable to a match; \
                 slice kept without match or min",
                discriminators
                    .iter()
                    .map(|d| format!("{}@{}", d.type_, d.path))
                    .collect::<Vec<_>>()
            ));
        }

        let schema = finalize(node, warnings);
        out.insert(
            name,
            Slice {
                min: if can_match { min } else { None },
                max,
                match_,
                order: None,
                reslice,
                slice_is_constraining,
                schema: schema.map(Arc::new),
            },
        );
    }

    if out.is_empty() {
        return None;
    }
    Some(Slicing {
        slices: out,
        rules,
        ordered,
    })
}

// ---------------------------------------------------------------------------
// Restricted-FHIRPath discriminator paths
// ---------------------------------------------------------------------------

/// One parsed segment of a restricted discriminator path.
#[derive(Debug, Clone, PartialEq)]
enum DSeg {
    /// Plain element selection (`code`, `component`).
    Key(String),
    /// `extension('url')` — select a particular extension by url.
    Extension(String),
    /// `ofType(Type)` — pick one branch of a choice element.
    OfType(String),
    /// `resolve()` — follow a Reference; unsupported.
    Resolve,
}

/// A resolved segment of the runtime match path: choice elements and
/// `ofType()` collapse into the concrete JSON key.
#[derive(Debug, Clone, PartialEq)]
enum RSeg {
    Key(String),
    Extension(String),
}

/// Parse a discriminator path. `$this` (alone or as a leading segment)
/// contributes no segments. Returns `None` on anything outside the
/// restricted grammar.
fn parse_disc_path(path: &str) -> Option<Vec<DSeg>> {
    let mut out = Vec::new();
    for raw in split_top_level(path) {
        let raw = raw.trim();
        if raw == "$this" {
            continue;
        }
        if raw == "resolve()" {
            out.push(DSeg::Resolve);
            continue;
        }
        if let Some(url) = function_arg(raw, "extension") {
            out.push(DSeg::Extension(url));
            continue;
        }
        if let Some(ty) = function_arg(raw, "ofType") {
            out.push(DSeg::OfType(ty));
            continue;
        }
        if raw.is_empty() || raw.contains('(') || raw.contains(')') {
            return None;
        }
        out.push(DSeg::Key(raw.to_string()));
    }
    Some(out)
}

/// Split on top-level `.` only — extension urls contain dots inside `(...)`.
fn split_top_level(path: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (i, c) in path.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            '.' if depth == 0 => {
                out.push(&path[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&path[start..]);
    out
}

/// `function_arg("extension('http://x')", "extension")` → `Some("http://x")`.
fn function_arg(segment: &str, name: &str) -> Option<String> {
    let inner = segment
        .strip_prefix(name)?
        .strip_prefix('(')?
        .strip_suffix(')')?;
    let arg = inner.trim().trim_matches(|c| c == '\'' || c == '"');
    if arg.is_empty() {
        return None;
    }
    Some(arg.to_string())
}

/// Navigate parsed segments through the slice subtree, resolving choice
/// elements and `ofType()` to concrete JSON keys and `extension('url')` to
/// the matching extension sub-slice.
fn resolve_path<'a>(
    node: &'a super::tree::Node,
    segs: &[DSeg],
) -> Option<(&'a super::tree::Node, Vec<RSeg>)> {
    let mut current = node;
    let mut out = Vec::new();
    let mut i = 0;
    while i < segs.len() {
        match &segs[i] {
            DSeg::Resolve => return None,
            // `ofType()` is consumed with the key it follows; a leading one
            // has nothing to type-select.
            DSeg::OfType(_) => return None,
            DSeg::Extension(url) => {
                let ext = current.children.get("extension")?;
                let slice = find_extension_slice(ext, url)?;
                current = &slice.node;
                out.push(RSeg::Extension(url.clone()));
                i += 1;
            }
            DSeg::Key(name) => {
                let key = match segs.get(i + 1) {
                    Some(DSeg::OfType(ty)) => {
                        i += 1;
                        format!("{name}{}", capitalize(ty))
                    }
                    _ => name.clone(),
                };
                let (child, resolved) = child_resolving_choice(current, &key)?;
                current = child;
                out.push(RSeg::Key(resolved));
                i += 1;
            }
        }
    }
    Some((current, out))
}

/// Child lookup that sees through choice elements: a declarer with a single
/// branch (`value` → `valueQuantity`) resolves to that branch, and a bare
/// branch (`valueQuantity` present without its declarer) is found via
/// `choiceOf` when it is unambiguous.
fn child_resolving_choice<'a>(
    node: &'a super::tree::Node,
    name: &str,
) -> Option<(&'a super::tree::Node, String)> {
    if let Some(child) = node.children.get(name) {
        if let Some(choices) = &child.schema.choices
            && choices.len() == 1
            && let Some(branch) = node.children.get(&choices[0])
        {
            return Some((branch, choices[0].clone()));
        }
        return Some((child, name.to_string()));
    }
    let mut branches = node
        .children
        .iter()
        .filter(|(_, c)| c.schema.choice_of.as_deref() == Some(name));
    match (branches.next(), branches.next()) {
        (Some((key, child)), None) => Some((child, key.clone())),
        _ => None,
    }
}

/// Find the extension sub-slice discriminated by `url` — either sliced by
/// type profile (sugar) or by a fixed/pattern `url` child.
fn find_extension_slice<'a>(ext: &'a super::tree::Node, url: &str) -> Option<&'a SliceNode> {
    ext.slices.values().find(|s| {
        s.extension_profile.as_deref() == Some(url)
            || s.node
                .children
                .get("url")
                .and_then(|u| u.schema.fixed.as_ref().or(u.schema.pattern.as_ref()))
                .and_then(Value::as_str)
                == Some(url)
    })
}

// ---------------------------------------------------------------------------
// Match building
// ---------------------------------------------------------------------------

/// Build a match from discriminators, reading constants / type / binding /
/// profile / cardinality metadata out of the slice subtree.
fn build_match(
    node: &super::tree::Node,
    discriminators: &[EdDiscriminator],
    extension_profile: Option<&str>,
) -> Option<Match> {
    if discriminators.is_empty() {
        return None;
    }

    let mut parsed: Vec<(&EdDiscriminator, Vec<DSeg>)> = Vec::with_capacity(discriminators.len());
    for disc in discriminators {
        let segs = parse_disc_path(&disc.path)?;
        if segs.contains(&DSeg::Resolve) {
            return None; // needs the instance graph
        }
        parsed.push((disc, segs));
    }

    let kinds: Vec<&str> = discriminators.iter().map(|d| d.type_.as_str()).collect();
    if kinds.iter().all(|k| matches!(*k, "value" | "pattern")) {
        return build_pattern_match(node, &parsed);
    }
    if kinds.iter().all(|k| *k == "exists") {
        return build_exists_match(node, &parsed);
    }

    // Single non-pattern discriminator (homogeneous set of one kind).
    if kinds.iter().all(|k| *k == "type") && parsed.len() == 1 {
        let (target, _) = resolve_path(node, &parsed[0].1)?;
        let type_code = target.schema.type_.as_ref()?;
        return Some(Match {
            type_: Some("type".to_string()),
            value: Some(Value::String(type_code.clone())),
            resolve_ref: None,
        });
    }
    if kinds.iter().all(|k| *k == "profile") && parsed.len() == 1 {
        let (target, _) = resolve_path(node, &parsed[0].1)?;
        let profile = extension_profile
            .map(str::to_string)
            .or_else(|| target.type_profiles.first().cloned())
            .or_else(|| target.schema.url.clone())
            .or_else(|| {
                target
                    .schema
                    .refers
                    .as_ref()
                    .and_then(|r| r.first().cloned())
            })?;
        return Some(Match {
            type_: Some("profile".to_string()),
            value: Some(Value::String(profile)),
            resolve_ref: None,
        });
    }
    if kinds.iter().all(|k| *k == "binding") && parsed.len() == 1 {
        let (target, _) = resolve_path(node, &parsed[0].1)?;
        let vs = target.schema.binding.as_ref()?.value_set.clone();
        return Some(Match {
            type_: Some("binding".to_string()),
            value: Some(Value::String(vs)),
            resolve_ref: None,
        });
    }

    None
}

fn build_pattern_match(
    node: &super::tree::Node,
    discs: &[(&EdDiscriminator, Vec<DSeg>)],
) -> Option<Match> {
    // A path traversing extension('url') is not expressible as a plain
    // partial-match pattern (patterns match arrays prefix-wise, extensions
    // need containment); it compiles to a dedicated `extension` matcher,
    // only supported as the sole discriminator.
    if discs
        .iter()
        .any(|(_, segs)| segs.iter().any(|s| matches!(s, DSeg::Extension(_))))
    {
        if discs.len() != 1 {
            return None;
        }
        return build_extension_match(node, &discs[0].1);
    }

    let mut this_constant: Option<Value> = None;
    let mut pattern = Map::new();

    for (_, segs) in discs {
        let (target, rsegs) = resolve_path(node, segs)?;
        let constant = constant_of(target)?;
        if segs.is_empty() {
            if discs.len() > 1 {
                return None; // $this plus siblings — ambiguous
            }
            this_constant = Some(constant);
        } else {
            let keys: Vec<&str> = rsegs
                .iter()
                .map(|r| match r {
                    RSeg::Key(k) => k.as_str(),
                    RSeg::Extension(_) => unreachable!("extension paths handled above"),
                })
                .collect();
            insert_nested(&mut pattern, &keys, constant);
        }
    }

    let value = match this_constant {
        Some(v) => v,
        None if !pattern.is_empty() => Value::Object(pattern),
        None => return None,
    };
    Some(Match {
        type_: Some("pattern".to_string()),
        value: Some(value),
        resolve_ref: None,
    })
}

/// `extension('u1')[.extension('u2')…][.key…]` — compiled to a nested
/// `extension` matcher: `{url, extension: {…}}` for each chained url, with
/// the trailing keys becoming a partial-match `pattern` on the innermost
/// extension element.
fn build_extension_match(node: &super::tree::Node, segs: &[DSeg]) -> Option<Match> {
    let mut urls: Vec<String> = Vec::new();
    let mut idx = 0;
    while let Some(DSeg::Extension(url)) = segs.get(idx) {
        urls.push(url.clone());
        idx += 1;
    }
    // Keys before the first extension(), or extension() again after keys,
    // are not expressible in this matcher shape.
    if urls.is_empty()
        || segs[idx..]
            .iter()
            .any(|s| !matches!(s, DSeg::Key(_) | DSeg::OfType(_)))
    {
        return None;
    }

    let (target, rsegs) = resolve_path(node, segs)?;
    let constant = constant_of(target)?;

    let tail_keys: Vec<&str> = rsegs[urls.len()..]
        .iter()
        .map(|r| match r {
            RSeg::Key(k) => k.as_str(),
            RSeg::Extension(_) => unreachable!("chain shape checked above"),
        })
        .collect();
    let pattern = if tail_keys.is_empty() {
        constant
    } else {
        let mut map = Map::new();
        insert_nested(&mut map, &tail_keys, constant);
        Value::Object(map)
    };

    let mut matcher = json!({ "url": urls.pop()?, "pattern": pattern });
    while let Some(url) = urls.pop() {
        matcher = json!({ "url": url, "extension": matcher });
    }
    Some(Match {
        type_: Some("extension".to_string()),
        value: Some(matcher),
        resolve_ref: None,
    })
}

/// `exists` discriminators: one `{path, exists}` entry per discriminator.
/// Expected presence is read from the slice differential; a path whose
/// presence the differential doesn't pin is untranslatable.
fn build_exists_match(
    node: &super::tree::Node,
    discs: &[(&EdDiscriminator, Vec<DSeg>)],
) -> Option<Match> {
    let mut entries: Vec<Value> = Vec::new();
    for (_, segs) in discs {
        let (last, init) = segs.split_last()?;
        let DSeg::Key(name) = last else {
            return None;
        };
        let (parent, rsegs) = resolve_path(node, init)?;
        let expected = expected_existence(parent, name)?;
        let mut path: Vec<Value> = rsegs
            .iter()
            .map(|r| match r {
                RSeg::Key(k) => Value::String(k.clone()),
                RSeg::Extension(u) => json!({ "extension": u }),
            })
            .collect();
        path.push(Value::String(name.clone()));
        entries.push(json!({ "path": path, "exists": expected }));
    }
    Some(Match {
        type_: Some("exists".to_string()),
        value: Some(Value::Array(entries)),
        resolve_ref: None,
    })
}

/// Did the slice differential pin `name` as present (`min >= 1`) or absent
/// (`max = 0`) under `parent`?
fn expected_existence(parent: &super::tree::Node, name: &str) -> Option<bool> {
    let listed =
        |list: &Option<Vec<String>>| list.as_ref().is_some_and(|l| l.iter().any(|v| v == name));
    if listed(&parent.schema.required) {
        return Some(true);
    }
    if listed(&parent.schema.excluded) {
        return Some(false);
    }
    let child = parent.children.get(name)?;
    if child.dead {
        return Some(false);
    }
    if child.schema.min.is_some_and(|m| m >= 1) {
        return Some(true);
    }
    if child.schema.max == Some(0) {
        return Some(false);
    }
    None
}

/// The fixed/pattern constant carried by a resolved node.
fn constant_of(node: &super::tree::Node) -> Option<Value> {
    node.schema
        .fixed
        .clone()
        .or_else(|| node.schema.pattern.clone())
}

/// `insert_nested(map, ["a", "b"], v)` → `{a: {b: v}}` (merging siblings).
fn insert_nested(map: &mut Map<String, Value>, keys: &[&str], value: Value) {
    let (last, init) = keys.split_last().expect("non-empty key path");
    let mut current = map;
    for key in init {
        current = current
            .entry(key.to_string())
            .or_insert_with(|| Value::Object(Map::new()))
            .as_object_mut()
            .expect("nested pattern segment is an object");
    }
    current.insert(last.to_string(), value);
}
