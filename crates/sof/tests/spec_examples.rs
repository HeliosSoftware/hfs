//! Runs the example ViewDefinitions published with the SQL on FHIR
//! specification (`tests/sql-on-fhir/examples/`, copied verbatim from
//! <https://github.com/HL7/sql-on-fhir/tree/main/input/resources/viewdefinition>)
//! through the structural lint and the in-memory engine.
//!
//! Every example must lint clean — `$sql-run` lints before it executes, so an
//! example the lint rejects is an example the server cannot run — and must
//! produce the rows expected for `test-bundle.json`.

#![cfg(feature = "R4")]

use helios_sof::lint::lint_view_definition;
use helios_sof::{ContentType, SofBundle, SofViewDefinition, run_view_definition};
use serde_json::{Value, json};
use std::path::PathBuf;

fn examples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/sql-on-fhir/examples")
}

fn read_json(name: &str) -> Value {
    let path = examples_dir().join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("Failed to parse {name}: {e}"))
}

fn run_example(name: &str) -> Value {
    let view_json = read_json(&format!("ViewDefinition-{name}.json"));

    let diagnostics = lint_view_definition(&view_json);
    assert!(
        diagnostics.is_empty(),
        "{name} does not lint clean: {diagnostics:#?}"
    );

    let view: helios_fhir::r4::ViewDefinition =
        serde_json::from_value(view_json.clone()).expect("Failed to create ViewDefinition");
    // The typed model must not lose anything the example says — in particular
    // `resourceDefinition`, which is not part of the ViewDefinition snapshot.
    let resource: helios_fhir::r4::Resource =
        serde_json::from_value(view_json.clone()).expect("Failed to create Resource");
    assert_eq!(
        serde_json::to_value(&resource).expect("Failed to serialize Resource"),
        view_json,
        "{name} does not survive a typed round trip"
    );
    let bundle: helios_fhir::r4::Bundle =
        serde_json::from_value(read_json("test-bundle.json")).expect("Failed to create bundle");

    let output = run_view_definition(
        SofViewDefinition::R4(view),
        SofBundle::R4(bundle),
        ContentType::Json,
    )
    .unwrap_or_else(|e| panic!("{name} failed to run: {e}"));
    serde_json::from_slice(&output).expect("Failed to parse output")
}

#[test]
fn every_example_is_covered() {
    let mut found: Vec<String> = std::fs::read_dir(examples_dir())
        .expect("Failed to list examples")
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.starts_with("ViewDefinition-"))
        .collect();
    found.sort();
    assert_eq!(
        found,
        [
            "ViewDefinition-CodeSystemHierarchy.json",
            "ViewDefinition-ConditionFlat.json",
            "ViewDefinition-EncounterFlat.json",
            "ViewDefinition-PatientAddresses.json",
            "ViewDefinition-PatientAndContactAddressUnion.json",
            "ViewDefinition-PatientDemographics.json",
            "ViewDefinition-PatientNamesWithIndex.json",
            "ViewDefinition-QuestionnaireResponseItems.json",
            "ViewDefinition-ShareablePatientDemographics.json",
            "ViewDefinition-UsCoreBloodPressures.json",
        ],
        "a newly synced example needs a test below"
    );
}

#[test]
fn code_system_hierarchy() {
    assert_eq!(
        run_example("CodeSystemHierarchy"),
        json!([
            {"id": "cs1", "parent_code": "A", "code": "A1"},
            {"id": "cs1", "parent_code": "A", "code": "A2"},
            {"id": "cs1", "parent_code": "A1", "code": "A1a"},
        ])
    );
}

#[test]
fn condition_flat() {
    assert_eq!(
        run_example("ConditionFlat"),
        json!([{
            "id": "c1",
            "patient_id": "p1",
            "encounter_id": "e1",
            "onset_datetime": "2023-05-01T00:00:00Z",
            "system": "http://snomed.info/sct",
            "code": "38341003",
            "category": "problem-list-item",
            "clinical_status": "active",
            "verification_status": "confirmed",
        }])
    );
}

#[test]
fn encounter_flat() {
    // `episodeOfCare` repeats, so `getReferenceKey` is invoked on a collection.
    assert_eq!(
        run_example("EncounterFlat"),
        json!([{
            "id": "e1",
            "status": "finished",
            "patient_id": "p1",
            "service_org_id": "org1",
            "period_start": "2023-05-01T09:00:00Z",
            "period_end": "2023-05-01T10:00:00Z",
            "EpisodeOfCareId": "eoc1",
            "type_sys": "http://snomed.info/sct",
            "type_code": "185349003",
            "practitioner_id": "pr1",
            "location_id": "l1",
        }])
    );
}

#[test]
fn patient_addresses() {
    assert_eq!(
        run_example("PatientAddresses"),
        json!([{
            "patient_id": "p1",
            "street": "1 Main St\nApt 2",
            "use": "home",
            "city": "Boston",
            "zip": "02101",
        }])
    );
}

#[test]
fn patient_and_contact_address_union() {
    assert_eq!(
        run_example("PatientAndContactAddressUnion"),
        json!([
            {"resource_id": "p1", "street": "1 Main St\nApt 2", "city": "Boston", "zip": "02101", "is_patient": true},
            {"resource_id": "p1", "street": "9 Elm", "city": "Salem", "zip": "01970", "is_patient": false},
        ])
    );
}

#[test]
fn patient_demographics() {
    let expected = json!([{
        "id": "p1",
        "gender": "female",
        "given_name": "Ann B",
        "family_name": "Smith",
    }]);
    assert_eq!(run_example("PatientDemographics"), expected);
    assert_eq!(run_example("ShareablePatientDemographics"), expected);
}

#[test]
fn patient_names_with_index() {
    assert_eq!(
        run_example("PatientNamesWithIndex"),
        json!([
            {"patient_id": "p1", "name_index": 0, "use": "official", "family": "Smith", "given": "Ann B"},
            {"patient_id": "p1", "name_index": 1, "use": "nickname", "family": null, "given": "Annie"},
        ])
    );
}

#[test]
fn questionnaire_response_items() {
    // The example repeats over `item` only, so the item nested under an
    // answer (`1.2.1` in the test bundle) is not reached.
    let rows = run_example("QuestionnaireResponseItems");
    let link_ids: Vec<&str> = rows
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["item_link_id"].as_str().unwrap())
        .collect();
    assert_eq!(link_ids, ["1", "1.1", "1.2", "2"]);
    assert_eq!(rows[1]["answer_value_string"], "Ann");
    assert_eq!(rows[2]["answer_value_integer"], 40);
    assert_eq!(rows[3]["answer_value_date"], "1984-01-01");
}

#[test]
fn us_core_blood_pressures() {
    assert_eq!(
        run_example("UsCoreBloodPressures"),
        json!([{
            "id": "o1",
            "patient_id": "p1",
            "effective_date_time": "2024-01-02T03:04:05Z",
            "sbp_quantity_system": "http://unitsofmeasure.org",
            "sbp_quantity_code": "mm[Hg]",
            "sbp_quantity_unit": "mmHg",
            "sbp_quantity_value": 120,
            "dbp_quantity_system": "http://unitsofmeasure.org",
            "dbp_quantity_code": "mm[Hg]",
            "dbp_quantity_unit": "mmHg",
            "dbp_quantity_value": 80,
        }])
    );
}
