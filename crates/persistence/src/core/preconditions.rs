//! Shared parsing and evaluation of HTTP entity-tag preconditions.
//!
//! `If-Match` and `If-None-Match` are **lists**:
//!
//! ```text
//! If-Match      = "*" / #entity-tag
//! If-None-Match = "*" / #entity-tag
//! ```
//!
//! ([RFC 9110 §13.1.1], [§13.1.2]). The `#` list rule expands to
//! `element *( OWS "," OWS element )` ([§5.6.1.1]), and a sender may split one
//! list across repeated field lines, which a recipient must recombine in order
//! ([§5.3]). Every HFS call site used to compare the whole field value as one
//! opaque string, so any multi-valued header could never match — a permanent
//! `412` on `PUT`/`PATCH` (issue #311).
//!
//! This module is the single place that parses those headers and the single
//! place that compares tags, so the HTTP handlers in `helios-rest`, the bundle
//! executors in every storage backend, and `helios-hts` cannot drift apart.
//! It lives in `helios-persistence` rather than `helios-rest` because
//! `helios-persistence` is the lower crate: `helios-rest` and `helios-hts` both
//! depend on it, and neither may be depended upon by it.
//!
//! # Weak vs strong comparison: a deliberate FHIR deviation
//!
//! RFC 9110 requires `If-Match` to use **strong** comparison, under which a weak
//! tag never matches anything ([§8.8.3.2]). FHIR, however, mandates that servers
//! emit **weak** ETags (`ETag: W/"{versionId}"`) and that clients echo them back
//! in `If-Match`. Applying strong comparison literally would therefore make every
//! FHIR conditional update fail forever.
//!
//! So the comparison here is on the opaque tag value alone, ignoring the `W/`
//! weakness flag — i.e. weak comparison for both headers. This matches what the
//! MongoDB and S3 backends and [`normalize_etag`] already did in practice, and it
//! is the only reading under which FHIR's own rules are satisfiable. The one
//! RFC-visible consequence of ignoring weakness is that a route emitting the same
//! opaque value as *both* a weak and a strong validator would conflate them; no
//! HFS route does that (FHIR routes emit only `W/"n"`, `/_user/settings` emits
//! only `"n"`).
//!
//! # Failing closed
//!
//! A precondition that cannot be evaluated must never be treated as *absent*.
//! Degrading an unparseable `If-Match` to "no precondition" turns a guarded
//! update into an unconditional overwrite — a silent lost update. Callers get a
//! [`MalformedPrecondition`] error and are expected to turn it into a `412`,
//! matching the rationale already documented for `/_user/settings` (issue #270).
//!
//! [RFC 9110 §13.1.1]: https://www.rfc-editor.org/rfc/rfc9110#section-13.1.1
//! [§13.1.2]: https://www.rfc-editor.org/rfc/rfc9110#section-13.1.2
//! [§5.6.1.1]: https://www.rfc-editor.org/rfc/rfc9110#section-5.6.1.1
//! [§5.3]: https://www.rfc-editor.org/rfc/rfc9110#section-5.3
//! [§8.8.3.2]: https://www.rfc-editor.org/rfc/rfc9110#section-8.8.3.2

use std::fmt;

use super::transaction::BundleEntryResult;

/// A single parsed entity-tag.
///
/// `value` is the *unquoted* opaque tag: `W/"3"` and `"3"` both yield
/// `value == "3"`, differing only in [`EntityTag::weak`]. Comparison here never
/// consults `weak` (see the module docs), but it is retained so callers can
/// report exactly what the client sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityTag {
    /// Whether the tag carried the `W/` weakness prefix.
    pub weak: bool,
    /// The opaque tag value, without surrounding quotes.
    pub value: String,
}

impl EntityTag {
    /// Creates a strong tag with the given opaque value.
    ///
    /// Named `new_strong` rather than `strong` so it cannot be confused with the
    /// [`EntityTag::weak`] *field* at a call site.
    pub fn new_strong(value: impl Into<String>) -> Self {
        Self {
            weak: false,
            value: value.into(),
        }
    }

    /// Creates a weak tag with the given opaque value.
    pub fn new_weak(value: impl Into<String>) -> Self {
        Self {
            weak: true,
            value: value.into(),
        }
    }
}

impl fmt::Display for EntityTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.weak {
            write!(f, "W/\"{}\"", self.value)
        } else {
            write!(f, "\"{}\"", self.value)
        }
    }
}

/// A parsed `If-Match` / `If-None-Match` precondition.
///
/// The three states must not be collapsed onto `Option<String>`: conflating
/// "absent" with "present but unusable" is what allows a malformed header to
/// become an unconditional write.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum EntityTagPrecondition {
    /// The header was not sent at all.
    #[default]
    Absent,
    /// `*` — matches if and only if a current representation exists.
    Any,
    /// A non-empty list of candidate tags.
    Tags(Vec<EntityTag>),
}

/// A syntactically invalid precondition field value.
///
/// Callers must translate this into a failed precondition (`412`), never into
/// [`EntityTagPrecondition::Absent`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MalformedPrecondition {
    /// The offending raw field value, for diagnostics.
    pub raw: String,
    /// Why it could not be parsed.
    pub reason: &'static str,
}

impl fmt::Display for MalformedPrecondition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "malformed entity-tag precondition {:?}: {}",
            self.raw, self.reason
        )
    }
}

impl std::error::Error for MalformedPrecondition {}

impl EntityTagPrecondition {
    /// Parses one or more field lines of an `If-Match` / `If-None-Match` header.
    ///
    /// Pass every field line for the header in the order received; per
    /// [RFC 9110 §5.3] they form a single comma-separated list. Passing an empty
    /// iterator yields [`EntityTagPrecondition::Absent`].
    ///
    /// Empty list elements are tolerated and skipped ([§5.6.1.2] requires
    /// recipients to accept a reasonable number of them), but a field value that
    /// contains *only* separators is malformed rather than absent — the client
    /// sent a precondition and it cannot be satisfied.
    ///
    /// `*` is an alternative to the list, not a member of it ([§13.1.1]:
    /// `"*" / #entity-tag`, where `entity-tag` is always quoted), so `*` mixed
    /// with tags is rejected.
    ///
    /// [RFC 9110 §5.3]: https://www.rfc-editor.org/rfc/rfc9110#section-5.3
    /// [§5.6.1.2]: https://www.rfc-editor.org/rfc/rfc9110#section-5.6.1.2
    /// [§13.1.1]: https://www.rfc-editor.org/rfc/rfc9110#section-13.1.1
    pub fn parse<'a, I>(field_lines: I) -> Result<Self, MalformedPrecondition>
    where
        I: IntoIterator<Item = &'a str>,
    {
        let lines: Vec<&str> = field_lines.into_iter().collect();

        // No field line at all — the client sent no precondition.
        if lines.is_empty() {
            return Ok(Self::Absent);
        }

        let joined = lines.join(",");
        let trimmed = joined.trim();

        // A field line WAS sent but carries no value. That is not "absent": the
        // client asked for a precondition we cannot evaluate, so it must fail
        // closed rather than become an unconditional write.
        if trimmed.is_empty() {
            return Err(MalformedPrecondition {
                raw: joined.clone(),
                reason: "If-Match/If-None-Match was present but empty",
            });
        }

        if trimmed == "*" {
            return Ok(Self::Any);
        }

        let mut tags = Vec::new();
        for element in split_list(trimmed) {
            let element = element.trim();
            if element.is_empty() {
                // Tolerated empty list element (§5.6.1.2).
                continue;
            }
            tags.push(parse_entity_tag(element, trimmed)?);
        }

        if tags.is_empty() {
            return Err(MalformedPrecondition {
                raw: trimmed.to_string(),
                reason: "field value contained no entity-tags",
            });
        }

        Ok(Self::Tags(tags))
    }

    /// Parses raw header bytes, failing closed on a non-UTF-8 value.
    ///
    /// `etagc` admits `obs-text` (`%x80-FF`), so a non-UTF-8 field value is
    /// *well-formed* HTTP — it simply cannot equal any tag this server issues
    /// (all of which are ASCII decimal version ids). It must therefore evaluate
    /// to "no match", never to [`EntityTagPrecondition::Absent`]. The previous
    /// `to_str().ok()` handling silently dropped such a header, converting a
    /// guarded write into an unconditional one.
    pub fn parse_bytes<'a, I>(field_lines: I) -> Result<Self, MalformedPrecondition>
    where
        I: IntoIterator<Item = &'a [u8]>,
    {
        let mut decoded: Vec<String> = Vec::new();
        for line in field_lines {
            match std::str::from_utf8(line) {
                Ok(s) => decoded.push(s.to_string()),
                // Well-formed but unmatchable: substitute a tag that cannot
                // collide with any server-issued version id, so evaluation
                // yields `false` rather than "absent".
                Err(_) => {
                    return Ok(Self::Tags(vec![EntityTag {
                        weak: false,
                        value: NON_UTF8_SENTINEL.to_string(),
                    }]));
                }
            }
        }
        Self::parse(decoded.iter().map(String::as_str))
    }

    /// Evaluates an **`If-Match`** precondition.
    ///
    /// `current_version_id` is the resource's current `versionId`, or `None`
    /// when no current representation exists (absent, or deleted — a deleted
    /// resource has no current representation, so `*` does not match it).
    ///
    /// Per [RFC 9110 §13.1.1]: absent → true; `*` → true only if a current
    /// representation exists; a list → true if any member matches.
    ///
    /// [RFC 9110 §13.1.1]: https://www.rfc-editor.org/rfc/rfc9110#section-13.1.1
    pub fn if_match_satisfied(&self, current_version_id: Option<&str>) -> bool {
        match self {
            Self::Absent => true,
            Self::Any => current_version_id.is_some(),
            Self::Tags(tags) => match current_version_id {
                None => false,
                Some(current) => tags.iter().any(|t| tag_matches(t, current)),
            },
        }
    }

    /// Evaluates an **`If-None-Match`** precondition.
    ///
    /// The inverse of [`Self::if_match_satisfied`]: absent → true (proceed);
    /// `*` → true only if *no* current representation exists; a list → true only
    /// if no member matches ([RFC 9110 §13.1.2]).
    ///
    /// [RFC 9110 §13.1.2]: https://www.rfc-editor.org/rfc/rfc9110#section-13.1.2
    pub fn if_none_match_satisfied(&self, current_version_id: Option<&str>) -> bool {
        match self {
            Self::Absent => true,
            Self::Any => current_version_id.is_none(),
            Self::Tags(tags) => match current_version_id {
                None => true,
                Some(current) => !tags.iter().any(|t| tag_matches(t, current)),
            },
        }
    }

    /// Returns the tag that matched `current_version_id`, if any.
    ///
    /// Backends whose conditional write primitives take a *single* expected
    /// version (`update_with_match` / `delete_with_match`) use this to collapse
    /// the list down at the call site, which has already read the current
    /// resource. That keeps those trait signatures — implemented by four
    /// backends and the composite store — unchanged.
    pub fn matched_tag(&self, current_version_id: Option<&str>) -> Option<&EntityTag> {
        match self {
            Self::Tags(tags) => {
                let current = current_version_id?;
                tags.iter().find(|t| tag_matches(t, current))
            }
            _ => None,
        }
    }

    /// Whether a precondition was supplied at all.
    pub fn is_present(&self) -> bool {
        !matches!(self, Self::Absent)
    }
}

/// Opaque value substituted for a non-UTF-8 field value so that evaluation
/// yields "no match" instead of "absent". Contains characters that cannot occur
/// in a server-issued version id.
const NON_UTF8_SENTINEL: &str = "\u{0}non-utf8\u{0}";

/// Compares one parsed tag against a current version id.
///
/// Weakness is deliberately ignored — see the module docs. `current` is the bare
/// `versionId`, but it is normalized defensively so that a caller passing a full
/// `W/"3"` ETag still compares correctly.
fn tag_matches(tag: &EntityTag, current: &str) -> bool {
    tag.value == super::versioned::normalize_etag(current)
}

/// Splits a list field value on commas that are not inside a quoted string.
///
/// A comma inside `"..."` is part of the opaque tag, not a list separator.
fn split_list(value: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut in_quotes = false;
    for (idx, ch) in value.char_indices() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                parts.push(&value[start..idx]);
                start = idx + 1;
            }
            _ => {}
        }
    }
    parts.push(&value[start..]);
    parts
}

/// Parses a single `entity-tag = [ weak ] opaque-tag` element.
fn parse_entity_tag(element: &str, raw: &str) -> Result<EntityTag, MalformedPrecondition> {
    if element == "*" {
        // `*` is an alternative to the whole list, never a member of one.
        return Err(MalformedPrecondition {
            raw: raw.to_string(),
            reason: "'*' cannot appear alongside entity-tags",
        });
    }

    let (weak, quoted) = match element.strip_prefix("W/") {
        Some(rest) => (true, rest),
        None => (false, element),
    };

    let quoted = quoted.trim_start();
    if quoted.len() < 2 || !quoted.starts_with('"') || !quoted.ends_with('"') {
        return Err(MalformedPrecondition {
            raw: raw.to_string(),
            reason: "entity-tag must be enclosed in double quotes",
        });
    }

    let value = &quoted[1..quoted.len() - 1];
    if value.contains('"') {
        return Err(MalformedPrecondition {
            raw: raw.to_string(),
            reason: "entity-tag contains an unescaped double quote",
        });
    }

    Ok(EntityTag {
        weak,
        value: value.to_string(),
    })
}

/// Evaluates a bundle entry's `ifMatch` against the current version, returning a
/// ready-made `412` entry result when the precondition fails.
///
/// Every backend's bundle executor funnels through this so that batch and
/// transaction arms, and all four backends, agree by construction. Before this
/// existed the SQLite and PostgreSQL **batch** arms ignored `ifMatch` entirely
/// (a silent lost update), their transaction arms compared raw strings while
/// MongoDB and S3 compared normalized ones, and none of the four honored
/// `ifMatch` on `DELETE`.
///
/// `current_version_id` is `None` when the target does not exist (or is
/// deleted), in which case any supplied `ifMatch` — including `*` — fails.
///
/// Returns `None` to proceed, or `Some(result)` carrying the `412` to record for
/// this entry.
///
/// A malformed field value also yields `Some(412)` rather than a separate error
/// type: it is a failed precondition either way, and collapsing the two keeps
/// every backend call site to a single `if let`. The distinction survives in the
/// diagnostics string.
pub fn bundle_if_match_gate(
    raw_if_match: Option<&str>,
    current_version_id: Option<&str>,
) -> Option<BundleEntryResult> {
    let raw = raw_if_match?;

    let precondition = match EntityTagPrecondition::parse([raw]) {
        Ok(p) => p,
        Err(e) => {
            return Some(precondition_failed_entry(&format!(
                "If-Match precondition failed: {e}"
            )));
        }
    };

    if precondition.if_match_satisfied(current_version_id) {
        return None;
    }

    let diagnostics = match current_version_id {
        Some(current) => {
            format!("If-Match precondition failed: supplied {raw}, current version {current}")
        }
        None => format!(
            "If-Match precondition failed: supplied {raw}, but the resource has no current version"
        ),
    };

    Some(precondition_failed_entry(&diagnostics))
}

/// Whether a raw `If-Match` field value is satisfied by `current_version_id`.
///
/// This is the comparison used by the backends' conditional-write primitives
/// (`update_with_match` / `delete_with_match`), which receive the client's
/// `If-Match` field value verbatim. They previously compared it with
/// [`normalize_etag`](super::versioned::normalize_etag) as one opaque string, so
/// a comma-separated list could never match (issue #311).
///
/// # Leniency
///
/// Unlike [`EntityTagPrecondition::parse`], a bare unquoted token (`3`) is
/// accepted. `normalize_etag`'s documented contract at this layer is that an
/// ETag "may be formatted as `W/\"1\"`, `\"1\"`, or just `1`", and internal
/// callers pass bare version ids, so rejecting them would break callers that
/// have nothing to do with HTTP headers. Strict list parsing is tried first; the
/// bare form is only reached as a fallback.
pub fn if_match_field_satisfied(raw: &str, current_version_id: &str) -> bool {
    if let Ok(precondition) = EntityTagPrecondition::parse([raw]) {
        return precondition.if_match_satisfied(Some(current_version_id));
    }
    super::versioned::normalize_etag(raw) == super::versioned::normalize_etag(current_version_id)
}

/// Builds the `412` bundle entry result used by every backend.
pub fn precondition_failed_entry(diagnostics: &str) -> BundleEntryResult {
    BundleEntryResult::error(
        412,
        serde_json::json!({
            "resourceType": "OperationOutcome",
            "issue": [{
                "severity": "error",
                "code": "conflict",
                "diagnostics": diagnostics,
            }]
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(p: &EntityTagPrecondition) -> Vec<String> {
        match p {
            EntityTagPrecondition::Tags(t) => t.iter().map(|t| t.value.clone()).collect(),
            _ => panic!("expected Tags, got {p:?}"),
        }
    }

    // ── Parsing ──────────────────────────────────────────────────────────────

    #[test]
    fn absent_when_no_field_lines() {
        assert_eq!(
            EntityTagPrecondition::parse(std::iter::empty::<&str>()).unwrap(),
            EntityTagPrecondition::Absent
        );
    }

    /// A header that was sent but is empty is NOT the same as one that was never
    /// sent — treating it as absent would be an unconditional write.
    #[test]
    fn present_but_empty_is_malformed_not_absent() {
        assert!(EntityTagPrecondition::parse([""]).is_err());
        assert!(EntityTagPrecondition::parse(["   "]).is_err());
    }

    #[test]
    fn single_weak_tag() {
        let p = EntityTagPrecondition::parse(["W/\"3\""]).unwrap();
        assert_eq!(tags(&p), vec!["3"]);
        assert!(matches!(&p, EntityTagPrecondition::Tags(t) if t[0].weak));
    }

    #[test]
    fn single_strong_tag() {
        let p = EntityTagPrecondition::parse(["\"3\""]).unwrap();
        assert_eq!(tags(&p), vec!["3"]);
        assert!(matches!(&p, EntityTagPrecondition::Tags(t) if !t[0].weak));
    }

    /// The bug in issue #311: this is one legal field value, not an opaque string.
    #[test]
    fn multi_valued_list_is_parsed() {
        let p = EntityTagPrecondition::parse(["W/\"3\", W/\"4\""]).unwrap();
        assert_eq!(tags(&p), vec!["3", "4"]);
    }

    #[test]
    fn optional_whitespace_around_commas() {
        let p = EntityTagPrecondition::parse(["W/\"1\"   ,\tW/\"2\" ,W/\"3\""]).unwrap();
        assert_eq!(tags(&p), vec!["1", "2", "3"]);
    }

    /// RFC 9110 §5.3: repeated field lines are one list.
    #[test]
    fn repeated_field_lines_are_combined() {
        let p = EntityTagPrecondition::parse(["W/\"3\"", "W/\"4\""]).unwrap();
        assert_eq!(tags(&p), vec!["3", "4"]);
    }

    #[test]
    fn star_alone_is_any() {
        assert_eq!(
            EntityTagPrecondition::parse(["*"]).unwrap(),
            EntityTagPrecondition::Any
        );
        assert_eq!(
            EntityTagPrecondition::parse(["  *  "]).unwrap(),
            EntityTagPrecondition::Any
        );
    }

    /// `*` is an alternative to the list, not a member of it.
    #[test]
    fn star_mixed_with_tags_is_malformed() {
        assert!(EntityTagPrecondition::parse(["*, W/\"3\""]).is_err());
        assert!(EntityTagPrecondition::parse(["W/\"3\", *"]).is_err());
    }

    #[test]
    fn empty_list_elements_are_tolerated() {
        let p = EntityTagPrecondition::parse(["W/\"1\",, W/\"2\""]).unwrap();
        assert_eq!(tags(&p), vec!["1", "2"]);
    }

    #[test]
    fn only_separators_is_malformed_not_absent() {
        // The client did send a precondition; it just cannot be satisfied.
        assert!(EntityTagPrecondition::parse([",,,"]).is_err());
    }

    #[test]
    fn unquoted_tag_is_malformed() {
        assert!(EntityTagPrecondition::parse(["3"]).is_err());
        assert!(EntityTagPrecondition::parse(["W/3"]).is_err());
    }

    #[test]
    fn unterminated_quote_is_malformed() {
        assert!(EntityTagPrecondition::parse(["\"3"]).is_err());
    }

    #[test]
    fn empty_opaque_tag_is_valid() {
        let p = EntityTagPrecondition::parse(["W/\"\""]).unwrap();
        assert_eq!(tags(&p), vec![""]);
    }

    #[test]
    fn comma_inside_quotes_is_not_a_separator() {
        let p = EntityTagPrecondition::parse(["\"a,b\""]).unwrap();
        assert_eq!(tags(&p), vec!["a,b"]);
    }

    #[test]
    fn long_list_is_parsed() {
        let raw = (1..=50)
            .map(|n| format!("W/\"{n}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let p = EntityTagPrecondition::parse([raw.as_str()]).unwrap();
        assert_eq!(tags(&p).len(), 50);
    }

    /// Non-UTF-8 is well-formed HTTP (`obs-text`) but unmatchable — it must
    /// evaluate to "no match", NOT to `Absent` (which would be an
    /// unconditional write).
    #[test]
    fn non_utf8_fails_closed() {
        let p = EntityTagPrecondition::parse_bytes([&[0xFF, 0xFE][..]]).unwrap();
        assert!(p.is_present(), "must not be treated as absent");
        assert!(!p.if_match_satisfied(Some("3")));
        assert!(!p.if_match_satisfied(None));
    }

    // ── If-Match evaluation ──────────────────────────────────────────────────

    #[test]
    fn if_match_absent_always_proceeds() {
        let p = EntityTagPrecondition::Absent;
        assert!(p.if_match_satisfied(Some("3")));
        assert!(p.if_match_satisfied(None));
    }

    #[test]
    fn if_match_any_requires_current_representation() {
        let p = EntityTagPrecondition::Any;
        assert!(p.if_match_satisfied(Some("3")));
        // `*` on an absent or deleted resource is a FAILED precondition; it must
        // not license an update-as-create.
        assert!(!p.if_match_satisfied(None));
    }

    #[test]
    fn if_match_matches_any_member() {
        let p = EntityTagPrecondition::parse(["W/\"3\", W/\"4\""]).unwrap();
        assert!(p.if_match_satisfied(Some("3")));
        assert!(p.if_match_satisfied(Some("4")));
        assert!(!p.if_match_satisfied(Some("5")));
    }

    /// FHIR emits `W/"n"`; a client echoing the strong form must still match.
    #[test]
    fn weak_and_strong_forms_are_interchangeable() {
        assert!(
            EntityTagPrecondition::parse(["\"3\""])
                .unwrap()
                .if_match_satisfied(Some("3"))
        );
        assert!(
            EntityTagPrecondition::parse(["W/\"3\""])
                .unwrap()
                .if_match_satisfied(Some("3"))
        );
    }

    #[test]
    fn if_match_tag_against_absent_resource_fails() {
        let p = EntityTagPrecondition::parse(["W/\"3\""]).unwrap();
        assert!(!p.if_match_satisfied(None));
    }

    #[test]
    fn current_version_given_as_full_etag_still_matches() {
        let p = EntityTagPrecondition::parse(["W/\"3\""]).unwrap();
        assert!(p.if_match_satisfied(Some("W/\"3\"")));
    }

    // ── If-None-Match evaluation ─────────────────────────────────────────────

    #[test]
    fn if_none_match_is_the_inverse() {
        let p = EntityTagPrecondition::parse(["W/\"3\", W/\"4\""]).unwrap();
        assert!(!p.if_none_match_satisfied(Some("3")));
        assert!(p.if_none_match_satisfied(Some("9")));
        assert!(p.if_none_match_satisfied(None));

        let any = EntityTagPrecondition::Any;
        assert!(!any.if_none_match_satisfied(Some("3")));
        assert!(any.if_none_match_satisfied(None));
    }

    // ── matched_tag ──────────────────────────────────────────────────────────

    #[test]
    fn matched_tag_collapses_list_for_single_tag_backends() {
        let p = EntityTagPrecondition::parse(["W/\"3\", W/\"4\""]).unwrap();
        assert_eq!(p.matched_tag(Some("4")).unwrap().value, "4");
        assert!(p.matched_tag(Some("9")).is_none());
        assert!(EntityTagPrecondition::Any.matched_tag(Some("4")).is_none());
    }

    // ── bundle gate ──────────────────────────────────────────────────────────

    #[test]
    fn gate_proceeds_when_absent_or_matching() {
        assert!(bundle_if_match_gate(None, Some("3")).is_none());
        assert!(bundle_if_match_gate(Some("W/\"3\""), Some("3")).is_none());
        // Multi-valued: the regression from #311.
        assert!(bundle_if_match_gate(Some("W/\"3\", W/\"4\""), Some("4")).is_none());
        // A client echoing the strong form must still match a weak ETag.
        assert!(bundle_if_match_gate(Some("\"3\""), Some("3")).is_none());
    }

    #[test]
    fn gate_returns_412_on_mismatch_and_on_absent_resource() {
        assert_eq!(
            bundle_if_match_gate(Some("W/\"3\""), Some("9"))
                .expect("mismatch must fail")
                .status,
            412
        );
        assert_eq!(
            bundle_if_match_gate(Some("W/\"3\""), None)
                .expect("absent resource must fail")
                .status,
            412
        );
        // `*` asserts a current representation exists.
        assert_eq!(
            bundle_if_match_gate(Some("*"), None)
                .expect("star on absent must fail")
                .status,
            412
        );
        assert!(bundle_if_match_gate(Some("*"), Some("3")).is_none());
    }

    #[test]
    fn gate_fails_closed_on_malformed() {
        assert_eq!(
            bundle_if_match_gate(Some("garbage"), Some("3"))
                .expect("malformed must fail closed")
                .status,
            412
        );
    }

    // ── Entity-tag construction and rendering ────────────────────────────────

    /// `Display` is the wire format — it is what an `ETag` response header and a
    /// generated `If-Match` are rendered from — so the `W/` prefix and the
    /// quoting have to be exact, not merely round-trippable.
    #[test]
    fn entity_tag_renders_the_wire_format() {
        assert_eq!(EntityTag::new_strong("3").to_string(), "\"3\"");
        assert_eq!(EntityTag::new_weak("3").to_string(), "W/\"3\"");
    }

    /// Rendering and parsing must be inverses. A drift here (a missing `W/`, an
    /// extra layer of quotes) would not fail any single-sided test but would make
    /// every server-generated tag unmatchable by the client that echoes it back.
    #[test]
    fn entity_tag_display_round_trips_through_parse() {
        for tag in [EntityTag::new_strong("3"), EntityTag::new_weak("42")] {
            let rendered = tag.to_string();
            assert_eq!(
                EntityTagPrecondition::parse([rendered.as_str()]).unwrap(),
                EntityTagPrecondition::Tags(vec![tag.clone()]),
                "{rendered} must parse back to the tag it was rendered from"
            );
        }
    }

    /// The two constructors differ only in weakness, and weakness is part of
    /// identity — `new_strong` and `new_weak` must not be interchangeable.
    #[test]
    fn new_strong_and_new_weak_differ_only_in_weakness() {
        let strong = EntityTag::new_strong("7");
        let weak = EntityTag::new_weak("7");

        assert!(!strong.weak);
        assert!(weak.weak);
        assert_eq!(strong.value, weak.value);
        assert_ne!(strong, weak);
    }

    /// An unescaped `"` inside the opaque tag makes the element unparseable.
    ///
    /// It must be reported as malformed — and so fail closed — rather than being
    /// silently truncated at the stray quote, which would let `"a"b"` match the
    /// stored version `a`.
    #[test]
    fn unescaped_quote_inside_tag_is_malformed() {
        let err = EntityTagPrecondition::parse([r#""a"b""#]).unwrap_err();
        assert_eq!(err.reason, "entity-tag contains an unescaped double quote");
        assert_eq!(err.raw, r#""a"b""#);
    }
}
