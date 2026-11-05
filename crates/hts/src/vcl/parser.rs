//! VCL (Value Set Composition Language) lexer, AST, and recursive-descent
//! parser.
//!
//! Implements the grammar published at
//! <https://build.fhir.org/ig/FHIR/ig-guidance/vcl.html>. The parser is pure
//! (no storage access, no backend `cfg`) and reports every error with a
//! 1-based character column into the expression it was given.
//!
//! ## Grammar notes
//!
//! * `expr` allows exactly one kind of operator per level: `A,B`, `A;B` or
//!   `A-B`. Mixing them without parentheses (`A,B;C`) is a syntax error, and
//!   exclusion does not chain (`A-B-C`).
//! * `-` is a valid code character after the first character, so `A-B` is
//!   one code. Exclusion after a bare code needs whitespace or parentheses.
//! * URIs follow the published lexer rule, not the reference parsers: they may
//!   contain `: ; , = ^ { } .` and end only at `(`, `)`, `|`, whitespace or
//!   end of input. Parentheses inside a URI must be percent-encoded.
//! * Quoted values support only the `\"` and `\\` escapes.
//! * Newlines are rejected outside quoted values.
//!
//! ## Deliberate extensions
//!
//! * A hierarchical operator with no property (`<<404684003`) is accepted and
//!   gets the property `concept`. The published grammar requires
//!   `concept<<404684003`; the issue's own example omits it.
//! * `prop^{A}` (a one-element code list) is accepted, as in the reference
//!   parsers. Lists used as the source of `of` accept one element too.
//!
//! ## Out of scope here
//!
//! The parser accepts the whole grammar, including constructs that HTS cannot
//! evaluate (`!!<`, `?`, `of`, nested filter lists). Rejecting those is the
//! translator's job.

use std::fmt;

/// Longest accepted expression, in bytes.
pub const MAX_EXPRESSION_LEN: usize = 64 * 1024;

/// Deepest accepted nesting of `( … )` groups and `{ … }` lists.
pub const MAX_NESTING_DEPTH: usize = 32;

// ── Errors ────────────────────────────────────────────────────────────────────

/// A syntax error with its position in the (already percent-decoded)
/// expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VclError {
    /// 1-based character column of the offending character or token.
    pub pos: usize,
    /// Human-readable description, without the position.
    pub message: String,
}

impl VclError {
    fn new(pos: usize, message: impl Into<String>) -> Self {
        Self {
            pos,
            message: message.into(),
        }
    }
}

impl fmt::Display for VclError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at position {}", self.message, self.pos)
    }
}

impl std::error::Error for VclError {}

// ── AST ───────────────────────────────────────────────────────────────────────

/// A URI with an optional `|version` suffix (`http://snomed.info/sct|2021`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UriRef {
    /// 1-based column of the URI.
    pub pos: usize,
    pub uri: String,
    pub version: Option<String>,
}

/// The operator of a [`Filter`], mapped to the FHIR filter operator it
/// corresponds to in `ValueSet.compose.include.filter.op`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterOp {
    /// `=`
    Eq,
    /// `<<` — `is-a`
    IsA,
    /// `~<<` — `is-not-a`
    IsNotA,
    /// `<` — `descendent-of`
    DescendantOf,
    /// `/` — `regex`
    Regex,
    /// `^` — `in`
    In,
    /// `~^` — `not-in`
    NotIn,
    /// `>>` — `generalizes`
    Generalizes,
    /// `<!` — `child-of`
    ChildOf,
    /// `!!<` — `descendent-leaf`
    DescendantLeaf,
    /// `?` — `exists`
    Exists,
    /// `.` — "of": `{value}.property`
    Of,
}

/// The right-hand side of a [`Filter`] (the left-hand side for
/// [`FilterOp::Of`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterValue {
    /// A single code (unquoted, or quoted with escapes resolved).
    Code(String),
    /// The unescaped pattern of a `/"…"` regex filter.
    Regex(String),
    /// `{a,b,…}`
    CodeList(Vec<String>),
    /// A bare URI after `^` / `~^`, or as the source of `of`.
    Uri(UriRef),
    /// `{filter,…}`
    FilterList(Vec<Filter>),
    /// `*` as the source of `of`.
    All,
}

/// `property op value`, or `value.property` for [`FilterOp::Of`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filter {
    /// 1-based column where the filter starts.
    pub pos: usize,
    pub property: String,
    pub op: FilterOp,
    pub value: FilterValue,
}

/// An expression node with the column where it starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expr {
    /// 1-based column where the expression starts.
    pub pos: usize,
    pub kind: ExprKind,
}

/// The shape of an [`Expr`]. Parenthesised groups produce no node of their
/// own: `(A;B)` is just the inner disjunction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExprKind {
    /// `*` — every code (of the enclosing system).
    All,
    /// A single code.
    Code(String),
    /// A property filter.
    Filter(Filter),
    /// `^uri` / `^(uri)` — include another value set.
    IncludeValueSet(UriRef),
    /// `(system)expr` — `expr` is scoped to a code system.
    Scoped { system: UriRef, expr: Box<Expr> },
    /// `A,B,…` — intersection (two or more operands).
    Conjunction(Vec<Expr>),
    /// `A;B,…` — union (two or more operands).
    Disjunction(Vec<Expr>),
    /// `A-B` — `left` minus `right`.
    Exclusion { left: Box<Expr>, right: Box<Expr> },
}

// ── Lexer ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Kind {
    Dash,
    Open,
    Close,
    LCurly,
    RCurly,
    Semi,
    Comma,
    Dot,
    Star,
    Eq,
    IsA,
    IsNotA,
    DescOf,
    Regex,
    In,
    NotIn,
    Generalizes,
    ChildOf,
    DescLeaf,
    Exists,
    /// Raw URI text, including any `|version`.
    Uri(String),
    Code {
        value: String,
        quoted: bool,
    },
    Eof,
}

#[derive(Debug, Clone)]
struct Tok {
    kind: Kind,
    /// 1-based column.
    pos: usize,
}

fn describe(kind: &Kind) -> String {
    match kind {
        Kind::Dash => "'-'".into(),
        Kind::Open => "'('".into(),
        Kind::Close => "')'".into(),
        Kind::LCurly => "'{'".into(),
        Kind::RCurly => "'}'".into(),
        Kind::Semi => "';'".into(),
        Kind::Comma => "','".into(),
        Kind::Dot => "'.'".into(),
        Kind::Star => "'*'".into(),
        Kind::Eq => "'='".into(),
        Kind::IsA => "'<<'".into(),
        Kind::IsNotA => "'~<<'".into(),
        Kind::DescOf => "'<'".into(),
        Kind::Regex => "'/'".into(),
        Kind::In => "'^'".into(),
        Kind::NotIn => "'~^'".into(),
        Kind::Generalizes => "'>>'".into(),
        Kind::ChildOf => "'<!'".into(),
        Kind::DescLeaf => "'!!<'".into(),
        Kind::Exists => "'?'".into(),
        Kind::Uri(u) => format!("URI '{u}'"),
        Kind::Code { value, .. } => format!("code '{value}'"),
        Kind::Eof => "end of expression".into(),
    }
}

fn is_uri_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "?=:;&_%@#$^!{}/+,-.".contains(c)
}

fn lex(input: &str) -> Result<Vec<Tok>, VclError> {
    let chars: Vec<char> = input.chars().collect();
    let n = chars.len();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < n {
        let c = chars[i];
        let pos = i + 1;
        let simple = |kind: Kind, len: usize| (kind, len);
        let (kind, len) = match c {
            ' ' | '\t' => {
                i += 1;
                continue;
            }
            '\n' | '\r' => {
                return Err(VclError::new(pos, "newlines are not permitted"));
            }
            '-' => simple(Kind::Dash, 1),
            '(' => simple(Kind::Open, 1),
            ')' => simple(Kind::Close, 1),
            '{' => simple(Kind::LCurly, 1),
            '}' => simple(Kind::RCurly, 1),
            ';' => simple(Kind::Semi, 1),
            ',' => simple(Kind::Comma, 1),
            '.' => simple(Kind::Dot, 1),
            '*' => simple(Kind::Star, 1),
            '=' => simple(Kind::Eq, 1),
            '/' => simple(Kind::Regex, 1),
            '^' => simple(Kind::In, 1),
            '?' => simple(Kind::Exists, 1),
            '<' => match chars.get(i + 1) {
                Some('<') => simple(Kind::IsA, 2),
                Some('!') => simple(Kind::ChildOf, 2),
                _ => simple(Kind::DescOf, 1),
            },
            '>' => match chars.get(i + 1) {
                Some('>') => simple(Kind::Generalizes, 2),
                _ => return Err(VclError::new(pos, "unexpected '>' (did you mean '>>'?)")),
            },
            '~' => match (chars.get(i + 1), chars.get(i + 2)) {
                (Some('<'), Some('<')) => simple(Kind::IsNotA, 3),
                (Some('^'), _) => simple(Kind::NotIn, 2),
                _ => {
                    return Err(VclError::new(
                        pos,
                        "unexpected '~' (expected '~<<' or '~^')",
                    ));
                }
            },
            '!' => {
                if chars.get(i + 1) == Some(&'!') && chars.get(i + 2) == Some(&'<') {
                    simple(Kind::DescLeaf, 3)
                } else {
                    return Err(VclError::new(pos, "unexpected '!' (expected '!!<')"));
                }
            }
            '"' => {
                let mut value = String::new();
                let mut j = i + 1;
                loop {
                    match chars.get(j) {
                        None => {
                            return Err(VclError::new(pos, "unterminated quoted value"));
                        }
                        Some('"') => break,
                        Some('\\') => match chars.get(j + 1) {
                            Some(&e @ ('"' | '\\')) => {
                                value.push(e);
                                j += 2;
                            }
                            _ => {
                                return Err(VclError::new(
                                    j + 1,
                                    "invalid escape (only \\\" and \\\\ are allowed)",
                                ));
                            }
                        },
                        Some(&ch) => {
                            value.push(ch);
                            j += 1;
                        }
                    }
                }
                (
                    Kind::Code {
                        value,
                        quoted: true,
                    },
                    j + 1 - i,
                )
            }
            c if c.is_ascii_alphanumeric() => {
                // A URI is letters, ':', then at least one URI character.
                let mut j = i;
                while j < n && chars[j].is_ascii_alphabetic() {
                    j += 1;
                }
                if j > i && j + 1 < n && chars[j] == ':' && is_uri_char(chars[j + 1]) {
                    let mut k = j + 1;
                    while k < n && is_uri_char(chars[k]) {
                        k += 1;
                    }
                    if chars.get(k) == Some(&'|') {
                        k += 1;
                        while k < n && !matches!(chars[k], '|' | '(' | ')' | '\n' | '\r') {
                            k += 1;
                        }
                        // Trailing blanks are separators, not part of the version.
                        while matches!(chars[k - 1], ' ' | '\t') {
                            k -= 1;
                        }
                    }
                    (Kind::Uri(chars[i..k].iter().collect()), k - i)
                } else {
                    let mut k = i + 1;
                    while k < n
                        && (chars[k].is_ascii_alphanumeric() || matches!(chars[k], '-' | '_'))
                    {
                        k += 1;
                    }
                    (
                        Kind::Code {
                            value: chars[i..k].iter().collect(),
                            quoted: false,
                        },
                        k - i,
                    )
                }
            }
            other => {
                return Err(VclError::new(
                    pos,
                    format!("unexpected character '{other}'"),
                ));
            }
        };
        toks.push(Tok { kind, pos });
        i += len;
    }
    toks.push(Tok {
        kind: Kind::Eof,
        pos: n + 1,
    });
    Ok(toks)
}

// ── Parser ────────────────────────────────────────────────────────────────────

/// Parse a (percent-decoded) VCL expression.
///
/// # Errors
///
/// Returns a [`VclError`] with the 1-based column of the first problem:
/// lexical errors, grammar violations, an empty expression, input longer than
/// [`MAX_EXPRESSION_LEN`], or nesting deeper than [`MAX_NESTING_DEPTH`].
pub fn parse(input: &str) -> Result<Expr, VclError> {
    if input.len() > MAX_EXPRESSION_LEN {
        return Err(VclError::new(
            MAX_EXPRESSION_LEN + 1,
            format!("expression exceeds the maximum length of {MAX_EXPRESSION_LEN} bytes"),
        ));
    }
    let toks = lex(input)?;
    let mut p = Parser {
        toks,
        idx: 0,
        depth: 0,
    };
    if p.kind() == &Kind::Eof {
        return Err(VclError::new(p.pos(), "empty expression"));
    }
    let expr = p.parse_expr()?;
    if p.kind() != &Kind::Eof {
        return Err(p.unexpected("end of expression"));
    }
    Ok(expr)
}

enum Item {
    Code(String),
    Filter(Filter),
}

enum Braced {
    Codes(Vec<String>),
    Filters(Vec<Filter>),
}

struct Parser {
    toks: Vec<Tok>,
    idx: usize,
    depth: usize,
}

fn is_filter_op(kind: &Kind) -> bool {
    matches!(
        kind,
        Kind::Eq
            | Kind::IsA
            | Kind::IsNotA
            | Kind::DescOf
            | Kind::Regex
            | Kind::In
            | Kind::NotIn
            | Kind::Generalizes
            | Kind::ChildOf
            | Kind::DescLeaf
            | Kind::Exists
    )
}

/// Operators that may appear without a property (extension: property
/// defaults to `concept`).
fn is_hierarchical_op(kind: &Kind) -> bool {
    matches!(
        kind,
        Kind::IsA
            | Kind::IsNotA
            | Kind::DescOf
            | Kind::Generalizes
            | Kind::ChildOf
            | Kind::DescLeaf
    )
}

fn op_of(kind: &Kind) -> FilterOp {
    match kind {
        Kind::Eq => FilterOp::Eq,
        Kind::IsA => FilterOp::IsA,
        Kind::IsNotA => FilterOp::IsNotA,
        Kind::DescOf => FilterOp::DescendantOf,
        Kind::Regex => FilterOp::Regex,
        Kind::In => FilterOp::In,
        Kind::NotIn => FilterOp::NotIn,
        Kind::Generalizes => FilterOp::Generalizes,
        Kind::ChildOf => FilterOp::ChildOf,
        Kind::DescLeaf => FilterOp::DescendantLeaf,
        _ => FilterOp::Exists,
    }
}

fn uri_ref(raw: &str, pos: usize) -> UriRef {
    match raw.split_once('|') {
        Some((uri, version)) => UriRef {
            pos,
            uri: uri.to_owned(),
            version: Some(version.to_owned()),
        },
        None => UriRef {
            pos,
            uri: raw.to_owned(),
            version: None,
        },
    }
}

impl Parser {
    fn kind(&self) -> &Kind {
        &self.toks[self.idx].kind
    }

    fn kind_at(&self, offset: usize) -> &Kind {
        let i = (self.idx + offset).min(self.toks.len() - 1);
        &self.toks[i].kind
    }

    fn pos(&self) -> usize {
        self.toks[self.idx].pos
    }

    fn bump(&mut self) -> Tok {
        let t = self.toks[self.idx].clone();
        if self.idx < self.toks.len() - 1 {
            self.idx += 1;
        }
        t
    }

    fn unexpected(&self, expected: &str) -> VclError {
        VclError::new(
            self.pos(),
            format!("expected {expected}, found {}", describe(self.kind())),
        )
    }

    fn expect(&mut self, kind: Kind) -> Result<Tok, VclError> {
        if self.kind() == &kind {
            Ok(self.bump())
        } else {
            Err(self.unexpected(&describe(&kind)))
        }
    }

    fn enter(&mut self) -> Result<(), VclError> {
        if self.depth >= MAX_NESTING_DEPTH {
            return Err(VclError::new(
                self.pos(),
                format!("nesting is deeper than the maximum of {MAX_NESTING_DEPTH}"),
            ));
        }
        self.depth += 1;
        Ok(())
    }

    /// `expr : subExpr (conjunction | disjunction | exclusion)?`
    fn parse_expr(&mut self) -> Result<Expr, VclError> {
        let first = self.parse_sub_expr()?;
        let pos = first.pos;
        let kind = match self.kind() {
            Kind::Comma => ExprKind::Conjunction(self.parse_operands(first, &Kind::Comma)?),
            Kind::Semi => ExprKind::Disjunction(self.parse_operands(first, &Kind::Semi)?),
            Kind::Dash => {
                self.bump();
                let right = self.parse_sub_expr()?;
                ExprKind::Exclusion {
                    left: Box::new(first),
                    right: Box::new(right),
                }
            }
            _ => return Ok(first),
        };
        if matches!(self.kind(), Kind::Comma | Kind::Semi | Kind::Dash) {
            return Err(VclError::new(
                self.pos(),
                format!(
                    "{} cannot be mixed with other ',' ';' '-' operators at the same level; use parentheses",
                    describe(self.kind())
                ),
            ));
        }
        Ok(Expr { pos, kind })
    }

    fn parse_operands(&mut self, first: Expr, sep: &Kind) -> Result<Vec<Expr>, VclError> {
        let mut items = vec![first];
        while self.kind() == sep {
            self.bump();
            items.push(self.parse_sub_expr()?);
        }
        Ok(items)
    }

    /// `subExpr : systemUri? (simpleExpr | '(' expr ')')`
    fn parse_sub_expr(&mut self) -> Result<Expr, VclError> {
        let start = self.pos();
        let system = if self.kind() == &Kind::Open && matches!(self.kind_at(1), Kind::Uri(_)) {
            self.bump();
            let t = self.bump();
            self.expect(Kind::Close)?;
            let Kind::Uri(raw) = t.kind else {
                unreachable!("checked above")
            };
            Some(uri_ref(&raw, t.pos))
        } else {
            None
        };
        let inner = if self.kind() == &Kind::Open {
            self.enter()?;
            self.bump();
            let e = self.parse_expr()?;
            self.expect(Kind::Close)?;
            self.depth -= 1;
            e
        } else {
            self.parse_simple_expr()?
        };
        Ok(match system {
            Some(system) => Expr {
                pos: start,
                kind: ExprKind::Scoped {
                    system,
                    expr: Box::new(inner),
                },
            },
            None => inner,
        })
    }

    /// `simpleExpr : STAR | code | filter | includeVs`
    fn parse_simple_expr(&mut self) -> Result<Expr, VclError> {
        let pos = self.pos();
        match self.kind() {
            Kind::Star if self.kind_at(1) != &Kind::Dot => {
                self.bump();
                Ok(Expr {
                    pos,
                    kind: ExprKind::All,
                })
            }
            Kind::In => {
                self.bump();
                let r = match self.kind().clone() {
                    Kind::Uri(raw) => {
                        let t = self.bump();
                        uri_ref(&raw, t.pos)
                    }
                    Kind::Open if matches!(self.kind_at(1), Kind::Uri(_)) => {
                        self.bump();
                        let t = self.bump();
                        self.expect(Kind::Close)?;
                        let Kind::Uri(raw) = t.kind else {
                            unreachable!("checked above")
                        };
                        uri_ref(&raw, t.pos)
                    }
                    _ => return Err(self.unexpected("a value set URI or '(URI)' after '^'")),
                };
                Ok(Expr {
                    pos,
                    kind: ExprKind::IncludeValueSet(r),
                })
            }
            _ => match self.parse_item()? {
                Item::Code(c) => Ok(Expr {
                    pos,
                    kind: ExprKind::Code(c),
                }),
                Item::Filter(f) => Ok(Expr {
                    pos,
                    kind: ExprKind::Filter(f),
                }),
            },
        }
    }

    /// A code, or one filter (`prop op value`, `{src}.prop`, or an implicit
    /// `concept` filter such as `<<X`).
    fn parse_item(&mut self) -> Result<Item, VclError> {
        let pos = self.pos();
        match self.kind().clone() {
            Kind::Code { value, .. } => {
                self.bump();
                if is_filter_op(self.kind()) {
                    Ok(Item::Filter(self.parse_filter_tail(pos, value)?))
                } else if self.kind() == &Kind::Dot {
                    Ok(Item::Filter(self.parse_of(pos, FilterValue::Code(value))?))
                } else {
                    Ok(Item::Code(value))
                }
            }
            k if is_hierarchical_op(&k) => Ok(Item::Filter(
                self.parse_filter_tail(pos, "concept".to_owned())?,
            )),
            Kind::Star => {
                self.bump();
                Ok(Item::Filter(self.parse_of(pos, FilterValue::All)?))
            }
            Kind::Uri(raw) => {
                self.bump();
                Ok(Item::Filter(
                    self.parse_of(pos, FilterValue::Uri(uri_ref(&raw, pos)))?,
                ))
            }
            Kind::LCurly => {
                let value = match self.parse_braces()? {
                    Braced::Codes(c) => FilterValue::CodeList(c),
                    Braced::Filters(f) => FilterValue::FilterList(f),
                };
                Ok(Item::Filter(self.parse_of(pos, value)?))
            }
            _ => Err(self.unexpected("a code, a filter, '*', '^' or '('")),
        }
    }

    /// `'.' property` after an "of" source.
    fn parse_of(&mut self, pos: usize, source: FilterValue) -> Result<Filter, VclError> {
        self.expect(Kind::Dot)?;
        match self.kind().clone() {
            Kind::Code { value, .. } => {
                self.bump();
                Ok(Filter {
                    pos,
                    property: value,
                    op: FilterOp::Of,
                    value: source,
                })
            }
            _ => Err(self.unexpected("a property after '.'")),
        }
    }

    /// The operator and value of a filter whose property is already consumed.
    fn parse_filter_tail(&mut self, pos: usize, property: String) -> Result<Filter, VclError> {
        let op_tok = self.bump();
        let op = op_of(&op_tok.kind);
        let value = match op_tok.kind {
            Kind::Regex => match self.kind().clone() {
                Kind::Code {
                    value,
                    quoted: true,
                } => {
                    self.bump();
                    FilterValue::Regex(value)
                }
                _ => return Err(self.unexpected("a quoted regular expression after '/'")),
            },
            Kind::Exists => match self.kind().clone() {
                Kind::Code { value, .. } if value == "true" || value == "false" => {
                    self.bump();
                    FilterValue::Code(value)
                }
                _ => return Err(self.unexpected("'true' or 'false' after '?'")),
            },
            Kind::In | Kind::NotIn => match self.kind().clone() {
                Kind::Uri(raw) => {
                    let t = self.bump();
                    FilterValue::Uri(uri_ref(&raw, t.pos))
                }
                Kind::LCurly => match self.parse_braces()? {
                    Braced::Codes(c) => FilterValue::CodeList(c),
                    Braced::Filters(f) => FilterValue::FilterList(f),
                },
                _ => {
                    return Err(self.unexpected(&format!(
                        "a '{{…}}' list or a URI after {}",
                        describe(&op_tok.kind)
                    )));
                }
            },
            _ => match self.kind().clone() {
                Kind::Code { value, .. } => {
                    self.bump();
                    FilterValue::Code(value)
                }
                _ => {
                    return Err(
                        self.unexpected(&format!("a code after {}", describe(&op_tok.kind)))
                    );
                }
            },
        };
        Ok(Filter {
            pos,
            property,
            op,
            value,
        })
    }

    /// `{code,…}` or `{filter,…}`; the first element decides which.
    fn parse_braces(&mut self) -> Result<Braced, VclError> {
        self.enter()?;
        self.expect(Kind::LCurly)?;
        let first = self.parse_item()?;
        let out = match first {
            Item::Code(c) => {
                let mut codes = vec![c];
                while self.kind() == &Kind::Comma {
                    self.bump();
                    match self.kind().clone() {
                        Kind::Code { value, .. } if !is_filter_op(self.kind_at(1)) => {
                            self.bump();
                            codes.push(value);
                        }
                        _ => {
                            return Err(
                                self.unexpected("a code (lists cannot mix codes and filters)")
                            );
                        }
                    }
                }
                Braced::Codes(codes)
            }
            Item::Filter(f) => {
                let mut filters = vec![f];
                while self.kind() == &Kind::Comma {
                    self.bump();
                    match self.parse_item()? {
                        Item::Filter(f) => filters.push(f),
                        Item::Code(_) => {
                            return Err(VclError::new(
                                self.toks[self.idx.saturating_sub(1)].pos,
                                "expected a filter (lists cannot mix codes and filters)",
                            ));
                        }
                    }
                }
                Braced::Filters(filters)
            }
        };
        self.expect(Kind::RCurly)?;
        self.depth -= 1;
        Ok(out)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(s: &str) -> Expr {
        parse(s).unwrap_or_else(|e| panic!("`{s}` should parse: {e}"))
    }

    fn err(s: &str) -> VclError {
        parse(s).expect_err(&format!("`{s}` should not parse"))
    }

    fn code(s: &str) -> ExprKind {
        ExprKind::Code(s.to_owned())
    }

    fn filter_of(e: &Expr) -> &Filter {
        match &e.kind {
            ExprKind::Filter(f) => f,
            other => panic!("expected a filter, got {other:?}"),
        }
    }

    fn scoped(e: &Expr) -> (&UriRef, &Expr) {
        match &e.kind {
            ExprKind::Scoped { system, expr } => (system, expr),
            other => panic!("expected a scoped expression, got {other:?}"),
        }
    }

    // ── Published valid cases (Java, vcl-tests.js, smith-test.js) ────────────

    #[test]
    fn published_valid_expressions_all_parse() {
        let valid = [
            "(subscriber;provider)",
            "(http://loinc.org)(41995-2;4548-4;4549-2;17855-8;17856-6;62388-4;71875-9;59261-8;86910-7);(http://snomed.info/sct)(365845005;165679005;165680008;65681007;451061000124104;451051000124101);(http://www.ama-assn.org/go/cpt)(83036;83037;3044F;3046F)",
            "((http://snomed.info/sct)concept<<17311000168105;(http://snomed.info/sct)(61796011000036105;923929011000036103);(http://loinc.org)ancestor=LP185676-6)-((http://loinc.org)76573-5)",
            "(http://hl7.org/fhir/paymentstatus)paid;(http://hl7.org/fhir/payeetype)provider",
            "^(http://hl7.org/fhir/ValueSet/payeetype)",
            "ancestor=LP185676-6",
            r#"COMPONENT/".*Dichloroethane.*""#,
            "COMPONENT=LP15653-6",
            "(10007-3;10008-1)",
            "concept<<17311000168105",
            "concept~<<929360061000036106",
            "A",
            "A,B",
            "A;B",
            "(http://snomed.info/sct)*",
            "(in-progress;aborted;completed;entered-in-error)",
            "(^(http://csiro.au/fhir/ValueSet/selfexclude))-(^(http://csiro.au/fhir/ValueSet/selfexclude))",
            "(^(http://csiro.au/fhir/ValueSet/selfimport))-(^(http://csiro.au/fhir/ValueSet/selfexcludeA))",
            r#"concept<<"_ActNoImmunizationReason""#,
            "has_ingredient = 1886",
            r#"(constraint="<< 30506011000036107 |australian product|: 700000101000036108 |hasTP| = 17311000168105 |PANADOL|",expression="<< 30506011000036107 |australian product|: 700000101000036108 |hasTP| = 17311000168105 |PANADOL|")"#,
            "(has_ingredient=1886, has_dose_form=317541)",
            "123456",
            "test-code",
            r#""quoted code""#,
            "*",
            "^(http://hl7.org/fhir/ValueSet/test1)",
            "^(http://example.org/valueset)",
            "(http://example.com/cs/meds) (     {concept<<Panadol}.container,     {Panadol_500mg_24.unit_of_use}.active_ingredient,     concept<<12345,     concept^{12346,6374673,9234983,38924839,212,43243},     ingredient^{parent=98765, concept<<Panadol}   )",
            "(http://snomed.info/sct|20210131)123456789",
            r#""code with spaces""#,
            "panadol.contains",
            "{panadol,herons}.contains",
            "(http://example.com/CodeSystem/a)has_ingredient^{has_tradename=2201670}",
            r#"COMPONENT=".*test.*""#,
            "^(http://hl7.org/fhir/ValueSet/example)",
            "(A;B)-(C)",
            "(http://snomed.info/sct)<<404684003 - (http://snomed.info/sct)<<64572001",
        ];
        for s in valid {
            ok(s);
        }
    }

    #[test]
    fn spec_examples_not_covered_by_published_tests_parse() {
        let valid = [
            "((10007-3;10008-1);(COMPONENT=LP212516-1,PROPERTY=LP6817-3,TIME_ASPCT=LP6960-1,SYSTEM=LP28433-8))-(29557-6)",
            "(http://loinc.org)10007-3;^http://loinc.org/vs/LP257682-7",
            "(http://loinc.org)(parent^{LP46821-2,LP259418-4})",
            "concept^http://snomed.info/sct?fhir_vs/refset=929360061000036106",
            "concept<<chol-mmol",
            "{concept<<panadol}.contains",
            "{a.b}.c",
            "consists_of^{has_ingredient = 1886}",
            "consists_of^{has_ingredient^{has_tradename=2201670}}",
            "(has_ingredient=1886, has_dose_form^{concept<<1151133})",
            "ingredient?true",
            r#"code/"A[0-9]*\\.9""#,
            "has_ingredient^http://acme.com/VS/controlled",
            "concept~^{a,b}",
            "concept>>X",
            "concept<!X",
            "concept!!<X",
            "concept<X",
            "*.contains",
        ];
        for s in valid {
            ok(s);
        }
    }

    // ── Shapes ────────────────────────────────────────────────────────────────

    #[test]
    fn single_code_and_quoted_code() {
        assert_eq!(ok("A").kind, code("A"));
        assert_eq!(ok("test-code").kind, code("test-code"));
        assert_eq!(ok(r#""code with spaces""#).kind, code("code with spaces"));
        assert_eq!(ok("123456").kind, code("123456"));
    }

    #[test]
    fn conjunction_disjunction_exclusion() {
        assert_eq!(
            ok("A,B,C").kind,
            ExprKind::Conjunction(vec![
                Expr {
                    pos: 1,
                    kind: code("A")
                },
                Expr {
                    pos: 3,
                    kind: code("B")
                },
                Expr {
                    pos: 5,
                    kind: code("C")
                },
            ])
        );
        let ExprKind::Disjunction(items) = ok("A;B").kind else {
            panic!("expected a disjunction")
        };
        assert_eq!(items.len(), 2);
        let ExprKind::Exclusion { left, right } = ok("(A;B)-(C)").kind else {
            panic!("expected an exclusion")
        };
        assert!(matches!(left.kind, ExprKind::Disjunction(_)));
        assert_eq!(right.kind, code("C"));
    }

    #[test]
    fn dash_inside_a_code_is_not_exclusion() {
        assert_eq!(ok("A-B").kind, code("A-B"));
        assert!(matches!(ok("A - B").kind, ExprKind::Exclusion { .. }));
    }

    #[test]
    fn system_scope_with_and_without_version() {
        let e = ok("(http://snomed.info/sct|20210131)123456789");
        let (sys, inner) = scoped(&e);
        assert_eq!(sys.uri, "http://snomed.info/sct");
        assert_eq!(sys.version.as_deref(), Some("20210131"));
        assert_eq!(sys.pos, 2);
        assert_eq!(inner.kind, code("123456789"));

        let e = ok("(http://snomed.info/sct)123456789");
        assert_eq!(scoped(&e).0.version, None);
    }

    #[test]
    fn system_scope_over_group() {
        let e = ok("(http://loinc.org)(1-1;2-2)");
        let (_, inner) = scoped(&e);
        assert!(matches!(inner.kind, ExprKind::Disjunction(_)));
    }

    #[test]
    fn wildcard_and_wildcard_in_system() {
        assert_eq!(ok("*").kind, ExprKind::All);
        let e = ok("(http://snomed.info/sct)*");
        assert_eq!(scoped(&e).1.kind, ExprKind::All);
    }

    #[test]
    fn include_value_set_forms() {
        let ExprKind::IncludeValueSet(u) = ok("^(http://hl7.org/fhir/ValueSet/example)").kind
        else {
            panic!("expected include")
        };
        assert_eq!(u.uri, "http://hl7.org/fhir/ValueSet/example");
        let ExprKind::IncludeValueSet(u) = ok("^http://loinc.org/vs/LP1|2").kind else {
            panic!("expected include")
        };
        assert_eq!(u.uri, "http://loinc.org/vs/LP1");
        assert_eq!(u.version.as_deref(), Some("2"));
    }

    #[test]
    fn every_filter_operator() {
        let cases = [
            ("p=v", FilterOp::Eq),
            ("p<<v", FilterOp::IsA),
            ("p~<<v", FilterOp::IsNotA),
            ("p<v", FilterOp::DescendantOf),
            ("p>>v", FilterOp::Generalizes),
            ("p<!v", FilterOp::ChildOf),
            ("p!!<v", FilterOp::DescendantLeaf),
            ("p?true", FilterOp::Exists),
            ("p?false", FilterOp::Exists),
        ];
        for (s, op) in cases {
            let e = ok(s);
            let f = filter_of(&e);
            assert_eq!(f.op, op, "{s}");
            assert_eq!(f.property, "p");
            assert!(matches!(f.value, FilterValue::Code(_)), "{s}");
        }
    }

    #[test]
    fn regex_filter_unescapes_the_pattern() {
        let e = ok(r#"code/"A[0-9]*\\.9""#);
        let f = filter_of(&e);
        assert_eq!(f.op, FilterOp::Regex);
        assert_eq!(f.value, FilterValue::Regex(r"A[0-9]*\.9".to_owned()));
    }

    #[test]
    fn in_and_not_in_values() {
        let e = ok("parent^{LP46821-2,LP259418-4}");
        let f = filter_of(&e);
        assert_eq!(f.op, FilterOp::In);
        assert_eq!(
            f.value,
            FilterValue::CodeList(vec!["LP46821-2".into(), "LP259418-4".into()])
        );
        let e = ok("p~^{a,b}");
        assert_eq!(filter_of(&e).op, FilterOp::NotIn);
        let e = ok("p^{A}");
        assert_eq!(filter_of(&e).value, FilterValue::CodeList(vec!["A".into()]));
        let e = ok("has_ingredient^http://acme.com/VS/controlled");
        assert!(matches!(filter_of(&e).value, FilterValue::Uri(_)));
        let e = ok("p^{q=1,r<<2}");
        let FilterValue::FilterList(fs) = &filter_of(&e).value else {
            panic!("expected a filter list")
        };
        assert_eq!(fs.len(), 2);
        assert_eq!(fs[1].op, FilterOp::IsA);
    }

    #[test]
    fn of_filters() {
        let e = ok("panadol.contains");
        let f = filter_of(&e);
        assert_eq!(f.op, FilterOp::Of);
        assert_eq!(f.property, "contains");
        assert_eq!(f.value, FilterValue::Code("panadol".into()));
        let e = ok("{panadol,herons}.contains");
        assert_eq!(
            filter_of(&e).value,
            FilterValue::CodeList(vec!["panadol".into(), "herons".into()])
        );
        let e = ok("{concept<<panadol}.contains");
        assert!(matches!(filter_of(&e).value, FilterValue::FilterList(_)));
        let e = ok("*.contains");
        assert_eq!(filter_of(&e).value, FilterValue::All);
    }

    #[test]
    fn hierarchical_operator_without_property_defaults_to_concept() {
        let e = ok("(http://snomed.info/sct)<<404684003");
        let (_, inner) = scoped(&e);
        let f = filter_of(inner);
        assert_eq!(f.property, "concept");
        assert_eq!(f.op, FilterOp::IsA);
        assert_eq!(f.value, FilterValue::Code("404684003".into()));
        assert_eq!(filter_of(&ok("~<<X")).op, FilterOp::IsNotA);
    }

    #[test]
    fn issue_example_is_an_exclusion_of_two_scoped_filters() {
        let e = ok("(http://snomed.info/sct)<<404684003 - (http://snomed.info/sct)<<64572001");
        let ExprKind::Exclusion { left, right } = e.kind else {
            panic!("expected an exclusion")
        };
        assert!(matches!(left.kind, ExprKind::Scoped { .. }));
        assert!(matches!(right.kind, ExprKind::Scoped { .. }));
    }

    #[test]
    fn whitespace_is_skipped() {
        assert_eq!(ok("  A \t, B ").pos, 3);
        assert_eq!(ok("has_ingredient = 1886"), ok("has_ingredient=1886"));
    }

    #[test]
    fn uri_may_contain_separator_characters() {
        let e = ok("^http://x.org/vs?a=1;b=2");
        let ExprKind::IncludeValueSet(u) = e.kind else {
            panic!("expected include")
        };
        assert_eq!(u.uri, "http://x.org/vs?a=1;b=2");
    }

    // ── Published invalid cases, with positions ──────────────────────────────

    #[test]
    fn empty_and_blank_are_errors() {
        assert_eq!(err("").pos, 1);
        assert_eq!(err("").message, "empty expression");
        assert_eq!(err("   ").pos, 4);
    }

    #[test]
    fn unclosed_group_reports_end_of_input() {
        let e = err("((unclosed");
        assert_eq!(e.pos, 11);
        assert!(e.message.contains("')'"), "{}", e.message);
    }

    #[test]
    fn adjacent_codes_are_an_error() {
        assert_eq!(err("A B C").pos, 3);
    }

    #[test]
    fn filter_without_property_is_an_error() {
        assert_eq!(err("=value").pos, 1);
        assert_eq!(err("/\"x\"").pos, 1);
        assert_eq!(err("?true").pos, 1);
    }

    #[test]
    fn round_brackets_after_in_are_an_error() {
        assert_eq!(err("has_ingredient^(has_tradename=2201670)").pos, 16);
    }

    #[test]
    fn mixing_operators_without_parentheses_is_an_error() {
        assert_eq!(err("A,B;C").pos, 4);
        assert_eq!(err("A;B,C").pos, 4);
        assert_eq!(err("A,B - C").pos, 5);
        assert_eq!(err("A - B - C").pos, 7);
        assert_eq!(err("(A,B;C)").pos, 5);
        ok("(A,B);C");
        ok("(A;B)-(C)");
        ok("((A)-(B))-(C)");
    }

    #[test]
    fn dangling_operators_are_errors() {
        assert_eq!(err("A,").pos, 3);
        assert_eq!(err("A -").pos, 4);
        assert_eq!(ok("A-").kind, code("A-"));
        assert_eq!(err("(http://x.org/cs)").pos, 18);
        assert_eq!(err("p=").pos, 3);
        assert_eq!(err("p^").pos, 3);
        assert_eq!(err("p.").pos, 3);
        assert_eq!(err("A)").pos, 2);
    }

    #[test]
    fn lexical_errors_carry_the_offending_column() {
        assert_eq!(err("A @").pos, 3);
        assert_eq!(err("A > B").pos, 3);
        assert_eq!(err("A ~ B").pos, 3);
        assert_eq!(err("A ! B").pos, 3);
        assert_eq!(err("A\nB").pos, 2);
        assert_eq!(err(r#"A="open"#).pos, 3);
        assert_eq!(err(r#"A="a\x""#).pos, 5);
    }

    #[test]
    fn exists_requires_true_or_false() {
        assert_eq!(err("p?maybe").pos, 3);
    }

    #[test]
    fn regex_requires_a_quoted_value() {
        assert_eq!(err("p/abc").pos, 3);
    }

    #[test]
    fn lists_cannot_mix_codes_and_filters() {
        err("p^{a,q=1}");
        err("p^{q=1,a}");
        err("p^{}");
        err("p^{a,}");
    }

    // ── Limits ────────────────────────────────────────────────────────────────

    #[test]
    fn nesting_limit() {
        let nested = |n: usize| format!("{}A{}", "(".repeat(n), ")".repeat(n));
        ok(&nested(MAX_NESTING_DEPTH));
        let e = err(&nested(MAX_NESTING_DEPTH + 1));
        assert_eq!(e.pos, MAX_NESTING_DEPTH + 1);
        assert!(e.message.contains("deeper"), "{}", e.message);

        let braces = |n: usize| format!("p^{}a{}", "{".repeat(n), "}".repeat(n));
        err(&braces(MAX_NESTING_DEPTH + 1));
    }

    #[test]
    fn length_limit() {
        let long = "A".repeat(MAX_EXPRESSION_LEN + 1);
        let e = err(&long);
        assert_eq!(e.pos, MAX_EXPRESSION_LEN + 1);
        ok(&"A".repeat(MAX_EXPRESSION_LEN));
    }

    #[test]
    fn display_includes_the_position() {
        assert_eq!(
            err("=x").to_string(),
            format!("{} at position 1", err("=x").message)
        );
    }
}
