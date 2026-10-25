//! Nesting a flat `$expand` result into a tree, shared by every backend.
//!
//! Backends differ only in how they load `concept_hierarchy` edges; once the
//! edges restricted to the expansion are in hand, building the tree is pure and
//! lives here so the cycle handling cannot drift between SQLite and PostgreSQL.

use std::collections::{HashMap, HashSet};

use crate::types::ExpansionContains;

/// `(system_url, code, resolved_version)`.
/// The same code in different CodeSystem versions is a distinct tree node.
pub(crate) type ConceptKey = (String, String, Option<String>);

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
        .map(|c| (c.system.clone(), c.code.clone(), c.version.clone()))
        .collect();
    ordered.sort_by(|a, b| {
        a.1.cmp(&b.1)
            .then_with(|| a.0.cmp(&b.0))
            .then_with(|| a.2.cmp(&b.2))
    });

    let items_map: HashMap<ConceptKey, ExpansionContains> = flat
        .into_iter()
        .map(|c| ((c.system.clone(), c.code.clone(), c.version.clone()), c))
        .collect();

    let mut placed: HashSet<ConceptKey> = HashSet::new();
    let mut roots: Vec<ExpansionContains> = Vec::new();
    let mut path = HashSet::new();

    for key in ordered.iter().filter(|k| !has_parent.contains(k)) {
        let tree = build_subtree(key, &items_map, parent_to_children, &mut path);
        collect_keys(&tree, &mut placed);
        roots.push(tree);
    }
    // Anything still unplaced is a rootless cycle or hangs below one. Root
    // only the cycles nothing outside them points into, each at its lowest
    // code; everything else unplaced is reachable from one of them. Rooting
    // at the lowest unplaced code instead would promote a descendant that
    // sorts before its cycle, and it would then also appear nested.
    let unplaced: Vec<&ConceptKey> = ordered.iter().filter(|k| !placed.contains(*k)).collect();
    for key in source_cycle_roots(&unplaced, parent_to_children) {
        roots.push(build_subtree(
            key,
            &items_map,
            parent_to_children,
            &mut path,
        ));
    }

    roots.sort_by(|a, b| {
        a.code
            .cmp(&b.code)
            .then_with(|| a.system.cmp(&b.system))
            .then_with(|| a.version.cmp(&b.version))
    });
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
        child_items.sort_by(|a, b| {
            a.code
                .cmp(&b.code)
                .then_with(|| a.system.cmp(&b.system))
                .then_with(|| a.version.cmp(&b.version))
        });
        item.contains = child_items;
    }
    item
}

/// Pick one root per source strongly connected component of `unplaced`.
///
/// `unplaced` must be sorted by code. Every parent of an unplaced concept is
/// itself unplaced (a placed parent would have placed it), so the components
/// with no parent outside themselves are exactly the cycles that nothing else
/// reaches. Each is rooted at its first member in `unplaced` order, i.e. its
/// lowest code.
///
/// Tarjan's algorithm, iterative so a long chain cannot overflow the stack.
fn source_cycle_roots<'a>(
    unplaced: &[&'a ConceptKey],
    parent_to_children: &HashMap<ConceptKey, Vec<ConceptKey>>,
) -> Vec<&'a ConceptKey> {
    let pos: HashMap<&ConceptKey, usize> =
        unplaced.iter().enumerate().map(|(i, k)| (*k, i)).collect();
    let children: Vec<Vec<usize>> = unplaced
        .iter()
        .map(|k| {
            parent_to_children
                .get(*k)
                .into_iter()
                .flatten()
                .filter_map(|c| pos.get(c).copied())
                .collect()
        })
        .collect();

    let n = unplaced.len();
    let mut index = vec![usize::MAX; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut comp = vec![usize::MAX; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut next_index = 0usize;
    let mut comp_count = 0usize;

    for start in 0..n {
        if index[start] != usize::MAX {
            continue;
        }
        // (node, next child to visit)
        let mut work: Vec<(usize, usize)> = vec![(start, 0)];
        index[start] = next_index;
        low[start] = next_index;
        next_index += 1;
        stack.push(start);
        on_stack[start] = true;

        while let Some(&mut (v, ref mut ci)) = work.last_mut() {
            if let Some(&w) = children[v].get(*ci) {
                *ci += 1;
                if index[w] == usize::MAX {
                    index[w] = next_index;
                    low[w] = next_index;
                    next_index += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    work.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
                continue;
            }
            work.pop();
            if let Some(&(parent, _)) = work.last() {
                low[parent] = low[parent].min(low[v]);
            }
            if low[v] == index[v] {
                while let Some(w) = stack.pop() {
                    on_stack[w] = false;
                    comp[w] = comp_count;
                    if w == v {
                        break;
                    }
                }
                comp_count += 1;
            }
        }
    }

    // A component is a source when no edge enters it from another component.
    let mut has_incoming = vec![false; comp_count];
    for (v, kids) in children.iter().enumerate() {
        for &w in kids {
            if comp[v] != comp[w] {
                has_incoming[comp[w]] = true;
            }
        }
    }
    let mut rooted = vec![false; comp_count];
    let mut roots = Vec::new();
    for (i, key) in unplaced.iter().enumerate() {
        let c = comp[i];
        if !has_incoming[c] && !rooted[c] {
            rooted[c] = true;
            roots.push(*key);
        }
    }
    roots
}

fn collect_keys(node: &ExpansionContains, out: &mut HashSet<ConceptKey>) {
    out.insert((node.system.clone(), node.code.clone(), node.version.clone()));
    for child in &node.contains {
        collect_keys(child, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SYS: &str = "http://snomed.info/sct";

    fn key(code: &str) -> ConceptKey {
        (SYS.to_string(), code.to_string(), None)
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

    #[test]
    fn keeps_same_codes_in_different_versions_and_systems_distinct() {
        let groups = [
            (SYS, None),
            (SYS, Some("1")),
            (SYS, Some("2")),
            ("http://example.org/other", Some("1")),
        ];
        let mut nodes = Vec::new();
        let mut edges = HashMap::new();
        for (system, version) in groups.into_iter().rev() {
            let version = version.map(str::to_owned);
            for code in ["child", "parent"] {
                let mut item = node(code);
                item.system = system.to_owned();
                item.version = version.clone();
                item.display = Some(format!("{system}/{version:?}/{code}"));
                nodes.push(item);
            }
            edges.insert(
                (system.to_owned(), "parent".to_owned(), version.clone()),
                vec![(system.to_owned(), "child".to_owned(), version)],
            );
        }
        let tree = nest_expansion(nodes, &edges);
        assert_eq!(tree.len(), groups.len());
        // Equal codes retain a deterministic system/version order, regardless
        // of the order the backend returned the concepts and edges.
        let mut expected = groups.to_vec();
        expected.sort();
        for (parent, (system, version)) in tree.iter().zip(expected) {
            assert_eq!(parent.code, "parent");
            assert_eq!(parent.system, system);
            assert_eq!(parent.version.as_deref(), version);
            assert_eq!(parent.contains.len(), 1);
            let child = &parent.contains[0];
            assert_eq!(child.code, "child");
            assert_eq!(child.system, system);
            assert_eq!(child.version.as_deref(), version);
            assert_eq!(child.display, Some(format!("{system}/{version:?}/child")));
            assert!(child.contains.is_empty());
        }
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

    /// A child of a rootless cycle that sorts before the cycle's members must
    /// stay nested, not also become a root of its own.
    #[test]
    fn roots_a_rootless_cycle_not_its_lower_sorting_child() {
        let tree = nest_expansion(
            flat(&["a", "m", "n"]),
            &edges(&[("m", "n"), ("n", "m"), ("n", "a")]),
        );
        assert_eq!(render(&tree), "m(n(a))");
    }

    /// Only the upstream cycle is a source; the cycle below it is reached
    /// through it and must not be promoted as well.
    #[test]
    fn roots_only_the_upstream_cycle_of_a_chain() {
        let tree = nest_expansion(
            flat(&["a", "b", "x", "y"]),
            &edges(&[("x", "y"), ("y", "x"), ("y", "a"), ("a", "b"), ("b", "a")]),
        );
        assert_eq!(render(&tree), "x(y(a(b)))");
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
