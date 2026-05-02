//! Shared helpers for the `bindings_r5` integration test crate.

use fhir_validation::R5FhirPathEvaluator;
use fhir_validation::terminology::service::RemoteTerminologyService;
use helios_fhir::{FhirResource, FhirVersion};
use reqwest::Client;
use std::time::Duration;

pub fn r5_evaluator_for(resource: &FhirResource) -> R5FhirPathEvaluator {
    let FhirResource::R5(r) = resource else {
        panic!("expected R5 FhirResource");
    };
    R5FhirPathEvaluator::new((**r).clone())
}

/// Base URL for a FHIR terminology server (trailing slash optional), if set in the environment.
pub fn terminology_base_url_from_env() -> Option<String> {
    std::env::var("FHIR_TERMINOLOGY_BASE_URL")
        .ok()
        .map(|s| s.trim().trim_end_matches('/').to_owned())
        .filter(|s| !s.is_empty())
}

/// Client for async binding tests that need `$validate-code` / `member-of` against a real server.
///
/// Uses `FHIR_TERMINOLOGY_BASE_URL` when set; otherwise defaults to `http://localhost:8080/fhir`
/// (typical Snowstorm FHIR API base).
pub fn remote_terminology_for_tests() -> RemoteTerminologyService {
    let base =
        terminology_base_url_from_env().unwrap_or_else(|| "http://localhost:8090".to_string());
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(8))
        .pool_idle_timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(8)
        .tcp_keepalive(Duration::from_secs(30))
        .build()
        .expect("failed to build reqwest client");
    RemoteTerminologyService::with_client(client, base, FhirVersion::R5)
}
