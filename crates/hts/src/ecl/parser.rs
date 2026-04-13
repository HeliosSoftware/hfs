//! ECL (Expression Constraint Language) tokenizer, AST, and recursive-descent
//! parser.
//!
//! Implements the subset of SNOMED CT ECL defined by SNOMED International:
//! <https://confluence.ihtsdotools.org/display/DOCECL>
//!
//! ## Supported constructs
//!
//! | Syntax | Meaning |
//! |--------|---------|
//! | `123456789` | Exact concept by SCTID |
//! | `my-code` | Exact concept by alphanumeric code (HTS extension — see below) |
//! | `code \|display label\|` | Concept with pipe-quoted label (label stripped) |
//! | `*` | Wildcard — all concepts in the system |
//! | `< code` | Strict descendants (not self) |
//! | `<< code` | Self or descendants |
//! | `> code` | Strict ancestors (not self) |
//! | `>> code` | Self or ancestors |
//! | `A AND B` / `A , B` | Conjunction (intersection) |
//! | `A OR B` | Disjunction (union) |
//! | `A MINUS B` | Exclusion (set difference) |
//! | `( expr )` | Grouping (overrides default precedence) |
//!
//! ## Operator precedence (highest → lowest)
//!
//! 1. Unary concept operators (`<`, `<<`, `>`, `>>`) — tightest binding
//! 2. `AND` / `,`
//! 3. `MINUS`
//! 4. `OR` — loosest binding
//!
//! ## Extension: alphanumeric concept codes
//!
//! The ECL specification was designed for SNOMED CT whose concepts are
//! identified by numeric SCTIDs.  HTS extends the tokenizer to also accept
//! alphanumeric identifiers (letters, digits, `_`, `-`) so that non-SNOMED
//! code systems with human-readable codes (e.g. anatomy, UCUM units) can be
//! constrained using the same ECL syntax.
//!
//! The reserved keywords `AND`, `OR`, and `MINUS` (case-insensitive) are
//! never treated as concept codes.  All other alphabetic tokens are concept
//! codes.

// ── Tokens ────────────────────────────────────────────────────────────────────

/// Lexical tokens produced by [`tokenize`].
///
/// The tokenizer emits a flat sequence of these; the parser then consumes them
/// left-to-right via a recursive-descent grammar.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Token {
    /// A concept identifier — either a numeric SCTID (`404684003`) or an
    /// alphanumeric code (`arm`, `A11Y-code`).
    ConceptId(String),
    /// `*` — matches every concept in the system.
    Wildcard,
    /// `<<` — self-or-descendant operator.
    LtLt,
    /// `<` — strict-descendant operator.
    Lt,
    /// `>>` — self-or-ancestor operator.
    GtGt,
    /// `>` — strict-ancestor operator.
    Gt,
    /// `AND` or `,` — conjunction (intersection).
    And,
    /// `OR` — disjunction (union).
    Or,
    /// `MINUS` — set difference.
    Minus,
    /// `(` — open group.
    LParen,
    /// `)` — close group.
    RParen,
}

// ── AST ───────────────────────────────────────────────────────────────────────

/// A concept operator applied to a focus concept.
#[derive(Debug, Clone, PartialEq)]
pub enum ConceptOperator {
    /// `<`  — strict descendants only
    DescendantOf,
    /// `<<` — self or descendants
    DescendantOrSelf,
    /// `>`  — strict ancestors only
    AncestorOf,
    /// `>>` — self or ancestors
    AncestorOrSelf,
}

/// The focus (target) of a concept operator.
#[derive(Debug, Clone, PartialEq)]
pub enum FocusConcept {
    /// A specific SNOMED CT concept identified by its SCTID.
    Id(String),
    /// The `*` wildcard — matches every concept in the system.
    Wildcard,
}

/// An ECL expression node.
#[derive(Debug, Clone, PartialEq)]
pub enum EclExpr {
    /// A focus concept with an optional unary operator.
    Focus {
        op: Option<ConceptOperator>,
        concept: FocusConcept,
    },
    /// `A AND B` — intersection.
    And(Box<EclExpr>, Box<EclExpr>),
    /// `A OR B` — union.
    Or(Box<EclExpr>, Box<EclExpr>),
    /// `A MINUS B` — set difference.
    Minus(Box<EclExpr>, Box<EclExpr>),
}

// ── Tokenizer ─────────────────────────────────────────────────────────────────

/// Tokenize an ECL string into a flat [`Token`] sequence.
///
/// Whitespace is silently consumed.  Pipe-delimited display labels
/// (`|Clinical finding|`) are consumed and discarded — they carry no
/// semantic content for evaluation purposes.
///
/// # Errors
///
/// Returns `Err(message)` when an unrecognised character (e.g. `@`, `%`)
/// is encountered outside a pipe label.
pub(super) fn tokenize(input: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = input.chars().collect();
    let mut pos = 0;
    let mut tokens = Vec::new();

    while pos < chars.len() {
        // Skip whitespace.
        if chars[pos].is_whitespace() {
            pos += 1;
            continue;
        }

        // Skip pipe-delimited display labels: | ... |
        if chars[pos] == '|' {
            pos += 1;
            while pos < chars.len() && chars[pos] != '|' {
                pos += 1;
            }
            pos += 1; // consume closing '|'
            continue;
        }

        match chars[pos] {
            '*' => {
                tokens.push(Token::Wildcard);
                pos += 1;
            }
            '(' => {
                tokens.push(Token::LParen);
                pos += 1;
            }
            ')' => {
                tokens.push(Token::RParen);
                pos += 1;
            }
            ',' => {
                tokens.push(Token::And);
                pos += 1;
            }
            '<' => {
                if pos + 1 < chars.len() && chars[pos + 1] == '<' {
                    tokens.push(Token::LtLt);
                    pos += 2;
                } else {
                    tokens.push(Token::Lt);
                    pos += 1;
                }
            }
            '>' => {
                if pos + 1 < chars.len() && chars[pos + 1] == '>' {
                    tokens.push(Token::GtGt);
                    pos += 2;
                } else {
                    tokens.push(Token::Gt);
                    pos += 1;
                }
            }
            c if c.is_ascii_digit() => {
                // Allow digit-prefixed alphanumeric codes (e.g. SNOMED SCTIDs
                // as well as codes like "404684003").
                let start = pos;
                while pos < chars.len()
                    && (chars[pos].is_alphanumeric() || chars[pos] == '_' || chars[pos] == '-')
                {
                    pos += 1;
                }
                tokens.push(Token::ConceptId(chars[start..pos].iter().collect()));
            }
            c if c.is_alphabetic() => {
                let start = pos;
                while pos < chars.len()
                    && (chars[pos].is_alphanumeric() || chars[pos] == '_' || chars[pos] == '-')
                {
                    pos += 1;
                }
                let word: String = chars[start..pos].iter().collect();
                // Reserve AND / OR / MINUS as binary operators; treat everything
                // else as a concept code (ECL is used with non-numeric code
                // systems as well as SNOMED numeric SCTIDs).
                match word.to_uppercase().as_str() {
                    "AND" => tokens.push(Token::And),
                    "OR" => tokens.push(Token::Or),
                    "MINUS" => tokens.push(Token::Minus),
                    _ => tokens.push(Token::ConceptId(word)),
                }
            }
            other => {
                return Err(format!("Unexpected character: '{other}'"));
            }
        }
    }

    Ok(tokens)
}

// ── Parser ────────────────────────────────────────────────────────────────────

/// Parse an ECL string into an [`EclExpr`] AST.
///
/// Trims leading/trailing whitespace before tokenising, so callers need not
/// normalise the input string themselves.
///
/// # Errors
///
/// Returns `Err(message)` on any of:
/// - An unrecognised character in the input (propagated from [`tokenize`]).
/// - A parse grammar error (unexpected token or unexpected end of input).
/// - Trailing tokens after a complete expression (e.g. `<< 1 << 2`).
pub fn parse(input: &str) -> Result<EclExpr, String> {
    let tokens = tokenize(input.trim())?;
    let mut p = Parser { tokens, pos: 0 };
    let expr = p.parse_or()?;
    if p.pos < p.tokens.len() {
        return Err(format!(
            "Unexpected token at position {}: {:?}",
            p.pos, p.tokens[p.pos]
        ));
    }
    Ok(expr)
}

/// Recursive-descent parser state.
///
/// `tokens` is the flat token sequence from [`tokenize`]; `pos` tracks the
/// current lookahead position.  All `parse_*` methods advance `pos` as they
/// consume tokens and return the matched [`EclExpr`] sub-tree.
struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    /// Peek at the token at the current position without consuming it.
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    /// Consume and return the token at the current position.
    fn consume(&mut self) -> Option<Token> {
        let tok = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        tok
    }

    /// Consume the next token, asserting it equals `expected`.
    fn expect(&mut self, expected: &Token) -> Result<(), String> {
        match self.consume() {
            Some(ref t) if t == expected => Ok(()),
            Some(t) => Err(format!("Expected {expected:?}, got {t:?}")),
            None => Err(format!("Expected {expected:?}, got end of input")),
        }
    }

    // ── Grammar rules (lowest to highest precedence) ──────────────────────────

    /// `or_expr  →  minus_expr ( OR minus_expr )*`
    ///
    /// OR is left-associative and has the lowest precedence of the binary
    /// operators.
    fn parse_or(&mut self) -> Result<EclExpr, String> {
        let mut left = self.parse_minus()?;
        while self.peek() == Some(&Token::Or) {
            self.consume();
            let right = self.parse_minus()?;
            left = EclExpr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    /// `minus_expr  →  and_expr ( MINUS and_expr )?`
    ///
    /// MINUS is non-associative (only one MINUS per clause without grouping).
    /// Using `A MINUS B MINUS C` without parentheses is a syntax error.
    fn parse_minus(&mut self) -> Result<EclExpr, String> {
        let left = self.parse_and()?;
        if self.peek() == Some(&Token::Minus) {
            self.consume();
            let right = self.parse_and()?;
            return Ok(EclExpr::Minus(Box::new(left), Box::new(right)));
        }
        Ok(left)
    }

    /// `and_expr  →  focus ( AND focus )*`
    ///
    /// AND (and its comma alias `,`) is left-associative and has the highest
    /// precedence among binary operators.
    fn parse_and(&mut self) -> Result<EclExpr, String> {
        let mut left = self.parse_focus()?;
        while self.peek() == Some(&Token::And) {
            self.consume();
            let right = self.parse_focus()?;
            left = EclExpr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    /// `focus  →  "(" or_expr ")"  |  [operator] concept_ref`
    ///
    /// Handles both grouped sub-expressions and individual focus concept
    /// references (with or without a leading unary operator).
    fn parse_focus(&mut self) -> Result<EclExpr, String> {
        if self.peek() == Some(&Token::LParen) {
            self.consume();
            let inner = self.parse_or()?;
            self.expect(&Token::RParen)?;
            return Ok(inner);
        }

        let op = self.try_operator();
        let concept = self.parse_concept_ref()?;
        Ok(EclExpr::Focus { op, concept })
    }

    /// Consume a unary concept operator token if one is present, returning its
    /// [`ConceptOperator`] variant.  Returns `None` without consuming if the
    /// next token is not an operator.
    fn try_operator(&mut self) -> Option<ConceptOperator> {
        match self.peek() {
            Some(Token::LtLt) => {
                self.consume();
                Some(ConceptOperator::DescendantOrSelf)
            }
            Some(Token::Lt) => {
                self.consume();
                Some(ConceptOperator::DescendantOf)
            }
            Some(Token::GtGt) => {
                self.consume();
                Some(ConceptOperator::AncestorOrSelf)
            }
            Some(Token::Gt) => {
                self.consume();
                Some(ConceptOperator::AncestorOf)
            }
            _ => None,
        }
    }

    /// Consume a concept reference token (`*` or a concept ID) and return the
    /// corresponding [`FocusConcept`].
    fn parse_concept_ref(&mut self) -> Result<FocusConcept, String> {
        match self.consume() {
            Some(Token::Wildcard) => Ok(FocusConcept::Wildcard),
            Some(Token::ConceptId(id)) => Ok(FocusConcept::Id(id)),
            Some(other) => Err(format!("Expected concept ID or wildcard, got {other:?}")),
            None => Err("Expected concept ID or wildcard, got end of input".to_string()),
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Tokenizer ─────────────────────────────────────────────────────────────

    #[test]
    fn tokenize_concept_id() {
        assert_eq!(
            tokenize("404684003").unwrap(),
            vec![Token::ConceptId("404684003".into())]
        );
    }

    #[test]
    fn tokenize_wildcard() {
        assert_eq!(tokenize("*").unwrap(), vec![Token::Wildcard]);
    }

    #[test]
    fn tokenize_descendant_of() {
        assert_eq!(
            tokenize("< 404684003").unwrap(),
            vec![Token::Lt, Token::ConceptId("404684003".into())]
        );
    }

    #[test]
    fn tokenize_descendant_or_self() {
        assert_eq!(
            tokenize("<< 404684003").unwrap(),
            vec![Token::LtLt, Token::ConceptId("404684003".into())]
        );
    }

    #[test]
    fn tokenize_ancestor_of() {
        assert_eq!(
            tokenize("> 404684003").unwrap(),
            vec![Token::Gt, Token::ConceptId("404684003".into())]
        );
    }

    #[test]
    fn tokenize_ancestor_or_self() {
        assert_eq!(
            tokenize(">> 404684003").unwrap(),
            vec![Token::GtGt, Token::ConceptId("404684003".into())]
        );
    }

    #[test]
    fn tokenize_and_keyword() {
        let toks = tokenize("1 AND 2").unwrap();
        assert_eq!(toks[1], Token::And);
    }

    #[test]
    fn tokenize_comma_as_and() {
        let toks = tokenize("1,2").unwrap();
        assert_eq!(toks[1], Token::And);
    }

    #[test]
    fn tokenize_or_keyword() {
        let toks = tokenize("1 OR 2").unwrap();
        assert_eq!(toks[1], Token::Or);
    }

    #[test]
    fn tokenize_minus_keyword() {
        let toks = tokenize("1 MINUS 2").unwrap();
        assert_eq!(toks[1], Token::Minus);
    }

    #[test]
    fn tokenize_pipe_labels_are_stripped() {
        let toks = tokenize("404684003 |Clinical finding|").unwrap();
        assert_eq!(toks, vec![Token::ConceptId("404684003".into())]);
    }

    #[test]
    fn tokenize_alphabetic_code_is_concept_id() {
        // Non-keyword alphabetic words are treated as concept codes, not errors.
        // This supports code systems that use human-readable codes (e.g. anatomy).
        let toks = tokenize("limb").unwrap();
        assert_eq!(toks, vec![Token::ConceptId("limb".into())]);
    }

    #[test]
    fn tokenize_alphanumeric_code_is_concept_id() {
        let toks = tokenize("A11Y-code").unwrap();
        assert_eq!(toks, vec![Token::ConceptId("A11Y-code".into())]);
    }

    #[test]
    fn tokenize_unknown_char_returns_error() {
        assert!(tokenize("@123").is_err());
    }

    // ── Parser ────────────────────────────────────────────────────────────────

    #[test]
    fn parse_exact_concept() {
        let expr = parse("404684003").unwrap();
        assert_eq!(
            expr,
            EclExpr::Focus {
                op: None,
                concept: FocusConcept::Id("404684003".into()),
            }
        );
    }

    #[test]
    fn parse_wildcard() {
        let expr = parse("*").unwrap();
        assert_eq!(
            expr,
            EclExpr::Focus {
                op: None,
                concept: FocusConcept::Wildcard,
            }
        );
    }

    #[test]
    fn parse_descendant_of() {
        let expr = parse("< 404684003").unwrap();
        assert_eq!(
            expr,
            EclExpr::Focus {
                op: Some(ConceptOperator::DescendantOf),
                concept: FocusConcept::Id("404684003".into()),
            }
        );
    }

    #[test]
    fn parse_descendant_or_self() {
        let expr = parse("<< 404684003").unwrap();
        assert_eq!(
            expr,
            EclExpr::Focus {
                op: Some(ConceptOperator::DescendantOrSelf),
                concept: FocusConcept::Id("404684003".into()),
            }
        );
    }

    #[test]
    fn parse_ancestor_of() {
        let expr = parse("> 100").unwrap();
        assert_eq!(
            expr,
            EclExpr::Focus {
                op: Some(ConceptOperator::AncestorOf),
                concept: FocusConcept::Id("100".into()),
            }
        );
    }

    #[test]
    fn parse_ancestor_or_self() {
        let expr = parse(">> 100").unwrap();
        assert_eq!(
            expr,
            EclExpr::Focus {
                op: Some(ConceptOperator::AncestorOrSelf),
                concept: FocusConcept::Id("100".into()),
            }
        );
    }

    #[test]
    fn parse_concept_with_pipe_label() {
        let expr = parse("<< 404684003 |Clinical finding|").unwrap();
        assert_eq!(
            expr,
            EclExpr::Focus {
                op: Some(ConceptOperator::DescendantOrSelf),
                concept: FocusConcept::Id("404684003".into()),
            }
        );
    }

    #[test]
    fn parse_and_conjunction() {
        let expr = parse("<< 100 AND < 200").unwrap();
        assert!(matches!(expr, EclExpr::And(_, _)));
        if let EclExpr::And(l, r) = expr {
            assert_eq!(
                *l,
                EclExpr::Focus {
                    op: Some(ConceptOperator::DescendantOrSelf),
                    concept: FocusConcept::Id("100".into()),
                }
            );
            assert_eq!(
                *r,
                EclExpr::Focus {
                    op: Some(ConceptOperator::DescendantOf),
                    concept: FocusConcept::Id("200".into()),
                }
            );
        }
    }

    #[test]
    fn parse_comma_as_and() {
        let expr = parse("<< 100 , << 200").unwrap();
        assert!(matches!(expr, EclExpr::And(_, _)));
    }

    #[test]
    fn parse_or_disjunction() {
        let expr = parse("<< 100 OR < 200").unwrap();
        assert!(matches!(expr, EclExpr::Or(_, _)));
    }

    #[test]
    fn parse_minus_exclusion() {
        let expr = parse("<< 100 MINUS << 200").unwrap();
        assert!(matches!(expr, EclExpr::Minus(_, _)));
    }

    #[test]
    fn parse_and_has_higher_precedence_than_or() {
        // "A OR B AND C" should parse as "A OR (B AND C)"
        let expr = parse("<< 1 OR << 2 AND << 3").unwrap();
        // Top level is OR
        match expr {
            EclExpr::Or(l, r) => {
                assert!(matches!(*l, EclExpr::Focus { .. }));
                // Right side is AND
                assert!(matches!(*r, EclExpr::And(_, _)));
            }
            other => panic!("expected Or, got {other:?}"),
        }
    }

    #[test]
    fn parse_grouped_expression() {
        let expr = parse("(<< 100 OR << 200) AND << 300").unwrap();
        // Top level is AND
        match expr {
            EclExpr::And(l, r) => {
                // Left is OR (grouped)
                assert!(matches!(*l, EclExpr::Or(_, _)));
                assert!(matches!(*r, EclExpr::Focus { .. }));
            }
            other => panic!("expected And, got {other:?}"),
        }
    }

    #[test]
    fn parse_triple_and() {
        // Left-associative: (A AND B) AND C
        let expr = parse("<< 1 AND << 2 AND << 3").unwrap();
        match expr {
            EclExpr::And(l, _r) => {
                assert!(matches!(*l, EclExpr::And(_, _)));
            }
            other => panic!("expected nested And, got {other:?}"),
        }
    }

    #[test]
    fn parse_extra_tokens_returns_error() {
        assert!(parse("<< 1 << 2").is_err());
    }

    #[test]
    fn parse_empty_input_returns_error() {
        assert!(parse("").is_err());
    }

    #[test]
    fn parse_unclosed_paren_returns_error() {
        assert!(parse("(<< 1").is_err());
    }

    #[test]
    fn parse_leading_trailing_whitespace() {
        let expr = parse("  << 404684003  ").unwrap();
        assert!(matches!(
            expr,
            EclExpr::Focus {
                op: Some(ConceptOperator::DescendantOrSelf),
                ..
            }
        ));
    }
}
