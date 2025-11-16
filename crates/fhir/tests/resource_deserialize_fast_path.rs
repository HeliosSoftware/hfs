use helios_fhir::r4::Resource;
use helios_serde::json;

#[test]
fn resource_deserializes_when_resource_type_first() {
    let json = r#"{"resourceType":"Patient","id":"fast-path"}"#;
    let resource: Resource = json::from_str(json).expect("fast-path parse failed");

    match resource {
        Resource::Patient(patient) => {
            let id_value = patient.id.and_then(|id| id.value);
            assert_eq!(id_value.as_deref(), Some("fast-path"));
        }
        other => panic!("expected Patient resource, got {:?}", other),
    }
}

#[test]
fn resource_deserializes_when_resource_type_not_first() {
    let json = r#"{"id":"slow-path","resourceType":"Patient"}"#;
    let resource: Resource = json::from_str(json).expect("slow-path parse failed");

    match resource {
        Resource::Patient(patient) => {
            let id_value = patient.id.and_then(|id| id.value);
            assert_eq!(id_value.as_deref(), Some("slow-path"));
        }
        other => panic!("expected Patient resource, got {:?}", other),
    }
}
