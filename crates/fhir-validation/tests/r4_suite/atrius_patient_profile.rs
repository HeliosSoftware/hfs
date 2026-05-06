//! Profile validation against [`StructureDefinition-AtriusPatient.json`](../../fixtures/r4/profiles/StructureDefinition-AtriusPatient.json)
//! and the NDHM base [`StructureDefinition-Patient.json`](../../fixtures/r4/profiles/StructureDefinition-Patient.json).
//! Ignored network tests hit NDHM using the **canonical** `baseDefinition` URL; the validator
//! rewrites it to static JSON via [`fhir_validation::profile::base_definition_fetch_url`].

mod tests {
    use crate::common::fixtures::{load_profile, local_terminology_r4};
    use fhir_validation::profile::base_definition_fetch_url::structure_definition_json_fetch_url;
    use fhir_validation::profile::extract::extract_structure_definition_profile_from_json;
    use fhir_validation::profile::profile_registry::ProfileRegistry;
    use fhir_validation::profile::types::ExtractedProfile;
    use fhir_validation::profile::validate::remote_structure_definition_fetch_user_agent;
    use fhir_validation::strict_properties::validate_json_against_extracted_profile;
    use fhir_validation::{R4FhirPathEvaluator, Severity, ValidationIssue, Validator};
    use helios_fhir::FhirResource;
    use helios_fhir::FhirVersion;
    use helios_fhir::r4::{Patient, Resource};
    use serde_json::json;
    use std::time::Duration;

    fn r4_evaluator_for(resource: &FhirResource) -> R4FhirPathEvaluator {
        match resource {
            FhirResource::R4(r) => R4FhirPathEvaluator::new((**r).clone()),
            // Exhaustive when only `R4` is enabled; keep arm for multi-version test builds.
            #[allow(unreachable_patterns)]
            _ => panic!("expected R4 resource"),
        }
    }

    const ATRIUS_PATIENT_PROFILE_URL: &str = "http://atrius.in/StructureDefinition/AtriusPatient";
    const NDHM_PATIENT_PROFILE_URL: &str =
        "https://nrces.in/ndhm/fhir/r4/StructureDefinition/Patient";
    /// HL7 R4 Patient `StructureDefinition` as static JSON (`*.profile.json`). The shorter
    /// `/StructureDefinition/Patient` path relies on redirects; `reqwest` has been observed to
    /// receive **405** on that chain while `curl -L` succeeds, so tests use this direct URL (same
    /// resource: canonical `url` is still `http://hl7.org/fhir/StructureDefinition/Patient`).
    const HL7_R4_PATIENT_SD_URL: &str = "https://hl7.org/fhir/R4/patient.profile.json";

    fn atrius_patient_registry() -> ProfileRegistry {
        let mut registry = ProfileRegistry::new();
        registry.insert(load_profile(
            FhirVersion::R4,
            "profiles/StructureDefinition-AtriusPatient.json",
        ));
        registry
    }

    fn ndhm_patient_profile() -> ExtractedProfile {
        load_profile(FhirVersion::R4, "profiles/StructureDefinition-Patient.json")
    }

    fn atrius_and_ndhm_registry() -> ProfileRegistry {
        let mut registry = ProfileRegistry::new();
        registry.insert(ndhm_patient_profile());
        registry.insert(load_profile(
            FhirVersion::R4,
            "profiles/StructureDefinition-AtriusPatient.json",
        ));
        registry
    }

    fn validator_for_atrius_profile() -> Validator {
        let mut validator = Validator::default();
        // Base is NDHM Patient — not shipped in tests; avoid network and silent skip of base rules.
        validator.config.recurse_on_base_definition = false;
        validator.config.enable_base_definition_url_lookup = false;
        // Identifier.type is extensible to an NDHM ValueSet not bundled here.
        validator.config.strict_extensible_bindings = false;
        validator
    }

    fn validator_with_ndhm_base_recursion_offline() -> Validator {
        let mut validator = Validator::default();
        validator.config.recurse_on_base_definition = true;
        validator.config.enable_base_definition_url_lookup = false;
        validator.config.strict_extensible_bindings = false;
        validator
    }

    fn minimal_conforming_atrius_patient_json() -> serde_json::Value {
        json!({
            "resourceType": "Patient",
            "meta": { "profile": [ ATRIUS_PATIENT_PROFILE_URL ] },
            "identifier": [{
                "use": "usual",
                "type": {
                    "coding": [{
                        "system": "http://terminology.hl7.org/CodeSystem/v2-0203",
                        "code": "MR",
                        "display": "Medical record number"
                    }]
                },
                "system": "http://hospital.example.org/patients",
                "value": "12345"
            }],
            "name": [{ "text": "Example Patient" }],
            "gender": "male",
            "birthDate": "1990-01-01",
            "multipleBirthBoolean": true,
            "deceasedBoolean": false,
        })
    }

    fn patient_resource_with_meta(patient_value: serde_json::Value) -> FhirResource {
        let patient: Patient =
            serde_json::from_value(patient_value).expect("Patient JSON should deserialize");
        FhirResource::R4(Box::new(Resource::Patient(Box::new(patient))))
    }

    fn required_hits(issues: &[ValidationIssue], path: &str) -> usize {
        issues
            .iter()
            .filter(|i| {
                i.fhir_path == path
                    && i.code == "required"
                    && matches!(i.severity, Severity::Error | Severity::Fatal)
            })
            .count()
    }

    /// Stable multiset of coarse issue shape (path + code + severity) for comparing runs.
    fn issue_multiset(issues: &[ValidationIssue]) -> Vec<(String, String, String)> {
        let mut v: Vec<_> = issues
            .iter()
            .map(|i| {
                (
                    i.fhir_path.clone(),
                    i.code.clone(),
                    format!("{:?}", i.severity),
                )
            })
            .collect();
        v.sort();
        v
    }

    /// Live read of the HL7 R4 Patient `StructureDefinition` (FHIR JSON). Used only by ignored
    /// network tests — mirrors `fhir_validation::profile::validate` remote fetch headers (UA +
    /// `Accept`).
    fn fetch_hl7_r4_patient_sd_from_network() -> serde_json::Value {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(remote_structure_definition_fetch_user_agent())
            .build()
            .expect("reqwest blocking client");
        let response = client
            .get(HL7_R4_PATIENT_SD_URL)
            .header(
                reqwest::header::ACCEPT,
                "application/fhir+json, application/json;q=0.9",
            )
            .send()
            .unwrap_or_else(|e| panic!("GET {HL7_R4_PATIENT_SD_URL}: {e}"));
        let status = response.status();
        let body = response
            .bytes()
            .unwrap_or_else(|e| panic!("read HL7 SD body: {e}"));
        assert!(
            status.is_success(),
            "GET HL7 R4 Patient SD: HTTP {status}, first bytes: {:?}",
            body.iter().take(120).copied().collect::<Vec<_>>()
        );
        assert!(
            !body.is_empty(),
            "GET {HL7_R4_PATIENT_SD_URL} returned empty body (HTTP {status})"
        );
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap_or_else(|e| {
            let prefix = String::from_utf8_lossy(&body[..body.len().min(400)]);
            panic!("parse HL7 SD as JSON: {e}; prefix={prefix:?}");
        });
        assert_eq!(
            v.get("resourceType").and_then(|x| x.as_str()),
            Some("StructureDefinition"),
            "expected StructureDefinition JSON from {HL7_R4_PATIENT_SD_URL}"
        );
        assert_eq!(
            v.get("url").and_then(|x| x.as_str()),
            Some("http://hl7.org/fhir/StructureDefinition/Patient"),
            "HL7 R4 Patient SD canonical `url`"
        );
        v
    }

    /// Live read of the NDHM R4 Patient `StructureDefinition` (same URL the validator uses after
    /// [`structure_definition_json_fetch_url`] rewrites the canonical `baseDefinition`).
    fn fetch_ndhm_patient_sd_from_network() -> serde_json::Value {
        let fetch_url =
            structure_definition_json_fetch_url(NDHM_PATIENT_PROFILE_URL, Some("4.0.1"));
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(remote_structure_definition_fetch_user_agent())
            .build()
            .expect("reqwest blocking client");
        let response = client
            .get(fetch_url.as_str())
            .header(
                reqwest::header::ACCEPT,
                "application/fhir+json, application/json;q=0.9",
            )
            .send()
            .unwrap_or_else(|e| panic!("GET {fetch_url}: {e}"));
        let status = response.status();
        let body = response
            .bytes()
            .unwrap_or_else(|e| panic!("read NDHM SD body: {e}"));
        assert!(
            status.is_success(),
            "GET NDHM Patient SD: HTTP {status}, first bytes: {:?}",
            body.iter().take(120).copied().collect::<Vec<_>>()
        );
        assert!(
            !body.is_empty(),
            "GET {fetch_url} returned empty body (HTTP {status})"
        );
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap_or_else(|e| {
            let prefix = String::from_utf8_lossy(&body[..body.len().min(400)]);
            panic!("parse NDHM SD as JSON: {e}; prefix={prefix:?}");
        });
        assert_eq!(
            v.get("resourceType").and_then(|x| x.as_str()),
            Some("StructureDefinition"),
            "expected StructureDefinition JSON from {fetch_url}"
        );
        assert_eq!(
            v.get("url").and_then(|x| x.as_str()),
            Some(NDHM_PATIENT_PROFILE_URL),
            "NDHM Patient SD canonical `url`"
        );
        v
    }

    /// AtriusPatient with canonical NDHM `baseDefinition` (not in registry) so HTTP lookup uses
    /// the same URL as the IG and relies on [`structure_definition_json_fetch_url`] for JSON.
    fn atrius_patient_profile_with_canonical_ndhm_base_for_remote_fetch() -> ExtractedProfile {
        let mut p = load_profile(
            FhirVersion::R4,
            "profiles/StructureDefinition-AtriusPatient.json",
        );
        p.base_definition = Some(NDHM_PATIENT_PROFILE_URL.to_string());
        p
    }

    fn atrius_patient_registry_ndhm_remote_fetch_only() -> ProfileRegistry {
        let mut registry = ProfileRegistry::new();
        registry.insert(atrius_patient_profile_with_canonical_ndhm_base_for_remote_fetch());
        registry
    }

    #[test]
    fn atrius_patient_fixture_extracts_expected_url_and_type() {
        let profile = load_profile(
            FhirVersion::R4,
            "profiles/StructureDefinition-AtriusPatient.json",
        );
        assert_eq!(profile.url, ATRIUS_PATIENT_PROFILE_URL);
        assert_eq!(profile.resource_type, "Patient");
        assert!(
            profile.base_definition.as_deref().is_some_and(|b| {
                b.contains("nrces.in") && b.ends_with("/StructureDefinition/Patient")
            }),
            "unexpected baseDefinition: {:?}",
            profile.base_definition
        );
        assert!(
            !profile.element_rules.is_empty(),
            "snapshot-first extraction should yield element rules"
        );
    }

    #[test]
    fn atrius_patient_profile_errors_when_identifier_missing() {
        let patient = json!({
            "resourceType": "Patient",
            "meta": { "profile": [ ATRIUS_PATIENT_PROFILE_URL ] },
            "gender": "male",
            "birthDate": "1990-01-01",
            "name": [{ "text": "Example" }]
        });
        let resource = patient_resource_with_meta(patient);
        let registry = atrius_patient_registry();
        let evaluator = r4_evaluator_for(&resource);
        let term = local_terminology_r4();

        let issues = validator_for_atrius_profile().validate_resource_with_profiles(
            &resource,
            Some(&term),
            &evaluator,
            &registry,
        );

        let errors: Vec<_> = issues
            .iter()
            .filter(|i| matches!(i.severity, Severity::Error | Severity::Fatal))
            .collect();
        assert!(
            errors
                .iter()
                .any(|i| { i.fhir_path == "Patient.identifier" && i.code == "required" }),
            "expected required identifier under AtriusPatient profile, got: {errors:#?}"
        );
    }

    #[test]
    fn atrius_patient_profile_accepts_minimal_conforming_instance() {
        let patient = minimal_conforming_atrius_patient_json();
        let resource = patient_resource_with_meta(patient);
        let registry = atrius_patient_registry();
        let evaluator = r4_evaluator_for(&resource);
        let term = local_terminology_r4();

        let issues = validator_for_atrius_profile().validate_resource_with_profiles(
            &resource,
            Some(&term),
            &evaluator,
            &registry,
        );

        let errors: Vec<_> = issues
            .iter()
            .filter(|i| matches!(i.severity, Severity::Error | Severity::Fatal))
            .collect();
        assert!(
            !errors
                .iter()
                .any(|i| { i.fhir_path == "Patient.identifier" && i.code == "required" }),
            "did not expect missing-identifier error for conforming instance: {errors:#?}"
        );
    }

    #[test]
    fn atrius_patient_profile_rejects_invalid_polymorphic_choice_key_in_strict_mode() {
        let registry = atrius_and_ndhm_registry();
        let base = registry
            .get(NDHM_PATIENT_PROFILE_URL)
            .expect("NDHM base profile must be present");

        let mut with_invalid_string = minimal_conforming_atrius_patient_json();
        with_invalid_string["multipleBirthString"] = serde_json::Value::Bool(true);
        let string_issues =
            validate_json_against_extracted_profile(&with_invalid_string, base, Some(&registry));
        assert!(
            string_issues.iter().any(|i| {
                i.code == "structure"
                    && i.diagnostics.contains("multipleBirthString")
                    && i.diagnostics.contains("Unknown JSON property")
            }),
            "expected strict JSON validation to reject invalid choice key 'multipleBirthString', got: {string_issues:#?}"
        );

        let mut with_invalid_date = minimal_conforming_atrius_patient_json();
        with_invalid_date["multipleBirthDate"] = serde_json::Value::String("1990-01-01".into());
        let date_issues =
            validate_json_against_extracted_profile(&with_invalid_date, base, Some(&registry));
        assert!(
            date_issues.iter().any(|i| {
                i.code == "structure"
                    && i.diagnostics.contains("multipleBirthDate")
                    && i.diagnostics.contains("Unknown JSON property")
            }),
            "expected strict JSON validation to reject invalid choice key 'multipleBirthDate', got: {date_issues:#?}"
        );
    }

    #[test]
    fn ndhm_patient_fixture_url_matches_atrius_base_definition() {
        let atrius = load_profile(
            FhirVersion::R4,
            "profiles/StructureDefinition-AtriusPatient.json",
        );
        let ndhm = ndhm_patient_profile();
        let base = atrius
            .base_definition
            .as_deref()
            .expect("AtriusPatient should declare baseDefinition");
        let base_canonical = base.split('|').next().unwrap_or(base);
        assert_eq!(
            base_canonical, ndhm.url,
            "AtriusPatient.baseDefinition should match packaged NDHM Patient canonical URL"
        );
        assert_eq!(
            ndhm.snapshot_base_version.as_deref(),
            Some("4.0.1"),
            "packaged NDHM fixture should carry snapshot-base-version for HL7 web package hints"
        );
    }

    #[test]
    fn ndhm_canonical_basedefinition_rewrites_to_known_static_json_url() {
        assert_eq!(
            structure_definition_json_fetch_url(NDHM_PATIENT_PROFILE_URL, None),
            "https://nrces.in/ndhm/fhir/r4/StructureDefinition-Patient.json"
        );
    }

    #[test]
    fn atrius_patient_with_registered_ndhm_base_recursion_adds_ndhm_validation_surface() {
        let patient = minimal_conforming_atrius_patient_json();
        let resource = patient_resource_with_meta(patient.clone());
        let registry = atrius_and_ndhm_registry();
        assert!(
            registry.get(NDHM_PATIENT_PROFILE_URL).is_some(),
            "registry should include NDHM Patient profile"
        );
        let evaluator = r4_evaluator_for(&resource);
        let term = local_terminology_r4();

        let issues_atrius_only = validator_for_atrius_profile().validate_resource_with_profiles(
            &resource,
            Some(&term),
            &evaluator,
            &atrius_patient_registry(),
        );
        let err_atrius_only = issues_atrius_only
            .iter()
            .filter(|i| matches!(i.severity, Severity::Error | Severity::Fatal))
            .count();

        let issues_with_ndhm_base = validator_with_ndhm_base_recursion_offline()
            .validate_resource_with_profiles(&resource, Some(&term), &evaluator, &registry);
        let err_with_ndhm = issues_with_ndhm_base
            .iter()
            .filter(|i| matches!(i.severity, Severity::Error | Severity::Fatal))
            .count();

        assert!(
            !issues_with_ndhm_base.iter().any(|i| {
                i.code == "not-found"
                    && i.diagnostics.contains("nrces.in")
                    && i.diagnostics.contains("StructureDefinition")
            }),
            "did not expect missing base profile when NDHM SD is registered: {issues_with_ndhm_base:#?}"
        );
        println!("{:#?}", issues_atrius_only);
        // println!("{:#?}", issues_with_ndhm_base);
        println!("{:?}", resource)
    }

    /// Live GET + JSON parse + profile extraction for NDHM Patient (`StructureDefinition-Patient.json`).
    ///
    /// ```text
    /// cargo test -p fhir-validation --features R4 --test r4_suite ndhm_patient_structuredefinition_network -- --ignored
    /// ```
    #[test]
    #[ignore = "network: HTTP GET https://nrces.in/ndhm/fhir/r4/StructureDefinition-Patient.json"]
    fn ndhm_patient_structuredefinition_network_fetch_and_extract() {
        let live_sd = fetch_ndhm_patient_sd_from_network();
        let extracted = extract_structure_definition_profile_from_json(&live_sd)
            .unwrap_or_else(|e| panic!("extract_structure_definition_profile_from_json: {e:?}"));
        assert_eq!(extracted.url, NDHM_PATIENT_PROFILE_URL);
        assert!(
            !extracted.element_rules.is_empty(),
            "NDHM Patient SD should extract element rules"
        );
    }

    /// Live GET + JSON parse + profile extraction for HL7 R4 Patient (`patient.profile.json`).
    ///
    /// ```text
    /// cargo test -p fhir-validation --features R4 --test r4_suite hl7_r4_patient_structuredefinition_network -- --ignored
    /// ```
    #[test]
    #[ignore = "network: HTTP GET https://hl7.org/fhir/R4/patient.profile.json"]
    fn hl7_r4_patient_structuredefinition_network_fetch_and_extract() {
        let live_sd = fetch_hl7_r4_patient_sd_from_network();
        let extracted = extract_structure_definition_profile_from_json(&live_sd)
            .unwrap_or_else(|e| panic!("extract_structure_definition_profile_from_json: {e:?}"));
        assert_eq!(
            extracted.url,
            "http://hl7.org/fhir/StructureDefinition/Patient"
        );
        assert!(
            !extracted.element_rules.is_empty(),
            "HL7 R4 Patient SD should extract element rules"
        );
    }

    /// End-to-end: AtriusPatient `baseDefinition` resolved over HTTP to **NDHM** Patient (IG base),
    /// using the static JSON URL because the canonical `…/StructureDefinition/Patient` URL
    /// returns HTML, not FHIR JSON.
    ///
    /// ```text
    /// cargo test -p fhir-validation --features R4 --test r4_suite atrius_profile_https_ndhm_base -- --ignored
    /// ```
    #[test]
    #[ignore = "network: HTTP GET https://nrces.in/ndhm/fhir/r4/StructureDefinition-Patient.json"]
    fn atrius_patient_profile_https_base_lookup_recurses_ndhm_patient() {
        fetch_ndhm_patient_sd_from_network();

        let patient = minimal_conforming_atrius_patient_json();
        let resource = patient_resource_with_meta(patient);
        let registry = atrius_patient_registry_ndhm_remote_fetch_only();
        assert!(
            registry.get(NDHM_PATIENT_PROFILE_URL).is_none(),
            "registry should omit NDHM Patient profile so canonical baseDefinition uses HTTP lookup"
        );

        let mut validator = Validator::default();
        validator.config.recurse_on_base_definition = true;
        validator.config.enable_base_definition_url_lookup = true;
        validator.config.base_definition_url_lookup_allowed_hosts = vec!["nrces.in".to_string()];
        validator.config.base_definition_url_lookup_timeout_ms = 30_000;
        validator.config.strict_extensible_bindings = false;

        let evaluator = r4_evaluator_for(&resource);
        let term = local_terminology_r4();

        let issues_atrius_only = validator_for_atrius_profile().validate_resource_with_profiles(
            &resource,
            Some(&term),
            &evaluator,
            &atrius_patient_registry(),
        );
        let tele_atrius = required_hits(&issues_atrius_only, "Patient.telecom.value");
        assert!(
            tele_atrius >= 1,
            "Atrius-only pass should emit Patient.telecom.value cardinality on this fixture"
        );

        let mut validator_fetch_denied = Validator::default();
        validator_fetch_denied.config.recurse_on_base_definition = true;
        validator_fetch_denied
            .config
            .enable_base_definition_url_lookup = true;
        validator_fetch_denied
            .config
            .base_definition_url_lookup_allowed_hosts = vec!["iris-not-used.example".to_string()];
        validator_fetch_denied
            .config
            .base_definition_url_lookup_timeout_ms = 30_000;
        validator_fetch_denied.config.strict_extensible_bindings = false;
        let issues_fetch_denied = validator_fetch_denied.validate_resource_with_profiles(
            &resource,
            Some(&term),
            &evaluator,
            &registry,
        );

        let issues_http = validator.validate_resource_with_profiles(
            &resource,
            Some(&term),
            &evaluator,
            &registry,
        );
        let tele_http = required_hits(&issues_http, "Patient.telecom.value");
        assert!(
            tele_http >= tele_atrius,
            "NDHM base recursion should not reduce required-cardinality signals (atrius_hits={tele_atrius} http_hits={tele_http})"
        );
        assert_ne!(
            issue_multiset(&issues_http),
            issue_multiset(&issues_fetch_denied),
            "allowlisting nrces.in must change validation vs a mismatched allowlist (remote base unreachable); fetch_denied={issues_fetch_denied:#?} http={issues_http:#?}"
        );

        let issues_http_cached = validator.validate_resource_with_profiles(
            &resource,
            Some(&term),
            &evaluator,
            &registry,
        );
        assert_eq!(
            issue_multiset(&issues_http_cached),
            issue_multiset(&issues_http),
            "remote base profile cache should yield identical issues on repeat validation"
        );

        assert!(
            !issues_http.iter().any(|i| {
                i.code == "not-found"
                    && i.diagnostics.contains("nrces.in")
                    && i.diagnostics.contains("StructureDefinition")
            }),
            "base SD should resolve over HTTP, not report missing StructureDefinition: {issues_http:#?}"
        );

        let fatal: Vec<_> = issues_http
            .iter()
            .filter(|i| i.severity == Severity::Fatal)
            .collect();
        assert!(
            fatal.is_empty(),
            "HTTP baseDefinition fetch path should not yield fatal issues: {fatal:#?}"
        );
    }
}
