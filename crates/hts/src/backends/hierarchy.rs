//! Nesting a flat `$expand` result into a tree, shared by every backend.
//!
//! Backends differ only in how they load `concept_hierarchy` edges; once the
//! edges restricted to the expansion are in hand, building the tree is pure and
//! lives here so the cycle handling cannot drift between SQLite and PostgreSQL.

use std::collections::{HashMap, HashSet};

use crate::types::ExpansionContains;

/// `(system_url, code)`.
pub(crate) type ConceptKey = (String, String);

/// Nest `flat` into a tree using `parent_to_children`.
///
/// `parent_to_children` must only contain edges whose parent **and** child are
/// both in `flat`. Concepts with no parent inside the expansion become roots.
///
/// `concept_hierarchy` is not guaranteed acyclic: SNOMED CT RF2 imports leave
/// mutual parent/child pairs behind (e.g. `310387003` ⇄ `707221002`). Two
/// things follow:
///
/// - the walk has to stop at a back-edge, or it recurses until the stack
///   overflows and aborts the process — see [`build_subtree`];
/// - a cycle with no parent outside it has no root, so its members would never
///   be reached and would silently drop out of the expansion. Each such cycle
///   is promoted to a root at its lowest code instead.
///
/// Roots and every level of children are sorted by code so the tree order is
/// deterministic regardless of edge storage order.
pub(crate) fn nest_expansion(
    flat: Vec<ExpansionContains>,
    parent_to_children: &HashMap<ConceptKey, Vec<ConceptKey>>,
) -> Vec<ExpansionContains> {
    let has_parent: HashSet<&ConceptKey> = parent_to_children.values().flatten().collect();

    let mut ordered: Vec<ConceptKey> = flat
        .iter()
        .map(|c| (c.system.clone(), c.code.clone()))
        .collect();
    ordered.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));

    let items_map: HashMap<ConceptKey, ExpansionContains> = flat
        .into_iter()
        .map(|c| ((c.system.clone(), c.code.clone()), c))
        .collect();

    let mut placed: HashSet<ConceptKey> = HashSet::new();
    let mut roots: Vec<ExpansionContains> = Vec::new();
    let mut path = HashSet::new();

    for key in ordered.iter().filter(|k| !has_parent.contains(k)) {
        let tree = build_subtree(key, &items_map, parent_to_children, &mut path);
        collect_keys(&tree, &mut placed);
        roots.push(tree);
    }
    // Anything still unplaced sits in a cycle with no parent outside it.
    // `ordered` is sorted, so each such cycle is rooted at its lowest code.
    for key in &ordered {
        if placed.contains(key) {
            continue;
        }
        let tree = build_subtree(key, &items_map, parent_to_children, &mut path);
        collect_keys(&tree, &mut placed);
        roots.push(tree);
    }

    roots.sort_by(|a, b| a.code.cmp(&b.code));
    roots
}

/// Build the node for `key` with all its nested children.
///
/// `path` holds the keys on the current recursion stack; a child already on it
/// closes a cycle and is skipped. The guard is path-local rather than a global
/// visited set because these are poly-hierarchies: one concept legitimately
/// nests under each of its parents.
fn build_subtree(
    key: &ConceptKey,
    items_map: &HashMap<ConceptKey, ExpansionContains>,
    parent_to_children: &HashMap<ConceptKey, Vec<ConceptKey>>,
    path: &mut HashSet<ConceptKey>,
) -> ExpansionContains {
    let mut item = items_map[key].clone();
    if let Some(children) = parent_to_children.get(key) {
        path.insert(key.clone());
        let mut child_items: Vec<ExpansionContains> = Vec::with_capacity(children.len());
        for child_key in children {
            if !path.contains(child_key) {
                child_items.push(build_subtree(
                    child_key,
                    items_map,
                    parent_to_children,
                    path,
                ));
            }
        }
        path.remove(key);
        child_items.sort_by(|a, b| a.code.cmp(&b.code));
        item.contains = child_items;
    }
    item
}

fn collect_keys(node: &ExpansionContains, out: &mut HashSet<ConceptKey>) {
    out.insert((node.system.clone(), node.code.clone()));
    for child in &node.contains {
        collect_keys(child, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SYS: &str = "http://snomed.info/sct";

    fn key(code: &str) -> ConceptKey {
        (SYS.to_string(), code.to_string())
    }

    fn node(code: &str) -> ExpansionContains {
        ExpansionContains {
            system: SYS.to_string(),
            version: None,
            code: code.to_string(),
            display: None,
            is_abstract: None,
            inactive: None,
            designations: vec![],
            properties: vec![],
            extensions: vec![],
            contains: vec![],
        }
    }

    fn edges(pairs: &[(&str, &str)]) -> HashMap<ConceptKey, Vec<ConceptKey>> {
        let mut map: HashMap<ConceptKey, Vec<ConceptKey>> = HashMap::new();
        for (parent, child) in pairs {
            map.entry(key(parent)).or_default().push(key(child));
        }
        map
    }

    fn flat(codes: &[&str]) -> Vec<ExpansionContains> {
        codes.iter().map(|c| node(c)).collect()
    }

    /// Render a tree as `code(child child)` for compact assertions.
    fn render(nodes: &[ExpansionContains]) -> String {
        nodes
            .iter()
            .map(|n| {
                if n.contains.is_empty() {
                    n.code.clone()
                } else {
                    format!("{}({})", n.code, render(&n.contains))
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn nests_an_acyclic_hierarchy() {
        let tree = nest_expansion(
            flat(&["c", "root", "b", "a"]),
            &edges(&[("root", "b"), ("root", "a"), ("a", "c")]),
        );
        assert_eq!(render(&tree), "root(a(c) b)");
    }

    /// The overflow that took the server down: `root → a`, plus `a ⇄ b`.
    #[test]
    fn terminates_on_a_cycle_below_a_root() {
        let tree = nest_expansion(
            flat(&["root", "a", "b"]),
            &edges(&[("root", "a"), ("a", "b"), ("b", "a")]),
        );
        assert_eq!(render(&tree), "root(a(b))");
    }

    #[test]
    fn keeps_a_shared_concept_under_every_parent() {
        let tree = nest_expansion(
            flat(&["root", "p1", "p2", "shared"]),
            &edges(&[
                ("root", "p1"),
                ("root", "p2"),
                ("p1", "shared"),
                ("p2", "shared"),
            ]),
        );
        assert_eq!(render(&tree), "root(p1(shared) p2(shared))");
    }

    /// A mutual pair reachable from two sides renders twice, each nested under
    /// the other. That is the cost of a path-local guard, and the alternative
    /// (a global visited set) would break the poly-hierarchy case above.
    #[test]
    fn mutual_pair_under_a_shared_parent_nests_both_ways() {
        let tree = nest_expansion(
            flat(&["root", "a", "b"]),
            &edges(&[("root", "a"), ("root", "b"), ("a", "b"), ("b", "a")]),
        );
        assert_eq!(render(&tree), "root(a(b) b(a))");
    }

    /// Every member has a parent inside the expansion, so there is no root.
    #[test]
    fn promotes_a_cycle_that_is_the_whole_expansion() {
        let tree = nest_expansion(flat(&["b", "a"]), &edges(&[("a", "b"), ("b", "a")]));
        assert_eq!(render(&tree), "a(b)");
    }

    /// A parent-only cycle beside an ordinary root must not drop out.
    #[test]
    fn promotes_an_unreachable_cycle_beside_a_root() {
        let tree = nest_expansion(
            flat(&["root", "leaf", "x", "y"]),
            &edges(&[("root", "leaf"), ("x", "y"), ("y", "x")]),
        );
        assert_eq!(render(&tree), "root(leaf) x(y)");
    }

    #[test]
    fn every_concept_appears_at_least_once() {
        let codes = ["root", "a", "b", "c", "x", "y", "z"];
        let tree = nest_expansion(
            flat(&codes),
            &edges(&[
                ("root", "a"),
                ("a", "b"),
                ("b", "c"),
                ("c", "a"),
                ("x", "y"),
                ("y", "z"),
                ("z", "x"),
            ]),
        );
        let mut seen = HashSet::new();
        for root in &tree {
            collect_keys(root, &mut seen);
        }
        for code in codes {
            assert!(seen.contains(&key(code)), "{code} missing from {tree:?}");
        }
    }
}
