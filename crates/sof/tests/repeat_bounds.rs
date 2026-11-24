//! A `repeat` traversal is bounded by depth and by node count. A repeat path
//! that returns its own input (`$this`, the resource's type name, `%resource`)
//! used to recurse until the stack overflowed, which aborts the process; one
//! path listed twice doubled the node list at every level of the data.

use helios_sof::{
    ContentType, MAX_REPEAT_DEPTH, MAX_REPEAT_NODES, SofBundle, SofError, SofViewDefinition,
    run_view_definition,
};
use serde_json::{Value, json};

fn bundle(resources: &[Value]) -> SofBundle {
    let bundle = json!({
        "resourceType": "Bundle",
        "type": "collection",
        "entry": resources.iter().map(|r| json!({"resource": r})).collect::<Vec<_>>()
    });
    SofBundle::R4(serde_json::from_value(bundle).expect("bundle"))
}

fn repeat_view(resource: &str, paths: &[&str]) -> SofViewDefinition {
    let view = json!({
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": resource,
        "select": [{
            "repeat": paths,
            "column": [{"name": "link", "path": "linkId"}]
        }]
    });
    SofViewDefinition::R4(serde_json::from_value(view).expect("view"))
}

fn run(resource: &str, paths: &[&str], data: &[Value]) -> Result<Vec<u8>, SofError> {
    run_view_definition(
        repeat_view(resource, paths),
        bundle(data),
        ContentType::Json,
    )
}

fn expect_bounded(result: Result<Vec<u8>, SofError>, limit: &str) {
    match result {
        Err(SofError::InvalidViewDefinition(message)) => {
            assert!(message.contains(limit), "{message}")
        }
        other => panic!("expected the traversal to be refused, got {other:?}"),
    }
}

/// A Questionnaire whose items nest `depth` levels.
fn nested_questionnaire(depth: usize) -> Value {
    let mut item = json!({"linkId": format!("q{depth}"), "type": "display"});
    for level in (1..depth).rev() {
        item = json!({"linkId": format!("q{level}"), "type": "group", "item": [item]});
    }
    json!({"resourceType": "Questionnaire", "id": "q", "status": "active", "item": [item]})
}

#[test]
fn a_path_returning_its_input_is_refused_at_the_depth_limit() {
    let patient = json!({"resourceType": "Patient", "id": "p"});
    for path in ["$this", "Patient", "%resource"] {
        expect_bounded(
            run("Patient", &[path], std::slice::from_ref(&patient)),
            &format!("deeper than {MAX_REPEAT_DEPTH}"),
        );
    }
}

#[test]
fn a_path_listed_twice_is_refused_at_the_node_limit() {
    expect_bounded(
        run(
            "Questionnaire",
            &["item", "item"],
            &[nested_questionnaire(16)],
        ),
        &format!("more than {MAX_REPEAT_NODES}"),
    );
}

#[test]
fn a_descending_traversal_still_collects_every_node() {
    let out = run("Questionnaire", &["item"], &[nested_questionnaire(40)]).expect("rows");
    let rows: Vec<Value> = serde_json::from_slice(&out).expect("json rows");
    assert_eq!(rows.len(), 40, "{rows:?}");
    assert_eq!(rows[0]["link"], "q1");
    assert_eq!(rows[39]["link"], "q40");
}
