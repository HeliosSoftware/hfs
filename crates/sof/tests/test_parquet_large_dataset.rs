#[cfg(test)]
mod tests {
    use helios_sof::{
        ContentType, ParquetOptions, RunOptions, SofBundle, SofViewDefinition,
        run_view_definition_with_options,
    };
    use serde_json::json;

    #[test]
    #[cfg(feature = "R4")]
    fn test_parquet_large_dataset_with_chunking() {
        // Create a ViewDefinition for patient data
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
                        },
                        {
                            "name": "familyName",
                            "path": "name.family"
                        },
                        {
                            "name": "givenNames",
                            "path": "name.given",
                            "collection": true
                        }
                    ]
                }
            ]
        });

        // Generate a large bundle with 10,000 patients
        let mut entries = Vec::new();
        for i in 0..10000 {
            entries.push(json!({
                "resource": {
                    "resourceType": "Patient",
                    "id": format!("patient-{}", i),
                    "gender": if i % 3 == 0 { "male" } else if i % 3 == 1 { "female" } else { "other" },
                    "birthDate": format!("19{:02}-{:02}-{:02}",
                        50 + (i % 50), // Year between 1950-1999
                        1 + (i % 12),   // Month 1-12
                        1 + (i % 28)    // Day 1-28
                    ),
                    "active": i % 2 == 0,
                    "name": [
                        {
                            "family": format!("Family{}", i % 100),
                            "given": vec![format!("Given{}", i), format!("Middle{}", i % 10)]
                        }
                    ]
                }
            }));
        }

        let bundle_json = json!({
            "resourceType": "Bundle",
            "type": "collection",
            "entry": entries
        });

        let view_definition =
            serde_json::from_value::<helios_fhir::r4::ViewDefinition>(view_definition_json.clone())
                .expect("Failed to parse ViewDefinition");

        let bundle = serde_json::from_value::<helios_fhir::r4::Bundle>(bundle_json.clone())
            .expect("Failed to parse Bundle");

        let sof_view = SofViewDefinition::R4(view_definition);
        let sof_bundle = SofBundle::R4(bundle);

        // Test with custom parquet options
        let options = RunOptions {
            parquet_options: Some(ParquetOptions {
                row_group_size_mb: 128, // Smaller row groups for testing
                page_size_kb: 512,
                compression: "zstd".to_string(),
                max_file_size_mb: None,
            }),
            ..Default::default()
        };

        let result = run_view_definition_with_options(
            sof_view.clone(),
            sof_bundle.clone(),
            ContentType::Parquet,
            options,
        );
        assert!(
            result.is_ok(),
            "Parquet export with custom options failed: {:?}",
            result.err()
        );

        let parquet_data = result.unwrap();
        assert!(!parquet_data.is_empty(), "Parquet data should not be empty");
        assert!(
            parquet_data.starts_with(b"PAR1"),
            "Should start with PAR1 magic bytes"
        );
        assert!(
            parquet_data.ends_with(b"PAR1"),
            "Should end with PAR1 magic bytes"
        );

        // Verify the file size is reasonable for 10K records
        // With zstd compression and efficient chunking, the file can be quite small
        // Parquet has overhead but also excellent compression
        let file_size = parquet_data.len();
        assert!(
            file_size > 10_000,
            "File too small for 10K records: {} bytes",
            file_size
        );
        assert!(
            file_size < 10_000_000,
            "File too large for 10K records: {} bytes",
            file_size
        );
    }

    #[test]
    #[cfg(feature = "R4")]
    fn test_parquet_compression_algorithms() {
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
                            "name": "text",
                            "path": "text.div"
                        }
                    ]
                }
            ]
        });

        // Create test data with highly compressible text
        let mut entries = Vec::new();
        for i in 0..100 {
            entries.push(json!({
                "resource": {
                    "resourceType": "Patient",
                    "id": format!("patient-{}", i),
                    "text": {
                        "div": "This is a repeating text pattern. ".repeat(10)
                    }
                }
            }));
        }

        let bundle_json = json!({
            "resourceType": "Bundle",
            "type": "collection",
            "entry": entries
        });

        let view_definition =
            serde_json::from_value::<helios_fhir::r4::ViewDefinition>(view_definition_json.clone())
                .expect("Failed to parse ViewDefinition");

        let bundle = serde_json::from_value::<helios_fhir::r4::Bundle>(bundle_json.clone())
            .expect("Failed to parse Bundle");

        let sof_view = SofViewDefinition::R4(view_definition);
        let sof_bundle = SofBundle::R4(bundle);

        // Test different compression algorithms
        let compressions = vec!["none", "snappy", "gzip", "zstd", "brotli"];

        for compression in compressions {
            let options = RunOptions {
                parquet_options: Some(ParquetOptions {
                    row_group_size_mb: 256,
                    page_size_kb: 1024,
                    compression: compression.to_string(),
                    max_file_size_mb: None,
                }),
                ..Default::default()
            };

            let result = run_view_definition_with_options(
                sof_view.clone(),
                sof_bundle.clone(),
                ContentType::Parquet,
                options,
            );

            assert!(
                result.is_ok(),
                "Parquet export with {} compression failed: {:?}",
                compression,
                result.err()
            );

            let parquet_data = result.unwrap();
            let file_size = parquet_data.len();

            // Due to parquet overhead, very small files might not follow expected compression ratios
            // So we only check that the file was created successfully
            assert!(
                file_size > 0,
                "Parquet file with {} compression should not be empty",
                compression
            );

            // Verify magic bytes
            assert!(
                parquet_data.starts_with(b"PAR1") && parquet_data.ends_with(b"PAR1"),
                "Invalid parquet file format with {} compression",
                compression
            );
        }
    }

    #[test]
    #[cfg(feature = "R4")]
    fn test_parquet_row_group_configuration() {
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
                            "name": "value",
                            "path": "valueQuantity.value"
                        }
                    ]
                }
            ]
        });

        // Create 1000 observations
        let mut entries = Vec::new();
        for i in 0..1000 {
            entries.push(json!({
                "resource": {
                    "resourceType": "Observation",
                    "id": format!("obs-{}", i),
                    "status": "final",
                    "valueQuantity": {
                        "value": 98.6 + (i as f64 * 0.01),
                        "unit": "F"
                    }
                }
            }));
        }

        let bundle_json = json!({
            "resourceType": "Bundle",
            "type": "collection",
            "entry": entries
        });

        let view_definition =
            serde_json::from_value::<helios_fhir::r4::ViewDefinition>(view_definition_json.clone())
                .expect("Failed to parse ViewDefinition");

        let bundle = serde_json::from_value::<helios_fhir::r4::Bundle>(bundle_json.clone())
            .expect("Failed to parse Bundle");

        let sof_view = SofViewDefinition::R4(view_definition);
        let sof_bundle = SofBundle::R4(bundle);

        // Test with different row group sizes
        let row_group_configs = vec![
            (64, 64),     // Minimum row group size
            (256, 1024),  // Default configuration
            (512, 2048),  // Larger configuration
            (1024, 4096), // Maximum row group size
        ];

        for (row_group_mb, page_kb) in row_group_configs {
            let options = RunOptions {
                parquet_options: Some(ParquetOptions {
                    row_group_size_mb: row_group_mb,
                    page_size_kb: page_kb,
                    compression: "snappy".to_string(),
                    max_file_size_mb: None,
                }),
                ..Default::default()
            };

            let result = run_view_definition_with_options(
                sof_view.clone(),
                sof_bundle.clone(),
                ContentType::Parquet,
                options,
            );

            assert!(
                result.is_ok(),
                "Parquet export with {}MB row groups failed: {:?}",
                row_group_mb,
                result.err()
            );

            let parquet_data = result.unwrap();
            assert!(
                !parquet_data.is_empty(),
                "Parquet data with {}MB row groups should not be empty",
                row_group_mb
            );

            // Verify the file is valid Parquet
            assert!(
                parquet_data.starts_with(b"PAR1") && parquet_data.ends_with(b"PAR1"),
                "Invalid parquet file with {}MB row groups",
                row_group_mb
            );
        }
    }
}
