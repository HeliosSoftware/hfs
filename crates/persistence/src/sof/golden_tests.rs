//! Full SQL and binding snapshots of the private SQL runner preamble.
//!
//! Normal tests only compare. Regenerate deliberately with
//! `HFS_UPDATE_SOF_GOLDEN=1 cargo test -p helios-persistence --lib --features postgres sof::golden_tests`.

use helios_fhir::FhirVersion;
use serde_json::{Value, json};

use crate::core::sof_runner::ViewFilters;

use super::compiler::SqlDialect;
use super::ir::LitValue;
use super::runtime::{RuntimeParam, prepare_sql_run};

struct Case {
    name: &'static str,
    view: Value,
    filters: ViewFilters,
}

fn view(resource: &str, select: Value) -> Value {
    json!({"resourceType":"ViewDefinition","resource":resource,"status":"active","select":select})
}

fn cases() -> Vec<Case> {
    let patient = |name, select| Case {
        name,
        view: view("Patient", select),
        filters: ViewFilters::default(),
    };
    let mut cases = vec![
        patient(
            "flat",
            json!([{"column":[{"path":"id","name":"id"},{"path":"active","name":"active","type":"boolean"}]}]),
        ),
        patient(
            "foreach",
            json!([{"column":[{"path":"id","name":"r"}]},{"forEach":"name","column":[{"path":"family","name":"u0"}]}]),
        ),
        patient(
            "nullable",
            json!([{"forEachOrNull":"name.where(use = 'official')","column":[{"path":"family","name":"family"}]}]),
        ),
        patient(
            "nested",
            json!([{"forEach":"name","select":[{"column":[{"path":"family","name":"family"}]},
            {"forEach":"given","column":[{"path":"$this","name":"given","type":"string"}]}]}]),
        ),
        patient(
            "chained",
            json!([{"forEach":"name.given","column":[{"path":"$this","name":"given","type":"string"}]}]),
        ),
        patient(
            "cartesian",
            json!([{"forEach":"name","column":[{"path":"family","name":"family"}]},
            {"forEach":"address","column":[{"path":"city","name":"city"}]}]),
        ),
        patient(
            "union",
            json!([{"unionAll":[{"column":[{"path":"id","name":"value"}]},
            {"forEach":"name","column":[{"path":"family","name":"value"}]}]}]),
        ),
        patient(
            "indexed",
            json!([{"forEachOrNull":"name[1]","column":[{"path":"family","name":"family"}]}]),
        ),
        patient(
            "collection",
            json!([{"column":[{"path":"name","name":"objects","collection":true},
            {"path":"name.family","name":"families","collection":true},{"path":"name.given","name":"givens","collection":true}]}]),
        ),
        patient(
            "join",
            json!([{"column":[{"path":"name.given.join('|')","name":"joined","type":"string"}]}]),
        ),
        patient(
            "pick",
            json!([{"column":[{"path":"name.where(use = 'official').first().family","name":"pick","type":"string"}]}]),
        ),
        patient(
            "bare-pick",
            json!([{"column":[{"path":"name.where(use = 'official').family","name":"pick","type":"string"}]}]),
        ),
        patient(
            "row-index-legacy",
            json!([{"forEach":"name","column":[{"path":"family","name":"family"},
            {"path":"%rowIndex","name":"index","type":"integer"}]}]),
        ),
    ];
    cases.push(Case {name:"repeat",view:view("QuestionnaireResponse",json!([
        {"column":[{"path":"id","name":"id"},{"path":"item.linkId","name":"roots","collection":true}]},
        {"repeat":["item","answer.item"],"column":[{"path":"linkId","name":"link"},
            {"path":"answer.valueString","name":"answers","collection":true},
            {"path":"answer.valueString.join('|')","name":"joined","type":"string"},
            {"path":"answer.where(valueString != 'skip').first().valueString","name":"picked","type":"string"}]}])),filters:ViewFilters::default()});
    let flat = cases[0].view.clone();
    for (name, filters) in [
        (
            "since",
            ViewFilters {
                since: Some("2024-01-01T12:34:56.123456789Z".parse().unwrap()),
                ..Default::default()
            },
        ),
        (
            "patient",
            ViewFilters {
                patient: vec!["Patient/p2".into(), "p1".into()],
                ..Default::default()
            },
        ),
        (
            "group",
            ViewFilters {
                group: vec!["Group/g1".into(), "g2".into()],
                patient: vec!["Patient/explicit".into()],
                ..Default::default()
            },
        ),
        (
            "limit-zero",
            ViewFilters {
                limit: Some(0),
                ..Default::default()
            },
        ),
        (
            "limit-fifty",
            ViewFilters {
                limit: Some(50),
                ..Default::default()
            },
        ),
        (
            "limit-oversized",
            ViewFilters {
                limit: Some(usize::MAX),
                ..Default::default()
            },
        ),
    ] {
        cases.push(Case {
            name,
            view: flat.clone(),
            filters,
        });
    }
    let mut constants = patient(
        "constant-bindings",
        json!([{"column":[
        {"path":"%text","name":"text","type":"string"},{"path":"%integer","name":"integer","type":"integer"},
        {"path":"%decimal","name":"decimal","type":"decimal"},{"path":"%boolean","name":"boolean","type":"boolean"},
        {"path":"'resources r'","name":"literal"}]}]),
    );
    constants.view["constant"] = json!([{"name":"unused","valueString":"unused"},{"name":"text","valueString":"0123"},
        {"name":"integer","valueInteger":42},{"name":"decimal","valueDecimal":1.25},{"name":"boolean","valueBoolean":true}]);
    constants.filters = ViewFilters {
        since: Some("2024-01-01T00:00:00Z".parse().unwrap()),
        patient: vec!["Patient/p1".into()],
        limit: Some(50),
        ..Default::default()
    };
    cases.push(constants);
    let mut union = cases
        .iter()
        .find(|c| c.name == "union")
        .unwrap()
        .view
        .clone();
    union["constant"] = json!([{"name":"unused","valueString":"unused"},{"name":"second","valueString":"B"},{"name":"first","valueString":"A"}]);
    union["select"] = json!([{"unionAll":[{"column":[{"path":"%first","name":"value"}]},{"column":[{"path":"%second","name":"value"}]}]}]);
    cases.push(Case {
        name: "union-runtime-filters",
        view: union,
        filters: ViewFilters {
            since: Some("2024-01-01T00:00:00Z".parse().unwrap()),
            group: vec!["g1".into()],
            patient: vec!["Patient/explicit".into()],
            limit: Some(50),
            ..Default::default()
        },
    });
    let repeat = cases
        .iter()
        .find(|c| c.name == "repeat")
        .unwrap()
        .view
        .clone();
    cases.push(Case {
        name: "repeat-runtime-filters",
        view: repeat,
        filters: ViewFilters {
            since: Some("2024-01-01T00:00:00Z".parse().unwrap()),
            patient: vec!["Patient/p1".into()],
            limit: Some(50),
            ..Default::default()
        },
    });
    cases.push(Case {
        name: "outside-compartment",
        view: view("Library", json!([{"column":[{"path":"id","name":"id"}]}])),
        filters: ViewFilters {
            patient: vec!["Patient/p1".into()],
            ..Default::default()
        },
    });
    cases.push(patient(
        "foreach-cells",
        json!([{"forEach":"contact","column":[
        {"path":"telecom","name":"objects","collection":true},
        {"path":"telecom.value","name":"values","collection":true},
        {"path":"telecom.value.join('|')","name":"joined","type":"string"},
        {"path":"telecom.where(system = 'phone').first().value","name":"pick","type":"string"}]}]),
    ));
    let mut anchors = patient(
        "literal-anchors",
        json!([{"column":[
        {"path":"'resources r'","name":"resource_literal"},
        {"path":"'r.tenant_id = ?1\n  AND r.resource_type = ?2\n  AND r.is_deleted = 0'","name":"sqlite_anchor"},
        {"path":"'r.tenant_id = $1\n  AND r.resource_type = $2\n  AND r.is_deleted = false'","name":"pg_anchor"}]}]),
    );
    anchors.filters = ViewFilters {
        since: Some("2024-01-01T00:00:00Z".parse().unwrap()),
        patient: vec!["Patient/p1".into()],
        ..Default::default()
    };
    cases.push(anchors);
    cases
}

fn binding(param: &RuntimeParam) -> Value {
    match param {
        RuntimeParam::Text(value) => json!({"type":"text","value":value}),
        RuntimeParam::TextList(value) => json!({"type":"text-list","value":value}),
        RuntimeParam::Timestamp(value) => json!({"type":"timestamp","value":value.to_rfc3339()}),
        RuntimeParam::Literal(literal) => match literal {
            LitValue::Null => json!({"type":"null","value":null}),
            LitValue::Bool(value) => json!({"type":"boolean","value":value}),
            LitValue::Int(value) => json!({"type":"integer","value":value}),
            LitValue::Decimal(value) => json!({"type":"decimal","value":value}),
            LitValue::Str(value) => json!({"type":"string","value":value}),
        },
    }
}

async fn check(dialect: SqlDialect, name: &str) {
    let directory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/sof");
    let update = std::env::var("HFS_UPDATE_SOF_GOLDEN").is_ok_and(|value| value == "1");
    if update {
        std::fs::create_dir_all(&directory).unwrap();
    }
    let cases = cases();
    assert_eq!(cases.len(), 26, "all golden cases must execute");
    for case in cases {
        let prepared=prepare_sql_run(&case.view,dialect,FhirVersion::R4,"golden-tenant",case.filters,|_| async {
            Ok(vec![json!({"resourceType":"Group","id":"g1","member":[
                {"entity":{"reference":"Patient/p2"}},{"entity":{"reference":"Practitioner/ignored"}},
                {"entity":{"reference":"Patient/p1"}}]}),json!({"resourceType":"Group","id":"g2","member":[
                    {"entity":{"reference":"Patient/p2"}}]})])
        }).await.expect("prepare golden run").expect("nonempty golden run");
        let metadata = json!({"bindings":prepared.params.iter().map(binding).collect::<Vec<_>>(),
            "columns":prepared.query.columns,"decodes":prepared.query.column_decodes.iter().map(|decode| format!("{decode:?}")).collect::<Vec<_>>(),
            "client_limit":prepared.client_limit});
        let actual = format!(
            "{}\n\n/* Runner bindings and output metadata:\n{}\n*/\n",
            prepared.query.sql,
            serde_json::to_string_pretty(&metadata).unwrap()
        );
        let path = directory.join(format!("{}.{name}.sql", case.name));
        if update {
            std::fs::write(&path, &actual).unwrap();
        }
        let expected = std::fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!(
                "{}: {error}; regenerate with HFS_UPDATE_SOF_GOLDEN=1",
                path.display()
            )
        });
        assert_eq!(
            actual,
            expected,
            "complete SQL/bindings golden {}",
            path.display()
        );
    }
}

#[tokio::test]
async fn sqlite_complete_sql_and_bindings() {
    check(SqlDialect::Sqlite, "sqlite").await;
}

#[tokio::test]
async fn postgres_complete_sql_and_bindings() {
    check(SqlDialect::Postgres, "postgres").await;
}
