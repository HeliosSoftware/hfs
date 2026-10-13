//! VCL (Value Set Composition Language) support.
//!
//! VCL is a compact text syntax for a `ValueSet.compose`, specified at
//! <https://build.fhir.org/ig/FHIR/ig-guidance/vcl.html> (a CI build, v0.1.0,
//! still changing). A client hands a VCL expression to a terminology server
//! as an implicit ValueSet URL:
//!
//! ```text
//! http://fhir.org/VCL?v1=<percent-encoded expression>
//! ```
//!
//! HTS treats VCL as a front-end: the expression is parsed into an AST
//! ([`parser`]), translated to a synthetic `ValueSet.compose`, and evaluated by
//! the existing inline-compose path on every backend.
//!
//! The parser is dialect-independent (no storage access, no backend `cfg`),
//! like the ECL parser.
//!
//! ## Support
//!
//! | VCL | `compose` filter / element | Status |
//! |-----|----------------------------|--------|
//! | `(sys)code`, `(sys\|ver)code`, `(sys)(a;b)` | `include.system`, `.version`, `.concept[]` | supported |
//! | `A;B` | separate `include` entries | supported |
//! | `A,B` over filters | several `filter` in one `include` | supported |
//! | `A-B` | `exclude` | supported |
//! | `(sys)*` | `include` with only `system` | supported |
//! | `^(vs)` / `^vs` | `include.valueSet` | supported |
//! | `p=v`, `<<`, `<`, `<!`, `/"re"` | `=`, `is-a`, `descendent-of`, `child-of`, `regex` | supported |
//! | `~<<`, `~^`, `^{a,b}`, `>>` | `is-not-a`, `not-in`, `in`, `generalizes` | needs backend support |
//! | `!!<`, `?`, `of` (`{x}.p`), nested filter lists, `p^URI` | — | parsed, rejected as not supported |
//!
//! ## Deviations from the published grammar
//!
//! * A hierarchical operator with no property (`<<404684003`) is accepted and
//!   means `concept<<404684003`.
//! * `prop^{A}` with a single code is accepted.
//! * Mixing `,` `;` `-` at one level without parentheses is rejected, also
//!   inside parentheses (the reference parsers accept it there).

pub mod parser;
pub mod translate;

use serde_json::{Value, json};

use crate::error::HtsError;
use crate::operations::params::find_str_param;

pub use parser::{
    Expr, ExprKind, Filter, FilterOp, FilterValue, MAX_EXPRESSION_LEN, MAX_NESTING_DEPTH, UriRef,
    VclError, parse,
};
pub use translate::{MAX_REF_DEPTH, to_value_set};

/// Base of a VCL implicit value set URL; the expression follows as `?v1=…`.
pub const VCL_URL_PREFIX: &str = "http://fhir.org/VCL";

/// The part of `url` after the VCL base URL (`?v1=…`), if it is one.
fn vcl_query(url: &str) -> Option<&str> {
    let rest = url
        .strip_prefix(VCL_URL_PREFIX)
        .or_else(|| url.strip_prefix("https://fhir.org/VCL"))?;
    rest.strip_prefix('?')
}

/// Whether `url` is a VCL implicit value set URL (`http://fhir.org/VCL?…`).
pub fn is_vcl_url(url: &str) -> bool {
    vcl_query(url).is_some()
}

/// Percent-decodes `s` exactly once. `+` is kept literally (VCL expressions
/// use it in codes and versions), and a `%` that is not followed by two hex
/// digits is kept as is, so an expression that was not encoded still works.
pub fn percent_decode(s: &str) -> Result<String, HtsError> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push(h << 4 | l);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| {
        HtsError::VsInvalid("Invalid VCL expression: the percent-decoded text is not UTF-8".into())
    })
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Extracts and percent-decodes the expression of a VCL URL. Everything after
/// `v1=` is the expression: an expression may itself contain `&` and `=`.
pub fn expression_from_url(url: &str) -> Result<String, HtsError> {
    let Some(raw) = vcl_query(url).and_then(|q| q.strip_prefix("v1=")) else {
        return Err(HtsError::VsInvalid(format!(
            "Invalid VCL URL: expected {VCL_URL_PREFIX}?v1=<percent-encoded expression>"
        )));
    };
    percent_decode(raw)
}

/// Parses and translates the VCL expression in `url` into a synthetic
/// `ValueSet` whose `url` is the VCL URL itself.
///
/// Errors: [`HtsError::VsInvalid`] (400) for a malformed or incomplete
/// expression, with the 1-based column in the decoded expression;
/// [`HtsError::NotSupported`] (501) for constructs HTS cannot evaluate.
pub fn value_set_from_vcl_url(url: &str) -> Result<Value, HtsError> {
    let expression = expression_from_url(url)?;
    let expr = parse(&expression).map_err(|e| {
        HtsError::VsInvalid(format!(
            "Invalid VCL expression at position {}: {}",
            e.pos, e.message
        ))
    })?;
    to_value_set(&expr, url)
}

/// Rewrites a `Parameters.parameter` list whose `url` is a VCL URL: every
/// `url` and `valueSet` parameter is dropped and the synthetic ValueSet is
/// appended as the `valueSet` resource parameter, so the request takes the
/// inline-compose path. Parameter lists without a VCL `url` are returned
/// unchanged.
pub fn rewrite_vcl_params(params: Vec<Value>) -> Result<Vec<Value>, HtsError> {
    let Some(url) = find_str_param(&params, "url").filter(|u| is_vcl_url(u)) else {
        return Ok(params);
    };
    let value_set = value_set_from_vcl_url(&url)?;
    let mut out: Vec<Value> = params
        .into_iter()
        .filter(|p| {
            !matches!(
                p.get("name").and_then(Value::as_str),
                Some("url" | "valueSet")
            )
        })
        .collect();
    out.push(json!({ "name": "valueSet", "resource": value_set }));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_vcl_urls() {
        assert!(is_vcl_url("http://fhir.org/VCL?v1=A"));
        assert!(is_vcl_url("https://fhir.org/VCL?v1=A"));
        assert!(!is_vcl_url("http://fhir.org/VCL"));
        assert!(!is_vcl_url("http://fhir.org/VCLx?v1=A"));
        assert!(!is_vcl_url("http://hl7.org/fhir/ValueSet/x"));
    }

    #[test]
    fn percent_decode_decodes_once() {
        assert_eq!(percent_decode("%28a%29").unwrap(), "(a)");
        assert_eq!(percent_decode("%2528").unwrap(), "%28");
        assert_eq!(percent_decode("a+b").unwrap(), "a+b");
        assert_eq!(percent_decode("100%").unwrap(), "100%");
        assert_eq!(percent_decode("%zz%4").unwrap(), "%zz%4");
        assert_eq!(percent_decode("%C3%A9").unwrap(), "é");
        assert!(percent_decode("%FF").is_err());
    }

    #[test]
    fn expression_keeps_ampersands_and_equals() {
        let e = expression_from_url("http://fhir.org/VCL?v1=(http://s)a=1&b").unwrap();
        assert_eq!(e, "(http://s)a=1&b");
        assert!(expression_from_url("http://fhir.org/VCL?x=1").is_err());
    }

    #[test]
    fn encoded_and_plain_urls_give_the_same_value_set() {
        let plain = value_set_from_vcl_url("http://fhir.org/VCL?v1=(http://s)(A;B)").unwrap();
        let encoded =
            value_set_from_vcl_url("http://fhir.org/VCL?v1=%28http%3A%2F%2Fs%29%28A%3BB%29")
                .unwrap();
        assert_eq!(plain["compose"], encoded["compose"]);
        assert_eq!(
            plain["compose"]["include"][0]["concept"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn malformed_expression_is_400_with_position() {
        match value_set_from_vcl_url("http://fhir.org/VCL?v1=(http://s)A%20B") {
            Err(HtsError::VsInvalid(m)) => {
                assert!(
                    m.starts_with("Invalid VCL expression at position 13:"),
                    "{m}"
                )
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(matches!(
            value_set_from_vcl_url("http://fhir.org/VCL?v1="),
            Err(HtsError::VsInvalid(_))
        ));
    }

    #[test]
    fn rewrite_replaces_url_and_value_set_params() {
        let params = vec![
            json!({ "name": "url", "valueUri": "http://fhir.org/VCL?v1=(http://s)A" }),
            json!({ "name": "valueSet", "resource": { "resourceType": "ValueSet" } }),
            json!({ "name": "count", "valueInteger": 5 }),
        ];
        let out = rewrite_vcl_params(params).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["name"], "count");
        assert_eq!(out[1]["name"], "valueSet");
        assert_eq!(
            out[1]["resource"]["url"],
            "http://fhir.org/VCL?v1=(http://s)A"
        );
        assert!(find_str_param(&out, "url").is_none());
    }

    #[test]
    fn rewrite_leaves_other_requests_alone() {
        let params = vec![json!({ "name": "url", "valueUri": "http://hl7.org/fhir/ValueSet/x" })];
        assert_eq!(rewrite_vcl_params(params.clone()).unwrap(), params);
        assert_eq!(rewrite_vcl_params(vec![]).unwrap(), Vec::<Value>::new());
    }

    #[test]
    fn rewrite_propagates_translation_errors() {
        let bad = vec![json!({ "name": "url", "valueString": "http://fhir.org/VCL?v1=A" })];
        assert!(matches!(
            rewrite_vcl_params(bad),
            Err(HtsError::VsInvalid(_))
        ));
    }
}
