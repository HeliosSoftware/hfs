//! A foldable, line-numbered, syntax-highlighted JSON view (#264).
//!
//! Brett's Resource Editor shows the JSON and the guided form side by side, and
//! the JSON is a real code view — line numbers down the gutter and a fold arrow
//! on every object and array so a big resource collapses to its shape. A
//! `<textarea>` cannot do that, so the view is rendered here, server-side, into
//! [`JsonLine`]s the template lays out and a few lines of JS fold.
//!
//! The document is walked directly rather than pretty-printed and re-parsed:
//! that way each container knows its own line, its closing line, and its
//! ancestry, which is exactly what folding needs.

use serde_json::Value;

/// One highlighted run within a line.
pub struct Token {
    pub text: String,
    /// `key` | `string` | `number` | `bool` | `null` | `punct`
    pub kind: &'static str,
}

/// One rendered line of JSON.
pub struct JsonLine {
    pub num: usize,
    /// Indent depth, for the left pad.
    pub depth: usize,
    /// This line opens a container (object/array) and can be folded.
    pub foldable: bool,
    /// This container's id, when `foldable` — its descendants and its closing
    /// line carry it in `parents`, so folding hides them.
    pub fold_id: String,
    /// Space-joined ids of every container this line sits inside. A line is
    /// hidden when any of these is collapsed.
    pub parents: String,
    /// The shape shown on the opening line when it is folded (`{ … }`,
    /// `[ 3 ]`).
    pub summary: String,
    pub tokens: Vec<Token>,
}

/// Renders a whole document into foldable lines.
pub fn lines(value: &Value) -> Vec<JsonLine> {
    let mut ctx = Ctx {
        lines: Vec::new(),
        counter: 0,
    };
    let mut root = Line::new(0, &[]);
    ctx.walk(value, &mut root, &[], true);
    ctx.lines
}

struct Ctx {
    lines: Vec<JsonLine>,
    counter: usize,
}

/// A line under construction — tokens accumulate onto it until it is pushed.
struct Line {
    depth: usize,
    parents: Vec<String>,
    tokens: Vec<Token>,
    foldable: bool,
    fold_id: String,
    summary: String,
}

impl Line {
    fn new(depth: usize, parents: &[String]) -> Self {
        Line {
            depth,
            parents: parents.to_vec(),
            tokens: Vec::new(),
            foldable: false,
            fold_id: String::new(),
            summary: String::new(),
        }
    }
    fn punct(&mut self, text: &str) {
        self.tokens.push(Token {
            text: text.to_string(),
            kind: "punct",
        });
    }
}

impl Ctx {
    fn push(&mut self, line: Line) {
        let num = self.lines.len() + 1;
        self.lines.push(JsonLine {
            num,
            depth: line.depth,
            foldable: line.foldable,
            fold_id: line.fold_id,
            parents: line.parents.join(" "),
            summary: line.summary,
            tokens: line.tokens,
        });
    }

    fn next_id(&mut self) -> String {
        self.counter += 1;
        format!("f{}", self.counter)
    }

    /// Emits `value` onto `line`, which already carries any leading `"key": `.
    /// `trailing` is the comma (or empty) that follows this value in its parent.
    fn walk(&mut self, value: &Value, line: &mut Line, trailing: &[&str], _root: bool) {
        match value {
            Value::Object(map) if !map.is_empty() => {
                let id = self.next_id();
                line.foldable = true;
                line.fold_id = id.clone();
                line.summary = format!("{{ … }}{}", trailing.join(""));
                line.punct("{");
                let opener = std::mem::replace(line, Line::new(0, &[]));
                self.push(opener);

                let mut child_parents = self.parents_of_last();
                child_parents.push(id.clone());
                let len = map.len();
                for (i, (key, child_value)) in map.iter().enumerate() {
                    let mut child = Line::new(depth_after(&child_parents), &child_parents);
                    child.tokens.push(Token {
                        text: format!("\"{}\"", escape(key)),
                        kind: "key",
                    });
                    child.punct(": ");
                    let comma = if i + 1 < len { "," } else { "" };
                    self.walk(child_value, &mut child, &[comma], false);
                }

                let mut closer =
                    Line::new(depth_after(&child_parents) - 1, &prefix(&child_parents));
                closer.parents.push(id);
                closer.punct(&format!("}}{}", trailing.join("")));
                self.push(closer);
            }
            Value::Array(items) if !items.is_empty() => {
                let id = self.next_id();
                line.foldable = true;
                line.fold_id = id.clone();
                line.summary = format!("[ {} ]{}", items.len(), trailing.join(""));
                line.punct("[");
                let opener = std::mem::replace(line, Line::new(0, &[]));
                self.push(opener);

                let mut child_parents = self.parents_of_last();
                child_parents.push(id.clone());
                let len = items.len();
                for (i, child_value) in items.iter().enumerate() {
                    let mut child = Line::new(depth_after(&child_parents), &child_parents);
                    let comma = if i + 1 < len { "," } else { "" };
                    self.walk(child_value, &mut child, &[comma], false);
                }

                let mut closer =
                    Line::new(depth_after(&child_parents) - 1, &prefix(&child_parents));
                closer.parents.push(id);
                closer.punct(&format!("]{}", trailing.join("")));
                self.push(closer);
            }
            // Scalars and empty containers are one line.
            other => {
                line.tokens.push(scalar_token(other));
                for t in trailing {
                    line.punct(t);
                }
                let done = std::mem::replace(line, Line::new(0, &[]));
                self.push(done);
            }
        }
    }

    /// The parent ids of the line just pushed (the opener), which its children
    /// build on.
    fn parents_of_last(&self) -> Vec<String> {
        self.lines
            .last()
            .map(|line| {
                if line.parents.is_empty() {
                    Vec::new()
                } else {
                    line.parents.split(' ').map(String::from).collect()
                }
            })
            .unwrap_or_default()
    }
}

/// The indent depth for a line whose ancestry is `parents`.
fn depth_after(parents: &[String]) -> usize {
    parents.len()
}

/// The parents of a closing line: everything but the container it closes.
fn prefix(parents: &[String]) -> Vec<String> {
    if parents.is_empty() {
        Vec::new()
    } else {
        parents[..parents.len() - 1].to_vec()
    }
}

fn scalar_token(value: &Value) -> Token {
    match value {
        Value::String(s) => Token {
            text: format!("\"{}\"", escape(s)),
            kind: "string",
        },
        Value::Number(n) => Token {
            text: n.to_string(),
            kind: "number",
        },
        Value::Bool(b) => Token {
            text: b.to_string(),
            kind: "bool",
        },
        Value::Null => Token {
            text: "null".to_string(),
            kind: "null",
        },
        Value::Object(_) => Token {
            text: "{}".to_string(),
            kind: "punct",
        },
        Value::Array(_) => Token {
            text: "[]".to_string(),
            kind: "punct",
        },
    }
}

/// Minimal JSON string escaping for display (the template auto-escapes HTML on
/// top of this).
fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn render(value: Value) -> Vec<JsonLine> {
        lines(&value)
    }

    #[test]
    fn a_flat_object_is_one_line_per_field_plus_braces() {
        let out = render(json!({ "resourceType": "Patient", "id": "a12" }));
        // { , resourceType, id, }
        assert_eq!(out.len(), 4);
        assert!(out[0].foldable, "the opening brace folds the object");
        assert_eq!(out[0].num, 1);
        // Keys are highlighted as keys.
        assert!(out[1].tokens.iter().any(|t| t.kind == "key"));
        assert!(out[1].tokens.iter().any(|t| t.kind == "string"));
    }

    #[test]
    fn line_numbers_are_sequential() {
        let out = render(json!({ "a": 1, "b": 2 }));
        for (i, line) in out.iter().enumerate() {
            assert_eq!(line.num, i + 1);
        }
    }

    #[test]
    fn a_nested_container_carries_its_parents_so_it_can_be_hidden() {
        let out = render(json!({ "name": [{ "family": "Duck" }] }));
        // The deepest line (family) sits inside both the object and the array.
        let family = out
            .iter()
            .find(|l| l.tokens.iter().any(|t| t.text.contains("family")))
            .expect("family line");
        // Two ancestors: the root object's array value, and the array's item.
        assert!(
            family.parents.split(' ').count() >= 2,
            "nested line names every container it is inside: {}",
            family.parents
        );
    }

    #[test]
    fn a_foldable_line_has_a_summary_for_the_collapsed_state() {
        let out = render(json!({ "name": [1, 2, 3] }));
        let array_open = out
            .iter()
            .find(|l| l.foldable && l.summary.starts_with('['))
            .expect("the array's opening line");
        assert_eq!(array_open.summary, "[ 3 ]");
    }

    #[test]
    fn the_closing_line_is_hidden_with_its_container() {
        let out = render(json!({ "a": { "b": 1 } }));
        // The inner object's opener and closer share the fold id.
        let inner_open = out.iter().find(|l| l.foldable && l.depth == 1).unwrap();
        let id = inner_open.fold_id.clone();
        let closers: Vec<_> = out
            .iter()
            .filter(|l| {
                l.parents.split(' ').any(|p| p == id)
                    && l.tokens.iter().any(|t| t.text.starts_with('}'))
            })
            .collect();
        assert_eq!(closers.len(), 1, "the closing brace folds with its object");
    }

    #[test]
    fn depth_increases_with_nesting() {
        let out = render(json!({ "a": { "b": { "c": 1 } } }));
        let deepest = out.iter().map(|l| l.depth).max().unwrap();
        assert!(deepest >= 3, "three levels of nesting reach depth 3");
    }
}
