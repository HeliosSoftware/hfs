//! Construction of public URLs advertised by the REST API.

use url::{Host, Url};

use helios_persistence::types::SearchBundle;

/// A validated public base URL.
///
/// Path components are appended through [`Url::path_segments_mut`] so tenant,
/// resource type, and resource id values cannot change the configured path
/// structure.
#[derive(Clone, Debug)]
pub(crate) struct PublicUrl {
    url: Url,
    normalized: String,
}

pub(crate) fn rewrite_bundle_full_urls(
    bundle: &mut SearchBundle,
    mut public_url: impl FnMut(&str, &str) -> String,
) {
    for entry in &mut bundle.entry {
        let Some(resource) = entry.resource.as_ref() else {
            continue;
        };
        let Some(resource_type) = resource
            .get("resourceType")
            .and_then(|value| value.as_str())
        else {
            continue;
        };
        let Some(id) = resource.get("id").and_then(|value| value.as_str()) else {
            continue;
        };
        entry.full_url = Some(public_url(resource_type, id));
    }
}

impl PublicUrl {
    /// Parses and normalizes a configured public base URL.
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        let authority = value
            .split_once("://")
            .map(|(_, authority)| authority)
            .ok_or_else(|| "HFS_BASE_URL must be an absolute URL".to_string())?;
        if authority.is_empty() || authority.starts_with('/') {
            return Err("HFS_BASE_URL must include a host".to_string());
        }
        if authority
            .split(['/', '?', '#'])
            .next()
            .is_some_and(|authority| authority.contains('@'))
        {
            return Err("HFS_BASE_URL must not include user information".to_string());
        }
        let mut url = Url::parse(value).map_err(|error| match error {
            url::ParseError::EmptyHost => "HFS_BASE_URL must include a host".to_string(),
            _ => format!("HFS_BASE_URL must be an absolute URL: {error}"),
        })?;

        if !matches!(url.scheme(), "http" | "https") {
            return Err("HFS_BASE_URL must use the http or https scheme".to_string());
        }
        if url.query().is_some() {
            return Err("HFS_BASE_URL must not include a query string".to_string());
        }
        if url.fragment().is_some() {
            return Err("HFS_BASE_URL must not include a fragment".to_string());
        }

        let trimmed_path = url.path().trim_end_matches('/').to_string();
        url.set_path(&trimmed_path);
        let normalized = url.as_str().trim_end_matches('/').to_string();

        Ok(Self { url, normalized })
    }

    /// Returns the normalized base without a trailing slash.
    pub(crate) fn as_str(&self) -> &str {
        &self.normalized
    }

    /// Returns a URL with the supplied path segments appended to this base.
    pub(crate) fn with_segments<I, T>(&self, segments: I) -> String
    where
        I: IntoIterator<Item = T>,
        T: AsRef<str>,
    {
        let mut url = self.url.clone();
        {
            let mut path = url
                .path_segments_mut()
                .expect("validated HTTP URLs can hold path segments");
            path.pop_if_empty();
            for segment in segments {
                path.push(segment.as_ref());
            }
        }
        url.to_string().trim_end_matches('/').to_string()
    }

    /// Returns a URL with path segments and an already encoded query string.
    pub(crate) fn with_segments_and_query<I, T>(&self, segments: I, query: &str) -> String
    where
        I: IntoIterator<Item = T>,
        T: AsRef<str>,
    {
        let value = self.with_segments(segments);
        if query.is_empty() {
            return value;
        }
        let mut url = Url::parse(&value).expect("URL was built from a validated public base");
        url.set_query(Some(query));
        url.to_string()
    }

    /// Returns whether the advertised host resolves syntactically to loopback.
    pub(crate) fn is_loopback(&self) -> bool {
        self.url.host().is_some_and(|host| match host {
            Host::Domain(host) => host.eq_ignore_ascii_case("localhost"),
            Host::Ipv4(address) => address.is_loopback(),
            Host::Ipv6(address) => address.is_loopback(),
        })
    }

    /// Returns the effective port, including the scheme default.
    pub(crate) fn port_or_known_default(&self) -> Option<u16> {
        self.url.port_or_known_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use helios_persistence::types::BundleEntry;
    use serde_json::json;

    #[test]
    fn preserves_prefix_and_encodes_appended_segments() {
        let base = PublicUrl::parse("https://example.test/fhir/").unwrap();
        assert_eq!(base.as_str(), "https://example.test/fhir");
        assert_eq!(
            base.with_segments(["tenant with space", "Patient", "a/b"]),
            "https://example.test/fhir/tenant%20with%20space/Patient/a%2Fb"
        );

        let encoded_prefix = PublicUrl::parse("https://example.test/fhir%20api/").unwrap();
        assert_eq!(
            encoded_prefix.with_segments(["Patient"]),
            "https://example.test/fhir%20api/Patient"
        );
    }

    #[test]
    fn recognizes_loopback_host_forms() {
        for value in [
            "http://localhost:8080",
            "http://127.0.0.1:8080",
            "http://[::1]:8080",
        ] {
            assert!(PublicUrl::parse(value).unwrap().is_loopback(), "{value}");
        }
        assert!(
            !PublicUrl::parse("https://fhir.example.test")
                .unwrap()
                .is_loopback()
        );
    }

    #[test]
    fn rejects_an_absolute_http_url_without_a_host() {
        assert_eq!(
            PublicUrl::parse("http://?tenant=acme").unwrap_err(),
            "HFS_BASE_URL must include a host"
        );
    }

    #[test]
    fn rejects_user_information() {
        assert_eq!(
            PublicUrl::parse("https://user:password@example.test/fhir").unwrap_err(),
            "HFS_BASE_URL must not include user information"
        );
    }

    #[test]
    fn rewrites_only_entries_with_a_resource_type_and_id() {
        let mut bundle = SearchBundle::new()
            .with_entry(BundleEntry {
                full_url: Some("unchanged-empty".to_string()),
                resource: None,
                search: None,
            })
            .with_entry(BundleEntry::match_entry(
                "unchanged-type",
                json!({"id": "patient-1"}),
            ))
            .with_entry(BundleEntry::match_entry(
                "unchanged-id",
                json!({"resourceType": "Patient"}),
            ))
            .with_entry(BundleEntry::match_entry(
                "old",
                json!({"resourceType": "Patient", "id": "patient-1"}),
            ));

        rewrite_bundle_full_urls(&mut bundle, |resource_type, id| {
            format!("https://public.example/fhir/{resource_type}/{id}")
        });

        assert_eq!(bundle.entry[0].full_url.as_deref(), Some("unchanged-empty"));
        assert_eq!(bundle.entry[1].full_url.as_deref(), Some("unchanged-type"));
        assert_eq!(bundle.entry[2].full_url.as_deref(), Some("unchanged-id"));
        assert_eq!(
            bundle.entry[3].full_url.as_deref(),
            Some("https://public.example/fhir/Patient/patient-1")
        );
    }
}
