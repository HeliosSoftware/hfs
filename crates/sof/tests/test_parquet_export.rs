#[cfg(test)]
mod tests {
    use helios_sof::{ContentType, SofBundle, SofViewDefinition, run_view_definition};
    use serde_json::json;

    #[test]
    #[cfg(feature = "R4")]
    fn test_parquet_export_basic() {
        let view_definition_json = json!({
            "resourceType": "ViewDefinition",
            "status": "active",
            "resource": "Patient",
            "select": [
                {
                    "column": [
                        {
                            "name": "id",
                            "path": "id"
                        },
                        {
                            "name": "gender",
                            "path": "gender"
                        },
                        {
                            "name": "birthDate",
                            "path": "birthDate"
                        },
                        {
                            "name": "active",
                            "path": "active"
                        }
                    ]
                }
            ]
        });

        let bundle_json = json!({
            "resourceType": "Bundle",
            "type": "collection",
            "entry": [
                {
                    "resource": {
                        "resourceType": "Patient",
                        "id": "patient-1",
                        "gender": "male",
                        "birthDate": "1990-01-01",
                        "active": true
                    }
                },
                {
                    "resource": {
                        "resourceType": "Patient",
                        "id": "patient-2",
                        "gender": "female",
                        "birthDate": "1985-05-15",
                        "active": false
                    }
                },
                {
                    "resource": {
                        "resourceType": "Patient",
                        "id": "patient-3",
                        "gender": "other",
                        "birthDate": "2000-12-31",
                        "active": true
                    }
                }
            ]
        });

        let view_definition =
            serde_json::from_value::<helios_fhir::r4::ViewDefinition>(view_definition_json.clone())
                .expect("Failed to parse ViewDefinition");

        let bundle = serde_json::from_value::<helios_fhir::r4::Bundle>(bundle_json.clone())
            .expect("Failed to parse Bundle");

        let sof_view = SofViewDefinition::R4(view_definition);
        let sof_bundle = SofBundle::R4(bundle);

        // Test Parquet export
        let result = run_view_definition(sof_view, sof_bundle, ContentType::Parquet);
        assert!(result.is_ok(), "Parquet export failed: {:?}", result.err());

        let parquet_data = result.unwrap();
        assert!(!parquet_data.is_empty(), "Parquet data should not be empty");

        // Verify Parquet magic bytes
        assert!(
            parquet_data.starts_with(b"PAR1"),
            "Should start with PAR1 magic bytes"
        );
        assert!(
            parquet_data.ends_with(b"PAR1"),
            "Should end with PAR1 magic bytes"
        );
    }

    #[test]
    #[cfg(feature = "R4")]
    fn test_parquet_export_with_arrays() {
        let view_definition_json = json!({
            "resourceType": "ViewDefinition",
            "status": "active",
            "resource": "Patient",
            "select": [
                {
                    "column": [
                        {
                            "name": "id",
                            "path": "id"
                        },
                        {
                            "name": "givenNames",
                            "path": "name.given",
                            "collection": true
                        },
                        {
                            "name": "telecomValues",
                            "path": "telecom.value",
                            "collection": true
                        }
                    ]
                }
            ]
        });

        let bundle_json = json!({
            "resourceType": "Bundle",
            "type": "collection",
            "entry": [
                {
                    "resource": {
                        "resourceType": "Patient",
                        "id": "patient-1",
                        "name": [
                            {
                                "given": ["John", "James"],
                                "family": "Doe"
                            }
                        ],
                        "telecom": [
                            {
                                "system": "phone",
                                "value": "555-1234"
                            },
                            {
                                "system": "email",
                                "value": "john@example.com"
                            }
                        ]
                    }
                }
            ]
        });

        let view_definition =
            serde_json::from_value::<helios_fhir::r4::ViewDefinition>(view_definition_json.clone())
                .expect("Failed to parse ViewDefinition");

        let bundle = serde_json::from_value::<helios_fhir::r4::Bundle>(bundle_json.clone())
            .expect("Failed to parse Bundle");

        let sof_view = SofViewDefinition::R4(view_definition);
        let sof_bundle = SofBundle::R4(bundle);

        let result = run_view_definition(sof_view, sof_bundle, ContentType::Parquet);
        assert!(
            result.is_ok(),
            "Parquet export with arrays failed: {:?}",
            result.err()
        );

        let parquet_data = result.unwrap();
        assert!(!parquet_data.is_empty(), "Parquet data should not be empty");
        assert!(
            parquet_data.starts_with(b"PAR1"),
            "Should start with PAR1 magic bytes"
        );
    }

    #[test]
    #[cfg(feature = "R4")]
    fn test_parquet_export_with_nulls() {
        let view_definition_json = json!({
            "resourceType": "ViewDefinition",
            "status": "active",
            "resource": "Patient",
            "select": [
                {
                    "column": [
                        {
                            "name": "id",
                            "path": "id"
                        },
                        {
                            "name": "gender",
                            "path": "gender"
                        },
                        {
                            "name": "birthDate",
                            "path": "birthDate"
                        },
                        {
                            "name": "deceasedBoolean",
                            "path": "deceasedBoolean"
                        }
                    ]
                }
            ]
        });

        let bundle_json = json!({
            "resourceType": "Bundle",
            "type": "collection",
            "entry": [
                {
                    "resource": {
                        "resourceType": "Patient",
                        "id": "patient-1",
                        "gender": "male"
                        // birthDate and deceasedBoolean are missing (null)
                    }
                },
                {
                    "resource": {
                        "resourceType": "Patient",
                        "id": "patient-2",
                        "birthDate": "1990-01-01",
                        "deceasedBoolean": false
                        // gender is missing (null)
                    }
                }
            ]
        });

        let view_definition =
            serde_json::from_value::<helios_fhir::r4::ViewDefinition>(view_definition_json.clone())
                .expect("Failed to parse ViewDefinition");

        let bundle = serde_json::from_value::<helios_fhir::r4::Bundle>(bundle_json.clone())
            .expect("Failed to parse Bundle");

        let sof_view = SofViewDefinition::R4(view_definition);
        let sof_bundle = SofBundle::R4(bundle);

        let result = run_view_definition(sof_view, sof_bundle, ContentType::Parquet);
        assert!(
            result.is_ok(),
            "Parquet export with nulls failed: {:?}",
            result.err()
        );

        let parquet_data = result.unwrap();
        assert!(!parquet_data.is_empty(), "Parquet data should not be empty");
        assert!(
            parquet_data.starts_with(b"PAR1"),
            "Should start with PAR1 magic bytes"
        );
    }

    #[test]
    #[cfg(feature = "R4")]
    fn test_parquet_export_mixed_types() {
        let view_definition_json = json!({
            "resourceType": "ViewDefinition",
            "status": "active",
            "resource": "Observation",
            "select": [
                {
                    "column": [
                        {
                            "name": "id",
                            "path": "id"
                        },
                        {
                            "name": "status",
                            "path": "status"
                        },
                        {
                            "name": "valueQuantity",
                            "path": "valueQuantity.value"
                        },
                        {
                            "name": "valueBoolean",
                            "path": "valueBoolean"
                        }
                    ]
                }
            ]
        });

        let bundle_json = json!({
            "resourceType": "Bundle",
            "type": "collection",
            "entry": [
                {
                    "resource": {
                        "resourceType": "Observation",
                        "id": "obs-1",
                        "status": "final",
                        "valueQuantity": {
                            "value": 98.6,
                            "unit": "F"
                        }
                    }
                },
                {
                    "resource": {
                        "resourceType": "Observation",
                        "id": "obs-2",
                        "status": "preliminary",
                        "valueBoolean": true
                    }
                }
            ]
        });

        let view_definition =
            serde_json::from_value::<helios_fhir::r4::ViewDefinition>(view_definition_json.clone())
                .expect("Failed to parse ViewDefinition");

        let bundle = serde_json::from_value::<helios_fhir::r4::Bundle>(bundle_json.clone())
            .expect("Failed to parse Bundle");

        let sof_view = SofViewDefinition::R4(view_definition);
        let sof_bundle = SofBundle::R4(bundle);

        let result = run_view_definition(sof_view, sof_bundle, ContentType::Parquet);
        assert!(
            result.is_ok(),
            "Parquet export with mixed types failed: {:?}",
            result.err()
        );

        let parquet_data = result.unwrap();
        assert!(!parquet_data.is_empty(), "Parquet data should not be empty");
        assert!(
            parquet_data.starts_with(b"PAR1"),
            "Should start with PAR1 magic bytes"
        );
    }
}
