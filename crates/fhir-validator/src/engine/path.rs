//! Path tracking during the validation walk.
//!
//! The conformance contract renders paths dot-joined, rooted at the
//! resource type, with array indices as bare numbers:
//! `Bundle.entry.0.resource.extra`. The REST layer additionally needs
//! bracket-indexed FHIRPath (`Bundle.entry[0].resource.extra`) for
//! `OperationOutcome.issue.expression`.

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PathSeg {
    Key(String),
    Index(usize),
}

#[derive(Debug, Clone, Default)]
pub struct PathTracker {
    segs: Vec<PathSeg>,
}

impl PathTracker {
    /// Root the path at the resource type (empty string when the data has no
    /// `resourceType`, mirroring the reference validator).
    pub(crate) fn new(root: impl Into<String>) -> Self {
        Self {
            segs: vec![PathSeg::Key(root.into())],
        }
    }

    pub(crate) fn push_key(&mut self, key: &str) {
        self.segs.push(PathSeg::Key(key.to_string()));
    }

    pub(crate) fn push_index(&mut self, index: usize) {
        self.segs.push(PathSeg::Index(index));
    }

    pub(crate) fn pop(&mut self) {
        self.segs.pop();
    }

    /// The index-free element path of the *parent* of the current key —
    /// `Patient.name` while validating `Patient.name.0.extension` — the form
    /// extension contexts are declared in (#615).
    pub(crate) fn parent_element_path(&self) -> String {
        let keys: Vec<&str> = self
            .segs
            .iter()
            .filter_map(|s| match s {
                PathSeg::Key(k) => Some(k.as_str()),
                PathSeg::Index(_) => None,
            })
            .collect();
        keys[..keys.len().saturating_sub(1)].join(".")
    }

    /// The last `Key` segment — the element name currently being validated.
    /// Used by the `choices` keyword, which reports on the branch key.
    pub(crate) fn last_key(&self) -> Option<&str> {
        self.segs.iter().rev().find_map(|s| match s {
            PathSeg::Key(k) => Some(k.as_str()),
            PathSeg::Index(_) => None,
        })
    }

    /// Dot-joined conformance rendering: `Patient.name.0.given`.
    pub(crate) fn render_dotted(&self) -> String {
        let mut out = String::new();
        for (i, seg) in self.segs.iter().enumerate() {
            if i > 0 {
                out.push('.');
            }
            match seg {
                PathSeg::Key(k) => out.push_str(k),
                PathSeg::Index(n) => out.push_str(&n.to_string()),
            }
        }
        out
    }
}

/// Render a dotted conformance path as bracket-indexed FHIRPath:
/// `Patient.name.0.given` → `Patient.name[0].given`.
///
/// Used by the REST layer for `OperationOutcome.issue.expression`.
pub fn dotted_to_fhirpath(dotted: &str) -> String {
    let mut out = String::new();
    for seg in dotted.split('.') {
        if !out.is_empty() && seg.bytes().all(|b| b.is_ascii_digit()) && !seg.is_empty() {
            out.push('[');
            out.push_str(seg);
            out.push(']');
        } else {
            if !out.is_empty() {
                out.push('.');
            }
            out.push_str(seg);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_dotted_with_numeric_indices() {
        let mut p = PathTracker::new("Bundle");
        p.push_key("entry");
        p.push_index(0);
        p.push_key("resource");
        p.push_key("extra");
        assert_eq!(p.render_dotted(), "Bundle.entry.0.resource.extra");
        p.pop();
        assert_eq!(p.render_dotted(), "Bundle.entry.0.resource");
    }

    #[test]
    fn empty_root_renders_like_reference() {
        // No resourceType → root joins to the empty string.
        let mut p = PathTracker::new("");
        assert_eq!(p.render_dotted(), "");
        p.push_key("x");
        assert_eq!(p.render_dotted(), ".x");
    }

    #[test]
    fn last_key_skips_indices() {
        let mut p = PathTracker::new("Patient");
        p.push_key("name");
        p.push_index(2);
        assert_eq!(p.last_key(), Some("name"));
    }

    #[test]
    fn fhirpath_rendering_brackets_indices() {
        assert_eq!(
            dotted_to_fhirpath("Bundle.entry.0.resource.extra"),
            "Bundle.entry[0].resource.extra"
        );
        assert_eq!(dotted_to_fhirpath("Patient"), "Patient");
        assert_eq!(dotted_to_fhirpath("Patient.name.10"), "Patient.name[10]");
    }
}
