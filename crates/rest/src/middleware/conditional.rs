//! Conditional request header handling.
//!
//! Handles HTTP conditional headers for FHIR requests:
//! - If-Match: Optimistic locking for updates
//! - If-None-Match: Conditional read
//! - If-Modified-Since: Conditional read by date
//! - If-None-Exist: Conditional create

use axum::{
    extract::FromRequestParts,
    http::{HeaderMap, HeaderName, StatusCode, header, request::Parts},
};
use chrono::{DateTime, Utc};
use helios_persistence::core::{EntityTagPrecondition, MalformedPrecondition};

/// Extracted conditional headers from a request.
///
/// `Default` is implemented by hand rather than derived because the parsed
/// precondition fields are `Result`s, which have no `Default`; an absent header
/// must default to `Ok(Absent)`, not to an error.
#[derive(Debug)]
pub struct ConditionalHeaders {
    /// Raw `If-Match` field value (all field lines joined), kept for logging and
    /// for the deprecated [`ConditionalHeaders::if_match`] accessor.
    if_match: Option<String>,

    /// Parsed `If-Match` precondition, or the parse error.
    ///
    /// `None` means the header was absent. This is stored eagerly so the parse
    /// happens once per request, and so a malformed value survives as an error
    /// rather than collapsing to "absent".
    if_match_parsed: Result<EntityTagPrecondition, MalformedPrecondition>,

    /// Raw `If-None-Match` field value (all field lines joined).
    if_none_match: Option<String>,

    /// Parsed `If-None-Match` precondition, or the parse error.
    if_none_match_parsed: Result<EntityTagPrecondition, MalformedPrecondition>,

    /// If-Modified-Since header value.
    if_modified_since: Option<DateTime<Utc>>,

    /// If-None-Exist header value (for conditional create).
    if_none_exist: Option<String>,
}

impl Default for ConditionalHeaders {
    fn default() -> Self {
        Self {
            if_match: None,
            if_match_parsed: Ok(EntityTagPrecondition::Absent),
            if_none_match: None,
            if_none_match_parsed: Ok(EntityTagPrecondition::Absent),
            if_modified_since: None,
            if_none_exist: None,
        }
    }
}

/// Raw + parsed form of one entity-tag precondition header.
struct EntityTagFields {
    raw: Option<String>,
    parsed: Result<EntityTagPrecondition, MalformedPrecondition>,
}

/// Reads every field line of `name` and parses them as one entity-tag list.
///
/// Uses [`HeaderMap::get_all`] because a sender may split a single
/// comma-separated list across repeated field lines, which a recipient must
/// recombine in order (RFC 9110 §5.3). The previous `get()` returned only the
/// first line and silently discarded the rest.
///
/// Bytes are handed to [`EntityTagPrecondition::parse_bytes`] rather than
/// `to_str().ok()` so a non-UTF-8 value fails closed instead of being dropped —
/// dropping it turned a guarded write into an unconditional one.
fn read_entity_tag_header(headers: &HeaderMap, name: HeaderName) -> EntityTagFields {
    let lines: Vec<&[u8]> = headers
        .get_all(&name)
        .iter()
        .map(|v| v.as_bytes())
        .collect();

    if lines.is_empty() {
        return EntityTagFields {
            raw: None,
            parsed: Ok(EntityTagPrecondition::Absent),
        };
    }

    let raw = lines
        .iter()
        .map(|b| String::from_utf8_lossy(b).into_owned())
        .collect::<Vec<_>>()
        .join(", ");

    EntityTagFields {
        raw: Some(raw),
        parsed: EntityTagPrecondition::parse_bytes(lines),
    }
}

impl ConditionalHeaders {
    /// Creates a new ConditionalHeaders from a HeaderMap.
    pub fn from_headers(headers: &HeaderMap) -> Self {
        let if_match = read_entity_tag_header(headers, header::IF_MATCH);
        let if_none_match = read_entity_tag_header(headers, header::IF_NONE_MATCH);

        let if_modified_since = headers
            .get(header::IF_MODIFIED_SINCE)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| DateTime::parse_from_rfc2822(s).ok())
            .map(|dt| dt.with_timezone(&Utc));

        // If-None-Exist is a custom FHIR header
        let if_none_exist = headers
            .get("if-none-exist")
            .and_then(|v| v.to_str().ok())
            .map(String::from);

        Self {
            if_match: if_match.raw,
            if_match_parsed: if_match.parsed,
            if_none_match: if_none_match.raw,
            if_none_match_parsed: if_none_match.parsed,
            if_modified_since,
            if_none_exist,
        }
    }

    /// Returns the raw `If-Match` field value.
    ///
    /// # Deprecated
    ///
    /// `If-Match` is a comma-separated list (RFC 9110 §13.1.1), so comparing
    /// this string as a whole can never match a multi-valued header — a
    /// permanent `412` on update/patch (issue #311). Use
    /// [`ConditionalHeaders::if_match_tags`] instead.
    ///
    /// Retained (rather than removed) because `helios-rest` is published, so
    /// removing it would be a breaking change; it also stays legitimately useful
    /// for logging the value exactly as the client sent it.
    #[deprecated(
        since = "0.2.2",
        note = "compares the whole field value; use `if_match_tags()` for RFC 9110 list semantics"
    )]
    pub fn if_match(&self) -> Option<&str> {
        self.if_match.as_deref()
    }

    /// Returns the raw `If-None-Match` field value.
    ///
    /// # Deprecated
    ///
    /// See [`ConditionalHeaders::if_match`]; use
    /// [`ConditionalHeaders::if_none_match_tags`] instead.
    #[deprecated(
        since = "0.2.2",
        note = "compares the whole field value; use `if_none_match_tags()` for RFC 9110 list semantics"
    )]
    pub fn if_none_match(&self) -> Option<&str> {
        self.if_none_match.as_deref()
    }

    /// Returns the parsed `If-Match` precondition.
    ///
    /// `Err` means the client sent a field value that cannot be parsed. It must
    /// be treated as a *failed* precondition, never as an absent one — see
    /// [`helios_persistence::core::preconditions`].
    pub fn if_match_tags(&self) -> Result<&EntityTagPrecondition, &MalformedPrecondition> {
        self.if_match_parsed.as_ref()
    }

    /// Returns the parsed `If-None-Match` precondition.
    pub fn if_none_match_tags(&self) -> Result<&EntityTagPrecondition, &MalformedPrecondition> {
        self.if_none_match_parsed.as_ref()
    }

    /// Whether an `If-Match` field was supplied at all.
    ///
    /// A malformed value counts as supplied: the client did ask for a
    /// precondition, so `HFS_REQUIRE_IF_MATCH` is satisfied and the value is
    /// then rejected on its own merits rather than reported as missing.
    pub fn has_if_match(&self) -> bool {
        match &self.if_match_parsed {
            Ok(p) => p.is_present(),
            Err(_) => true,
        }
    }

    /// Returns the If-Modified-Since header value.
    ///
    /// Used for conditional read - return 304 Not Modified if the
    /// resource has not been modified since this date.
    pub fn if_modified_since(&self) -> Option<DateTime<Utc>> {
        self.if_modified_since
    }

    /// Returns the If-None-Exist header value.
    ///
    /// Used for conditional create - only create the resource if
    /// no resource matches these search parameters.
    pub fn if_none_exist(&self) -> Option<&str> {
        self.if_none_exist.as_deref()
    }

    /// Checks if any conditional headers are present.
    pub fn has_conditions(&self) -> bool {
        self.if_match.is_some()
            || self.if_none_match.is_some()
            || self.if_modified_since.is_some()
            || self.if_none_exist.is_some()
    }
}

/// Axum extractor for conditional headers.
impl<S> FromRequestParts<S> for ConditionalHeaders
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(ConditionalHeaders::from_headers(&parts.headers))
    }
}

#[cfg(test)]
#[allow(deprecated)] // exercises the retained raw accessors on purpose
mod tests {
    use super::*;
    use axum::http::{HeaderName, HeaderValue};

    #[test]
    fn test_from_headers_if_match() {
        let mut headers = HeaderMap::new();
        headers.insert(header::IF_MATCH, HeaderValue::from_static("W/\"1\""));

        let conditional = ConditionalHeaders::from_headers(&headers);
        assert_eq!(conditional.if_match(), Some("W/\"1\""));
        assert!(
            conditional
                .if_match_tags()
                .unwrap()
                .if_match_satisfied(Some("1"))
        );
    }

    #[test]
    fn test_from_headers_if_none_match() {
        let mut headers = HeaderMap::new();
        headers.insert(header::IF_NONE_MATCH, HeaderValue::from_static("W/\"2\""));

        let conditional = ConditionalHeaders::from_headers(&headers);
        assert_eq!(conditional.if_none_match(), Some("W/\"2\""));
        assert!(
            !conditional
                .if_none_match_tags()
                .unwrap()
                .if_none_match_satisfied(Some("2"))
        );
    }

    /// Issue #311: a multi-valued `If-Match` must match on ANY listed tag.
    /// Before the fix the whole field value was compared as one string, so this
    /// could never match and produced a permanent 412.
    #[test]
    fn multi_valued_if_match_matches_any_member() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::IF_MATCH,
            HeaderValue::from_static("W/\"3\", W/\"4\""),
        );

        let conditional = ConditionalHeaders::from_headers(&headers);
        let tags = conditional.if_match_tags().unwrap();
        assert!(tags.if_match_satisfied(Some("3")));
        assert!(tags.if_match_satisfied(Some("4")));
        assert!(!tags.if_match_satisfied(Some("5")));
    }

    /// RFC 9110 §5.3: repeated field lines form one list. `HeaderMap::get`
    /// returned only the first, silently dropping the rest.
    #[test]
    fn repeated_if_match_field_lines_are_all_seen() {
        let mut headers = HeaderMap::new();
        headers.append(header::IF_MATCH, HeaderValue::from_static("W/\"3\""));
        headers.append(header::IF_MATCH, HeaderValue::from_static("W/\"4\""));

        let tags_owner = ConditionalHeaders::from_headers(&headers);
        let tags = tags_owner.if_match_tags().unwrap();
        assert!(tags.if_match_satisfied(Some("3")));
        assert!(
            tags.if_match_satisfied(Some("4")),
            "second field line must not be discarded"
        );
    }

    /// A non-UTF-8 value is well-formed HTTP (`obs-text`) but unmatchable. It
    /// must fail closed; previously `to_str().ok()` dropped it to `None`, which
    /// every caller read as "no precondition" — an unconditional overwrite.
    #[test]
    fn non_utf8_if_match_fails_closed() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::IF_MATCH,
            HeaderValue::from_bytes(&[0xFF, 0xFE]).unwrap(),
        );

        let conditional = ConditionalHeaders::from_headers(&headers);
        let tags = conditional.if_match_tags().unwrap();
        assert!(tags.is_present(), "must not be treated as absent");
        assert!(!tags.if_match_satisfied(Some("1")));
        assert!(conditional.has_conditions());
    }

    /// A malformed value must surface as an error the caller turns into a 412 —
    /// never as `Absent`.
    #[test]
    fn malformed_if_match_is_an_error_not_absent() {
        let mut headers = HeaderMap::new();
        headers.insert(header::IF_MATCH, HeaderValue::from_static("3"));

        let conditional = ConditionalHeaders::from_headers(&headers);
        assert!(conditional.if_match_tags().is_err());
    }

    #[test]
    fn absent_if_match_parses_to_absent() {
        let conditional = ConditionalHeaders::from_headers(&HeaderMap::new());
        assert_eq!(
            conditional.if_match_tags().unwrap(),
            &EntityTagPrecondition::Absent
        );
    }

    #[test]
    fn star_if_match_is_any() {
        let mut headers = HeaderMap::new();
        headers.insert(header::IF_MATCH, HeaderValue::from_static("*"));

        let conditional = ConditionalHeaders::from_headers(&headers);
        let tags = conditional.if_match_tags().unwrap();
        assert_eq!(tags, &EntityTagPrecondition::Any);
        assert!(tags.if_match_satisfied(Some("7")));
        // `*` asserts a current representation EXISTS.
        assert!(!tags.if_match_satisfied(None));
    }

    #[test]
    fn test_from_headers_if_none_exist() {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("if-none-exist"),
            HeaderValue::from_static("identifier=12345"),
        );

        let conditional = ConditionalHeaders::from_headers(&headers);
        assert_eq!(conditional.if_none_exist(), Some("identifier=12345"));
    }

    #[test]
    fn test_has_conditions() {
        let empty = ConditionalHeaders::default();
        assert!(!empty.has_conditions());

        let mut headers = HeaderMap::new();
        headers.insert(header::IF_MATCH, HeaderValue::from_static("W/\"1\""));
        let with_conditions = ConditionalHeaders::from_headers(&headers);
        assert!(with_conditions.has_conditions());
    }
}
