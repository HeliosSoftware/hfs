//! Same-origin navigation between editing surfaces (#1723).
//!
//! An explicit return destination stays optional: callers compute their own
//! fallback from the current saved/new state rather than posting a fallback
//! back as though the user had explicitly opened the editor from there.

use reqwest::Url;

const UI_ORIGIN: &str = "https://hfs.invalid";

/// Validates and normalizes a relative `/ui` destination, retaining its query
/// and fragment. Absolute/network URLs, ambiguous escaped path separators,
/// and traversal that leaves `/ui` are rejected rather than used as redirects.
pub(crate) fn safe_ui_return(requested: Option<&str>) -> Option<String> {
    let requested = requested?;
    if !requested.starts_with('/')
        || requested.starts_with("//")
        || requested.chars().any(|c| c.is_control() || c == '\\')
    {
        return None;
    }
    let path = requested.split(['?', '#']).next()?;
    if path != "/ui" && !path.starts_with("/ui/") {
        return None;
    }

    // URL parsing normalizes encoded dots too. Check the unnormalized path
    // first so `/ui/../ui` cannot leave the allowed tree and then re-enter it.
    let mut depth = 0usize;
    for segment in path[1..].split('/') {
        let decoded = decode_segment(segment)?;
        match decoded.as_str() {
            "" | "." => {}
            ".." => {
                if depth <= 1 {
                    return None;
                }
                depth -= 1;
            }
            _ => depth += 1,
        }
    }
    let base = Url::parse(UI_ORIGIN).ok()?;
    let url = base.join(requested).ok()?;
    if url.origin() != base.origin() || (url.path() != "/ui" && !url.path().starts_with("/ui/")) {
        return None;
    }
    Some(relative_url(&url))
}

/// Adds a validated explicit origin to an internal destination. Existing
/// `return_to` values are replaced, and query/fragment values are URL encoded.
pub(crate) fn with_return(destination: &str, return_to: Option<&str>) -> String {
    let destination = safe_ui_return(Some(destination)).unwrap_or_else(|| "/ui".to_string());
    let Some(return_to) = safe_ui_return(return_to) else {
        return destination;
    };
    let base = Url::parse(UI_ORIGIN).expect("constant UI origin is valid");
    let mut url = base.join(&destination).expect("validated UI destination");
    let mut pairs: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(key, _)| key != "return_to")
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    pairs.push(("return_to".to_string(), return_to));
    url.set_query(Some(
        &form_urlencoded::Serializer::new(String::new())
            .extend_pairs(pairs)
            .finish(),
    ));
    relative_url(&url)
}

fn relative_url(url: &Url) -> String {
    let mut target = url.path().to_string();
    if let Some(query) = url.query() {
        target.push('?');
        target.push_str(query);
    }
    if let Some(fragment) = url.fragment() {
        target.push('#');
        target.push_str(fragment);
    }
    target
}

fn decode_segment(segment: &str) -> Option<String> {
    let bytes = segment.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let byte = if bytes[index] == b'%' {
            let hex = std::str::from_utf8(bytes.get(index + 1..index + 3)?).ok()?;
            index += 3;
            u8::from_str_radix(hex, 16).ok()?
        } else {
            let byte = bytes[index];
            index += 1;
            byte
        };
        // A path separator must be literal, and encoded controls must not
        // acquire meaning after routing or another decoding pass.
        if byte == b'/' || byte == b'\\' || byte == b'%' || byte.is_ascii_control() {
            return None;
        }
        decoded.push(byte);
    }
    let decoded = String::from_utf8(decoded).ok()?;
    (!decoded.chars().any(char::is_control)).then_some(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_retains_absence_and_valid_query_fragment_destinations() {
        assert_eq!(safe_ui_return(None), None);
        for target in [
            "/ui",
            "/ui/",
            "/ui/resources?type=Patient&url=%2FPatient%3Fname%3DAna#results",
            "/ui/search-parameters?base=&q=Jos%C3%A9&sel=https%3A%2F%2Fexample.org%2Fsp",
            "/ui/sql/views?lib=a.b-1",
        ] {
            assert_eq!(safe_ui_return(Some(target)).as_deref(), Some(target));
        }
    }

    #[test]
    fn navigation_normalizes_traversal_within_ui() {
        assert_eq!(
            safe_ui_return(Some("/ui/sql/../resources")),
            Some("/ui/resources".into())
        );
        assert_eq!(
            safe_ui_return(Some("/ui/sql/%2e%2E/resources?type=Patient#x")),
            Some("/ui/resources?type=Patient#x".into())
        );
    }

    #[test]
    fn navigation_rejects_external_ambiguous_and_escaping_targets() {
        for target in [
            "",
            "ui/resources",
            " /ui",
            "//evil.test/ui",
            "https://evil.test/ui",
            "javascript:alert(1)",
            "/ui-other",
            "/uiframe/resources",
            "/ui/../resources",
            "/ui/../ui/resources",
            "/ui/%2e%2e/resources",
            "/ui/sql/../../ui",
            "/ui/%2f%2fevil.test",
            "/ui/%5c..%5c",
            "/ui/\\evil.test",
            "/ui/%00",
            "/ui/%0D%0a",
            "/ui/%252e%252e/resources",
            "/ui/%",
            "/ui/%GG",
            "/ui/%C2%85",
            "/ui?x=\n",
            "/ui#\t",
        ] {
            assert_eq!(safe_ui_return(Some(target)), None, "{target:?}");
        }
    }

    #[test]
    fn navigation_encodes_one_return_target_before_the_destination_fragment() {
        let origin = "/ui/search-parameters?base=Patient&sel=https%3A%2F%2Fexample.org%2Fx#detail";
        let target = with_return(
            "/ui/editor?type=SearchParameter&id=one&return_to=old#json",
            Some(origin),
        );
        let parsed = Url::parse(UI_ORIGIN).unwrap().join(&target).unwrap();
        let pairs: Vec<_> = parsed.query_pairs().collect();
        assert_eq!(
            pairs.iter().filter(|(key, _)| key == "return_to").count(),
            1
        );
        assert_eq!(
            pairs.iter().find(|(key, _)| key == "return_to").unwrap().1,
            origin
        );
        assert_eq!(parsed.fragment(), Some("json"));
        assert!(target.starts_with("/ui/editor?type=SearchParameter&id=one&return_to=%2Fui%2F"));
    }

    #[test]
    fn navigation_does_not_turn_missing_or_invalid_origins_into_explicit_fallbacks() {
        assert_eq!(
            with_return("/ui/sql/queries?lib=new#details", None),
            "/ui/sql/queries?lib=new#details"
        );
        assert_eq!(
            with_return("/ui/editor?type=Patient", Some("//evil.test")),
            "/ui/editor?type=Patient"
        );
        assert_eq!(with_return("https://evil.test", None), "/ui");
    }
}
