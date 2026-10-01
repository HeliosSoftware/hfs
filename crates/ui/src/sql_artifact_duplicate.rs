//! Assigns a fresh name and canonical to a SQL artifact copy before creating it.
//!
//! A canonical dependency does not identify its resource type, so URL collisions
//! must be checked across both ViewDefinitions and Libraries. Names only need
//! to distinguish peers of the same type. These are preflight checks, not an
//! atomic uniqueness constraint against concurrent writes or a lagging index.

use std::collections::HashSet;

use helios_fhir::FhirVersion;
use serde_json::Value;

use crate::{ConformanceSource, I18n};

const PAGE_SIZE: usize = 500;
const MAX_PAGES: usize = 100;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DuplicateError {
    Read(String),
    Incomplete,
}

impl DuplicateError {
    pub(crate) fn message(self, i18n: I18n) -> String {
        match self {
            Self::Read(reason) => i18n.t_arg("sql-duplicate-read-failed", "reason", reason),
            Self::Incomplete => i18n.t("sql-duplicate-incomplete"),
        }
    }
}

/// Prepares only a duplicate. Missing identity fields remain missing, and a
/// failed lookup leaves the submitted document untouched for the error render.
pub(crate) async fn prepare(
    source: &dyn ConformanceSource,
    resource_type: &str,
    resource: &mut Value,
    version: FhirVersion,
    tenant: &str,
) -> Result<(), DuplicateError> {
    let name = resource
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let url = resource
        .get("url")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let mut names = HashSet::new();
    let mut urls = HashSet::new();

    if name.is_some() || url.is_some() {
        collect_identities(
            source,
            resource_type,
            version,
            tenant,
            &mut names,
            &mut urls,
        )
        .await?;
        if url.is_some() {
            let other_type = if resource_type == "ViewDefinition" {
                "Library"
            } else {
                "ViewDefinition"
            };
            // A Library and a ViewDefinition can answer the same untyped
            // canonical reference. The other type's names are irrelevant.
            collect_identities(
                source,
                other_type,
                version,
                tenant,
                &mut HashSet::new(),
                &mut urls,
            )
            .await?;
        }
    }

    for ordinal in 1usize.. {
        let suffix = if ordinal == 1 {
            "_copy".to_string()
        } else {
            format!("_copy_{ordinal}")
        };
        let copy_name = name.as_ref().map(|name| format!("{name}{suffix}"));
        let copy_url = url.as_ref().map(|url| suffixed_url(url, &suffix));
        if copy_name.as_ref().is_some_and(|name| names.contains(name))
            || copy_url.as_ref().is_some_and(|url| urls.contains(url))
        {
            continue;
        }
        if let Some(map) = resource.as_object_mut() {
            map.remove("id");
            if let Some(name) = copy_name {
                map.insert("name".to_string(), Value::String(name));
            }
            if let Some(url) = copy_url {
                map.insert("url".to_string(), Value::String(url));
            }
        }
        return Ok(());
    }
    unreachable!("bounded catalogs cannot occupy every copy suffix")
}

async fn collect_identities(
    source: &dyn ConformanceSource,
    resource_type: &str,
    version: FhirVersion,
    tenant: &str,
    names: &mut HashSet<String>,
    urls: &mut HashSet<String>,
) -> Result<(), DuplicateError> {
    let mut offset = 0;
    for _ in 0..MAX_PAGES {
        // Filterless listing also works with S3's definition scans. The HTTP
        // source asks for an overflow row, so has_next does not rely on a
        // backend advertising a next link (S3 and some Mongo searches do not).
        let page = source
            .search_page(resource_type, &[], PAGE_SIZE, offset, version, tenant)
            .await
            .map_err(DuplicateError::Read)?;
        for resource in &page.resources {
            if let Some(name) = resource.get("name").and_then(Value::as_str) {
                names.insert(name.to_string());
            }
            if let Some(url) = resource.get("url").and_then(Value::as_str) {
                urls.insert(url.to_string());
            }
        }
        if !page.has_next {
            return Ok(());
        }
        if page.resources.is_empty() {
            return Err(DuplicateError::Incomplete);
        }
        offset += page.resources.len();
    }
    Err(DuplicateError::Incomplete)
}

/// Changes the path text without normalizing the canonical. Opaque URI bases
/// receive the same lexical suffix; this does not validate UUID/OID semantics.
fn suffixed_url(url: &str, suffix: &str) -> String {
    let end = url.find(['?', '#']).unwrap_or(url.len());
    let (base, tail) = url.split_at(end);
    let authority_without_path = base.split_once("://").is_some_and(|(scheme, rest)| {
        (scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
            && !rest.contains('/')
    });
    let slash = if authority_without_path { "/" } else { "" };
    format!("{base}{slash}{suffix}{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conformance::SearchPage;
    use async_trait::async_trait;
    use serde_json::json;
    use std::sync::Mutex;

    struct UnfinishedSource {
        empty: bool,
        offsets: Mutex<Vec<usize>>,
    }

    #[async_trait]
    impl ConformanceSource for UnfinishedSource {
        async fn fetch(&self, _: &str, _: FhirVersion, _: &str) -> Result<Vec<Value>, String> {
            unreachable!("duplication uses paginated listing")
        }

        async fn search_page(
            &self,
            _: &str,
            params: &[(String, String)],
            count: usize,
            offset: usize,
            _: FhirVersion,
            _: &str,
        ) -> Result<SearchPage, String> {
            assert!(params.is_empty());
            assert_eq!(count, PAGE_SIZE);
            self.offsets.lock().unwrap().push(offset);
            Ok(SearchPage {
                resources: if self.empty {
                    vec![]
                } else {
                    vec![json!({"id": offset})]
                },
                has_next: true,
            })
        }
    }

    #[test]
    fn url_suffix_preserves_query_fragment_and_opaque_text() {
        for (url, expected) in [
            (
                "https://example.org/Patient",
                "https://example.org/Patient_copy_2",
            ),
            ("https://example.org/", "https://example.org/_copy_2"),
            ("http://example.org", "http://example.org/_copy_2"),
            (
                "HTTPS://example.org?x=1#f",
                "HTTPS://example.org/_copy_2?x=1#f",
            ),
            (
                "http://example.org/path?q=a,b#part",
                "http://example.org/path_copy_2?q=a,b#part",
            ),
            (
                "http://example.org/path#part?text",
                "http://example.org/path_copy_2#part?text",
            ),
            ("urn:example:patient", "urn:example:patient_copy_2"),
        ] {
            assert_eq!(suffixed_url(url, "_copy_2"), expected);
        }
    }

    #[tokio::test]
    async fn unfinished_listing_fails_without_mutating_the_document() {
        for empty in [true, false] {
            let source = UnfinishedSource {
                empty,
                offsets: Mutex::new(vec![]),
            };
            let original =
                json!({"resourceType": "ViewDefinition", "id": "original", "name": "patients"});
            let mut document = original.clone();
            assert_eq!(
                prepare(
                    &source,
                    "ViewDefinition",
                    &mut document,
                    FhirVersion::R4,
                    "tenant"
                )
                .await,
                Err(DuplicateError::Incomplete)
            );
            assert_eq!(document, original);
            let offsets = source.offsets.lock().unwrap();
            assert_eq!(offsets.len(), if empty { 1 } else { MAX_PAGES });
            assert_eq!(offsets.last(), Some(&(offsets.len() - 1)));
        }
    }
}
