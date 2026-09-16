//! Path-segment helpers for the uri search type's `:above` / `:below` modifiers.

/// Computes all parent URIs for :above matching (shared with the reference
/// handler's URL/path-prefix `:above`).
///
/// For "http://example.org/fhir/ValueSet/123", returns:
/// - "http://example.org/fhir/ValueSet/123"
/// - "http://example.org/fhir/ValueSet"
/// - "http://example.org/fhir"
/// - "http://example.org"
pub fn compute_parent_uris(uri: &str) -> Vec<String> {
    let mut result = vec![uri.to_string()];

    // Strip query and fragment
    let base = uri
        .split('?')
        .next()
        .unwrap_or(uri)
        .split('#')
        .next()
        .unwrap_or(uri);

    // Find the scheme+authority part
    let scheme_end = if let Some(idx) = base.find("://") {
        // Find the first / after the authority
        base[idx + 3..].find('/').map(|i| idx + 3 + i)
    } else {
        None
    };

    let min_len = scheme_end.unwrap_or(0);

    let mut current = base.to_string();
    while let Some(last_slash) = current.rfind('/') {
        if last_slash < min_len {
            break;
        }
        current.truncate(last_slash);
        if !current.is_empty() {
            result.push(current.clone());
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parent_uris_walk_segments_down_to_the_authority() {
        let parents = compute_parent_uris("http://example.org/fhir/ValueSet/123");
        assert!(parents.contains(&"http://example.org/fhir/ValueSet/123".to_string()));
        assert!(parents.contains(&"http://example.org/fhir/ValueSet".to_string()));
        assert!(parents.contains(&"http://example.org/fhir".to_string()));
        assert!(parents.contains(&"http://example.org".to_string()));
        assert!(!parents.contains(&"http:/".to_string()));
        assert!(!parents.contains(&"".to_string()));

        // The original value (query string included) is kept verbatim as
        // the first element; the walk itself operates on the stripped base,
        // so the query is not reflected in any of the other entries.
        let with_query = "http://example.org/a/b?x=1";
        let parents = compute_parent_uris(with_query);
        assert_eq!(parents.first(), Some(&with_query.to_string()));
        assert!(parents.contains(&"http://example.org/a".to_string()));
    }
}
