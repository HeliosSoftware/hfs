//! Coding property round trips through the same HTTP contract on both stores.

mod common;

use axum::http::StatusCode;
use serde_json::{Value, json};

enum PropertyApp {
    #[cfg(feature = "sqlite")]
    Sqlite(common::TestApp),
    #[cfg(feature = "postgres")]
    Postgres(common::TestAppPg),
}

impl PropertyApp {
    async fn post(&self, path: &str, body: Value) -> Value {
        let body = body.to_string();
        let (status, response) = match self {
            #[cfg(feature = "sqlite")]
            Self::Sqlite(app) => app.post_fhir(path, body).await,
            #[cfg(feature = "postgres")]
            Self::Postgres(app) => app.post_fhir(path, body).await,
        };
        assert_eq!(status, StatusCode::OK, "{path}: {response}");
        response
    }
}

fn complete_coding() -> Value {
    json!({
        "id": "target-coding",
        "system": "http://example.org/target",
        "version": "2026",
        "code": "TARGET",
        "display": "Target concept",
        "userSelected": false,
        "extension": [{"url": "http://example.org/note", "valueString": "detail"}],
        "_display": {"extension": [{
            "url": "http://example.org/display-note", "valueString": "original"
        }]}
    })
}

fn assert_properties(actual: &[Value], expected: &[Value]) {
    assert_eq!(actual.len(), expected.len(), "properties: {actual:?}");
    let mut unmatched = actual.to_vec();
    for property in expected {
        let position = unmatched
            .iter()
            .position(|actual| actual == property)
            .unwrap_or_else(|| panic!("missing {property} in {actual:?}"));
        unmatched.remove(position);
    }
}

fn lookup_properties(response: &Value) -> Vec<Value> {
    response["parameter"]
        .as_array()
        .expect("lookup Parameters")
        .iter()
        .filter(|parameter| parameter["name"] == "property")
        .map(|parameter| {
            let parts = parameter["part"].as_array().expect("property parts");
            let code = parts
                .iter()
                .find(|part| part["name"] == "code")
                .expect("property code")["valueCode"]
                .clone();
            let mut property = parts
                .iter()
                .find(|part| part["name"] == "value")
                .expect("property value")
                .as_object()
                .unwrap()
                .clone();
            property.remove("name");
            property.insert("code".into(), code);
            Value::Object(property)
        })
        .collect()
}

fn assert_expansion_properties(response: &Value, expected: &[Value]) {
    let contains = response["expansion"]["contains"]
        .as_array()
        .expect("expansion contains");
    assert_eq!(contains.len(), 1, "{response}");
    assert_eq!(contains[0]["code"], "SOURCE");
    if (cfg!(feature = "R4") || cfg!(feature = "R4B"))
        && !cfg!(feature = "R5")
        && !cfg!(feature = "R6")
    {
        const PROPERTY_URL: &str = "http://hl7.org/fhir/5.0/StructureDefinition/extension-ValueSet.expansion.contains.property";
        assert!(contains[0].get("property").is_none(), "{response}");
        let actual: Vec<_> = contains[0]["extension"]
            .as_array()
            .expect("R4 concept property extensions")
            .iter()
            .filter(|extension| extension["url"] == PROPERTY_URL)
            .cloned()
            .collect();
        let expected: Vec<_> = expected
            .iter()
            .map(|property| {
                let mut value = property.as_object().unwrap().clone();
                let code = value.remove("code").unwrap();
                value.insert("url".into(), json!("value"));
                json!({"url": PROPERTY_URL, "extension": [
                    {"url": "code", "valueCode": code}, Value::Object(value)
                ]})
            })
            .collect();
        assert_properties(&actual, &expected);
    } else {
        assert_properties(
            contains[0]["property"]
                .as_array()
                .expect("concept properties"),
            expected,
        );
    }
}

async fn assert_coding_contract(app: PropertyApp) {
    // PostgreSQL apps share a database; each contract uses its own canonicals.
    let root = format!(
        "http://example.org/coding-properties/{}",
        uuid::Uuid::new_v4()
    );
    let system = format!("{root}/base");
    let value_set = format!("{root}/value-set");
    let supplement = format!("{root}/supplement");
    let coding = complete_coding();
    let bundle = json!({
        "resourceType": "Bundle", "type": "collection",
        "entry": [
            {"resource": {
                "resourceType": "CodeSystem", "url": system,
                "status": "active", "content": "complete",
                "property": [
                    {"code": "maps-to", "type": "Coding"},
                    {"code": "conflicting", "type": "string"},
                    {"code": "declared-only", "type": "Coding"},
                    {"code": "inactive", "type": "boolean"},
                    {"code": "rank", "type": "integer"},
                    {"code": "weight", "type": "decimal"},
                    {"code": "label", "type": "string"},
                    {"code": "category", "type": "code"},
                    {"code": "effective", "type": "dateTime"}
                ],
                "concept": [{"code": "SOURCE", "property": [
                    {"code": "maps-to", "valueCoding": coding},
                    {"code": "maps-to", "valueCoding": coding},
                    {"code": "maps-to", "valueString": coding.to_string()},
                    {"code": "conflicting", "valueCoding": coding},
                    {"code": "undeclared", "valueCoding": coding},
                    {"code": "declared-only", "valueString": "Plain text"},
                    {"code": "inactive", "valueBoolean": false},
                    {"code": "rank", "valueInteger": 0},
                    {"code": "weight", "valueDecimal": 2.5},
                    {"code": "label", "valueString": "Example"},
                    {"code": "category", "valueCode": "example"},
                    {"code": "effective", "valueDateTime": "2026-01-01T00:00:00Z"},
                    {"code": "invalid-null", "valueCoding": null},
                    {"code": "invalid-scalar", "valueCoding": "TARGET"}
                ]}]
            }},
            {"resource": {
                "resourceType": "CodeSystem", "url": supplement,
                "status": "active", "content": "supplement", "supplements": system,
                "property": [
                    {"code": "annotation", "type": "string"},
                    {"code": "supp-enabled", "type": "boolean"},
                    {"code": "supp-rank", "type": "integer"},
                    {"code": "supp-label", "type": "string"}
                ],
                "concept": [{"code": "SOURCE", "property": [
                    {"code": "annotation", "valueCoding": coding},
                    {"code": "annotation", "valueString": "Plain text"},
                    {"code": "supp-undeclared", "valueCoding": coding},
                    {"code": "supp-enabled", "valueBoolean": false},
                    {"code": "supp-rank", "valueInteger": 0},
                    {"code": "supp-label", "valueCode": "LABEL"}
                ]}]
            }},
            {"resource": {
                "resourceType": "ValueSet", "url": value_set, "status": "active",
                "compose": {"include": [{"system": system}]}
            }}
        ]
    });
    let imported = app.post("/import", bundle).await;
    assert_eq!(imported["code_systems"], 2, "{imported}");
    assert_eq!(imported["value_sets"], 1, "{imported}");
    assert_eq!(imported["concepts"], 2, "{imported}");
    assert_eq!(imported["concept_maps"], 0, "{imported}");
    assert!(
        imported["errors"].is_null() || imported["errors"].as_array().is_some_and(Vec::is_empty),
        "import errors: {imported}"
    );

    let base_expected = vec![
        json!({"code": "maps-to", "valueCoding": coding}),
        json!({"code": "maps-to", "valueString": coding.to_string()}),
        json!({"code": "conflicting", "valueCoding": coding}),
        json!({"code": "undeclared", "valueCoding": coding}),
        json!({"code": "declared-only", "valueString": "Plain text"}),
        json!({"code": "inactive", "valueBoolean": false}),
        json!({"code": "rank", "valueInteger": 0}),
        json!({"code": "weight", "valueDecimal": 2.5}),
        json!({"code": "label", "valueString": "Example"}),
        json!({"code": "category", "valueCode": "example"}),
        json!({"code": "effective", "valueDateTime": "2026-01-01T00:00:00Z"}),
    ];
    let lookup_params = vec![
        json!({"name": "system", "valueUri": system}),
        json!({"name": "code", "valueCode": "SOURCE"}),
    ];
    // Lookup retains duplicate properties; expansion deduplicates identical ones.
    let mut lookup_expected = base_expected.clone();
    lookup_expected.push(json!({"code": "maps-to", "valueCoding": coding}));
    for filtered in [false, true] {
        let mut parameters = lookup_params.clone();
        let expected = if filtered {
            parameters.push(json!({"name": "property", "valueCode": "maps-to"}));
            lookup_expected
                .iter()
                .filter(|property| property["code"] == "maps-to")
                .cloned()
                .collect::<Vec<_>>()
        } else {
            lookup_expected.clone()
        };
        let response = app
            .post(
                "/CodeSystem/$lookup",
                json!({
                    "resourceType": "Parameters", "parameter": parameters
                }),
            )
            .await;
        assert_properties(&lookup_properties(&response), &expected);
    }

    let mut expand_params = vec![json!({"name": "url", "valueUri": value_set})];
    for code in [
        "maps-to",
        "conflicting",
        "undeclared",
        "declared-only",
        "inactive",
        "rank",
        "weight",
        "label",
        "category",
        "effective",
    ] {
        expand_params.push(json!({"name": "property", "valueCode": code}));
    }
    let expanded = app
        .post(
            "/ValueSet/$expand",
            json!({
                "resourceType": "Parameters", "parameter": expand_params
            }),
        )
        .await;
    assert_expansion_properties(&expanded, &base_expected);

    let supplement_lookup = vec![
        json!({"code": "annotation", "valueCoding": coding}),
        json!({"code": "annotation", "valueString": "Plain text"}),
        json!({"code": "supp-undeclared", "valueCoding": coding}),
        json!({"code": "supp-enabled", "valueString": "false"}),
        json!({"code": "supp-rank", "valueString": "0"}),
        json!({"code": "supp-label", "valueString": "LABEL"}),
    ];
    // No filter and wildcard both select every supplement property. Named
    // filters exercise the separate PostgreSQL query with cp.property = ANY.
    for filter in [None, Some("*"), Some("annotation")] {
        let mut parameters = lookup_params.clone();
        parameters.push(json!({"name": "useSupplement", "valueCanonical": supplement}));
        if let Some(property) = filter {
            parameters.push(json!({"name": "property", "valueCode": property}));
        }
        let expected = if filter == Some("annotation") {
            supplement_lookup[..2].to_vec()
        } else {
            let mut expected = lookup_expected.clone();
            expected.extend(supplement_lookup.clone());
            expected
        };
        let response = app
            .post(
                "/CodeSystem/$lookup",
                json!({
                    "resourceType": "Parameters", "parameter": parameters
                }),
            )
            .await;
        assert_properties(&lookup_properties(&response), &expected);
    }

    let mut parameters = vec![
        json!({"name": "url", "valueUri": value_set}),
        json!({"name": "useSupplement", "valueCanonical": supplement}),
    ];
    for code in [
        "annotation",
        "supp-undeclared",
        "supp-enabled",
        "supp-rank",
        "supp-label",
    ] {
        parameters.push(json!({"name": "property", "valueCode": code}));
    }
    let expanded = app
        .post(
            "/ValueSet/$expand",
            json!({
                "resourceType": "Parameters", "parameter": parameters
            }),
        )
        .await;
    assert_expansion_properties(
        &expanded,
        &[
            json!({"code": "annotation", "valueCoding": coding}),
            json!({"code": "annotation", "valueString": "Plain text"}),
            json!({"code": "supp-undeclared", "valueCoding": coding}),
            json!({"code": "supp-enabled", "valueBoolean": false}),
            json!({"code": "supp-rank", "valueInteger": 0}),
            json!({"code": "supp-label", "valueString": "LABEL"}),
        ],
    );
}

#[cfg(feature = "sqlite")]
#[tokio::test]
async fn coding_properties_round_trip_sqlite() {
    assert_coding_contract(PropertyApp::Sqlite(common::TestApp::new())).await;
}

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread")]
async fn coding_properties_round_trip_postgres() {
    assert_coding_contract(PropertyApp::Postgres(common::TestAppPg::new().await)).await;
}
