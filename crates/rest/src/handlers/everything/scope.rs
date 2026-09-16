use std::collections::HashSet;

use helios_fhir::FhirVersion;
use helios_fhir::search::registry::SearchParameterRegistry;
use helios_persistence::types::{
    CompartmentMembership, SearchParamType, SearchParameter, SearchPrefix, SearchQuery,
    SearchValue, StoredResource,
};

use super::params::EverythingParams;

/// Types whose clinical date search parameter is not named `date`.
const CLINICAL_DATE_OVERRIDES: &[(&str, &str)] = &[
    ("Condition", "onset-date"),
    ("MedicationRequest", "authoredon"),
    ("MedicationStatement", "effective"),
    ("MedicationAdministration", "effective-time"),
    ("MedicationDispense", "whenhandedover"),
    ("Claim", "created"),
    ("ExplanationOfBenefit", "created"),
];

/// Segment 0 is always "Patient" (tier 1, served by `read`); segments 1.. are compartment member types.
pub(crate) fn build_segments(version: FhirVersion, types: Option<&[String]>) -> Vec<String> {
    let mut segments = vec!["Patient".to_string()];
    let members = crate::fhir_types::get_resource_type_names_for_version(version)
        .iter()
        .filter(|t| **t != "Patient")
        .filter(|t| !helios_fhir::get_compartment_params(version, "Patient", t).is_empty());
    match types {
        None => segments.extend(members.map(|t| t.to_string())),
        Some(wanted) => {
            // Table order, not `wanted`'s order: `_type` is a filter on
            // which compartment members to walk, not a request to reorder
            // the walk — clients that pass `_type=B,A` still get results in
            // the server's stable segment order.
            segments.extend(
                members
                    .filter(|t| wanted.iter().any(|w| w == *t))
                    .map(|t| t.to_string()),
            );
        }
    }
    segments
}

pub(crate) fn clinical_date_param(
    registry: &SearchParameterRegistry,
    resource_type: &str,
) -> Option<String> {
    // The override is only valid if the active FHIR version's registry
    // actually defines it — e.g. `MedicationAdministration.effective-time`
    // exists in R4/R4B but was renamed to `date` in R5/R6, so an R5/R6
    // registry has no `effective-time` param and the override must fall
    // through to the generic `date` lookup instead of silently matching
    // nothing.
    if let Some((_, p)) = CLINICAL_DATE_OVERRIDES
        .iter()
        .find(|(t, _)| *t == resource_type)
    {
        if registry.get_param(resource_type, p).is_some() {
            return Some((*p).to_string());
        }
    }
    registry
        .get_param(resource_type, "date")
        .filter(|def| def.param_type == SearchParamType::Date)
        .map(|_| "date".to_string())
}

fn date_param(name: &str, prefix: SearchPrefix, value: &str) -> SearchParameter {
    SearchParameter {
        name: name.to_string(),
        param_type: SearchParamType::Date,
        modifier: None,
        values: vec![SearchValue::new(prefix, value)],
        chain: vec![],
        components: vec![],
    }
}

pub(crate) fn build_segment_query(
    registry: &SearchParameterRegistry,
    version: FhirVersion,
    resource_type: &str,
    patient_id: &str,
    params: &EverythingParams,
    count: u32,
    cursor: Option<String>,
) -> SearchQuery {
    let mut query = SearchQuery::new(resource_type);
    query.compartment = Some(CompartmentMembership {
        params: helios_fhir::get_compartment_params(version, "Patient", resource_type)
            .iter()
            .map(|s| s.to_string())
            .collect(),
        reference: format!("Patient/{patient_id}"),
    });
    if let Some(since) = &params.since {
        query
            .parameters
            .push(date_param("_lastUpdated", SearchPrefix::Ge, since));
    }
    if params.start.is_some() || params.end.is_some() {
        if let Some(date_name) = clinical_date_param(registry, resource_type) {
            if let Some(start) = &params.start {
                query
                    .parameters
                    .push(date_param(&date_name, SearchPrefix::Ge, start));
            }
            if let Some(end) = &params.end {
                query
                    .parameters
                    .push(date_param(&date_name, SearchPrefix::Le, end));
            }
        }
    }
    query.count = Some(count);
    query.cursor = cursor;
    query.offset = None;
    query
}

fn walk_references(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(serde_json::Value::String(r)) = map.get("reference") {
                out.push(r.clone());
            }
            for v in map.values() {
                walk_references(v, out);
            }
        }
        serde_json::Value::Array(items) => items.iter().for_each(|v| walk_references(v, out)),
        _ => {}
    }
}

/// (type, id) pairs referenced by `resources` whose type is NOT a Patient-compartment member. Deduped, request order.
pub(crate) fn collect_supporting_refs(
    version: FhirVersion,
    resources: &[StoredResource],
) -> Vec<(String, String)> {
    let known = crate::fhir_types::get_resource_type_names_for_version(version);
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for res in resources {
        let mut raw = Vec::new();
        walk_references(res.content(), &mut raw);
        for r in raw {
            // Relative literal references: "Type/id" or "Type/id/_history/vid".
            // Anything else with a further '/' (absolute URLs, other nested
            // paths) is rejected.
            let Some((rt, rest)) = r.split_once('/') else {
                continue;
            };
            let id = match rest.split_once('/') {
                None => rest,
                Some((bare_id, tail)) => match tail.strip_prefix("_history/") {
                    Some(version) if !version.is_empty() => bare_id,
                    _ => continue,
                },
            };
            if id.is_empty() || !known.contains(&rt) {
                continue;
            }
            if !helios_fhir::get_compartment_params(version, "Patient", rt).is_empty() {
                continue; // a compartment member: it is (or will be) a `match`
            }
            if seen.insert((rt.to_string(), id.to_string())) {
                out.push((rt.to_string(), id.to_string()));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use helios_persistence::tenant::TenantId;
    use serde_json::json;

    use super::*;

    fn registry() -> SearchParameterRegistry {
        crate::test_support::spec_registry_r4()
    }

    fn params(list: &[(&str, &str)]) -> EverythingParams {
        let pairs: Vec<(String, String)> = list
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        EverythingParams::from_pairs(&pairs, FhirVersion::R4, 1000).unwrap()
    }

    fn stored(rt: &str, id: &str, content: serde_json::Value) -> StoredResource {
        StoredResource::new(rt, id, TenantId::new("test"), content, FhirVersion::R4)
    }

    #[test]
    fn segments_start_with_patient_then_members_in_table_order() {
        let segs = build_segments(FhirVersion::R4, None);
        assert_eq!(segs[0], "Patient");
        assert!(segs.contains(&"Observation".to_string()));
        assert!(segs.contains(&"Encounter".to_string()));
        assert!(
            !segs.contains(&"Practitioner".to_string()),
            "Practitioner is not a Patient-compartment member"
        );
        let dedup: HashSet<_> = segs.iter().collect();
        assert_eq!(dedup.len(), segs.len());
    }

    #[test]
    fn type_filter_restricts_members_but_keeps_patient() {
        // Requested in the *reverse* of table order, to prove `_type`
        // filters which members are walked without dictating the walk's
        // order — the output must still follow the compartment table.
        let wanted = ["Observation".to_string(), "Encounter".to_string()];
        let segs = build_segments(FhirVersion::R4, Some(&wanted));

        // Expected order is computed from the unfiltered walk (table
        // order), not hardcoded, so this test doesn't silently bitrot if
        // the table order changes.
        let expected: Vec<String> = build_segments(FhirVersion::R4, None)
            .into_iter()
            .filter(|t| t == "Patient" || wanted.contains(t))
            .collect();
        assert_eq!(segs, expected);
        assert_eq!(segs[0], "Patient");
        assert_ne!(
            segs,
            vec!["Patient", "Observation", "Encounter"],
            "must be table order, not `_type`'s request order"
        );
    }

    #[test]
    fn type_filter_ignores_non_members() {
        let segs = build_segments(FhirVersion::R4, Some(&["Practitioner".to_string()]));
        assert_eq!(segs, vec!["Patient"]);
    }

    #[test]
    fn clinical_date_map_prefers_overrides_then_date_param() {
        let reg = registry();
        assert_eq!(
            clinical_date_param(&reg, "Condition").as_deref(),
            Some("onset-date")
        );
        assert_eq!(
            clinical_date_param(&reg, "MedicationRequest").as_deref(),
            Some("authoredon")
        );
        assert_eq!(
            clinical_date_param(&reg, "Observation").as_deref(),
            Some("date")
        );
        assert_eq!(
            clinical_date_param(&reg, "Encounter").as_deref(),
            Some("date")
        );
        assert_eq!(clinical_date_param(&reg, "Coverage"), None);
    }

    /// The override map is hand-maintained and must not silently go stale
    /// against a version's actual search parameter registry —
    /// `MedicationAdministration.effective-time` exists in R4/R4B but was
    /// renamed to `date` in R5/R6, so a version whose registry doesn't
    /// define the overridden param name must fall through to `date`
    /// (and that `date` param must itself be resolvable by the registry)
    /// rather than resolving to a param that yields zero search results.
    #[test]
    #[allow(clippy::vec_init_then_push)]
    fn clinical_date_overrides_resolve_against_every_enabled_version() {
        #[allow(unused_mut)]
        let mut versions: Vec<FhirVersion> = Vec::new();
        #[cfg(feature = "R4")]
        versions.push(FhirVersion::R4);
        #[cfg(feature = "R4B")]
        versions.push(FhirVersion::R4B);
        #[cfg(feature = "R5")]
        versions.push(FhirVersion::R5);
        #[cfg(feature = "R6")]
        versions.push(FhirVersion::R6);
        assert!(!versions.is_empty(), "no FHIR version feature enabled");

        for version in versions {
            let reg = crate::test_support::spec_registry(version);
            let resolved = clinical_date_param(&reg, "MedicationAdministration");
            assert!(
                resolved.is_some(),
                "{version:?}: MedicationAdministration must resolve a clinical date param"
            );
            let name = resolved.unwrap();
            assert!(
                reg.get_param("MedicationAdministration", &name).is_some(),
                "{version:?}: resolved param '{name}' must exist in the {version:?} registry"
            );
        }
    }

    #[test]
    fn segment_query_scopes_to_compartment_and_applies_filters() {
        let reg = registry();
        let p = params(&[
            ("_since", "2021-01-01T00:00:00Z"),
            ("start", "2020"),
            ("end", "2020-12-31"),
        ]);
        let q = build_segment_query(
            &reg,
            FhirVersion::R4,
            "Observation",
            "p1",
            &p,
            7,
            Some("cur".into()),
        );
        assert_eq!(q.resource_type, "Observation");
        assert_eq!(q.count, Some(7));
        assert_eq!(q.cursor.as_deref(), Some("cur"));
        assert_eq!(q.offset, None);
        let c = q.compartment.as_ref().unwrap();
        assert_eq!(c.reference, "Patient/p1");
        assert!(c.params.contains(&"subject".to_string()));
        let names: Vec<(&str, SearchPrefix, &str)> = q
            .parameters
            .iter()
            .flat_map(|sp| {
                sp.values
                    .iter()
                    .map(move |v| (sp.name.as_str(), v.prefix, v.value.as_str()))
            })
            .collect();
        assert!(names.contains(&("_lastUpdated", SearchPrefix::Ge, "2021-01-01T00:00:00Z")));
        assert!(names.contains(&("date", SearchPrefix::Ge, "2020")));
        assert!(names.contains(&("date", SearchPrefix::Le, "2020-12-31")));
    }

    #[test]
    fn segment_query_skips_clinical_filter_for_types_without_date() {
        let reg = registry();
        let p = params(&[("start", "2020")]);
        let q = build_segment_query(&reg, FhirVersion::R4, "Coverage", "p1", &p, 10, None);
        assert!(q.parameters.is_empty());
    }

    #[test]
    fn supporting_refs_skips_compartment_members_and_dedups() {
        let obs = stored(
            "Observation",
            "o1",
            json!({
                "resourceType": "Observation", "id": "o1",
                "subject": { "reference": "Patient/p1" },
                "performer": [{ "reference": "Practitioner/dr1" }, { "reference": "Organization/org1" }],
                "encounter": { "reference": "Encounter/e1" },
                "note": [{ "authorReference": { "reference": "Practitioner/dr1" } }],
                "device": { "reference": "Device/dev1/_history/1" },
                "reasonReference": [{ "reference": "Organization/org2/extra" }]
            }),
        );
        let enc = stored(
            "Encounter",
            "e1",
            json!({
                "resourceType": "Encounter", "id": "e1",
                "serviceProvider": { "reference": "Organization/org1" },
                "participant": [{ "individual": { "reference": "Practitioner/dr1/_history/2" } }],
                "location": [{ "location": { "reference": "Location/l1" } }],
                "partOf": { "reference": "http://other.example/fhir/Encounter/abs" }
            }),
        );
        let refs = collect_supporting_refs(FhirVersion::R4, &[obs, enc]);
        assert_eq!(
            refs,
            vec![
                ("Practitioner".to_string(), "dr1".to_string()),
                ("Organization".to_string(), "org1".to_string()),
                ("Device".to_string(), "dev1".to_string()),
                ("Location".to_string(), "l1".to_string()),
            ]
        );
    }
}
