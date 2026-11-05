#[cfg(test)]
mod tests {
    use helios_sof::{
        ContentType, ParquetOptions, RunOptions, SofBundle, SofViewDefinition,
        run_view_definition_with_options,
    };
    use serde_json::json;

    #[test]
    #[cfg(feature = "R4")]
    fn test_parquet_with_custom_options() {
        // Create a simple ViewDefinition
        let view_definition_json = json!({
            "resourceType": "ViewDefinition",
            "status": "active",
            "resource": "Patient",
            "select": [{
                "column": [
                    {"name": "id", "path": "id"},
                    {"name": "gender", "path": "gender"}
                ]
            }]
        });

        // Create a small bundle with patients
        let bundle_json = json!({
            "resourceType": "Bundle",
            "type": "collection",
            "entry": [
                {
                    "resource": {
                        "resourceType": "Patient",
                        "id": "patient-1",
                        "gender": "male"
                    }
                },
                {
                    "resource": {
                        "resourceType": "Patient",
                        "id": "patient-2",
                        "gender": "female"
                    }
                }
            ]
        });

        let view_definition =
            serde_json::from_value::<helios_fhir::r4::ViewDefinition>(view_definition_json)
                .expect("Failed to parse ViewDefinition");
        let bundle = serde_json::from_value::<helios_fhir::r4::Bundle>(bundle_json)
            .expect("Failed to parse Bundle");

        let sof_view = SofViewDefinition::R4(view_definition);
        let sof_bundle = SofBundle::R4(bundle);

        // Test with custom Parquet options
        let options = RunOptions {
            parquet_options: Some(ParquetOptions {
                row_group_size_mb: 128,
                page_size_kb: 512,
                compression: "zstd".to_string(),
                max_file_size_mb: Some(100), // 100 MB max file size
            }),
            ..Default::default()
        };

        let result =
            run_view_definition_with_options(sof_view, sof_bundle, ContentType::Parquet, options);

        assert!(
            result.is_ok(),
            "Parquet generation with options failed: {:?}",
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
    }

    #[test]
    #[cfg(feature = "R4")]
    fn test_parquet_multi_file_generation() {
        use bytes::Bytes;
        use helios_sof::{format_parquet_multi_file, process_view_definition};
        use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

        // Create ViewDefinition and a larger dataset
        let view_definition_json = json!({
            "resourceType": "ViewDefinition",
            "status": "active",
            "resource": "Patient",
            "select": [{
                "column": [
                    {"name": "id", "path": "id"},
                    {"name": "text", "path": "text.div"}
                ]
            }]
        });

        // Enough poorly compressible rows to span several record batches, so the
        // writer has to roll over to further files at the 1 MB limit.
        let mut entries = Vec::new();
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        for i in 0..20_000 {
            let text: String = (0..300)
                .map(|_| {
                    state = state
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                    char::from(b'a' + ((state >> 33) % 26) as u8)
                })
                .collect();
            entries.push(json!({
                "resource": {
                    "resourceType": "Patient",
                    "id": format!("patient-{}", i),
                    "text": {
                        "div": text
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
            serde_json::from_value::<helios_fhir::r4::ViewDefinition>(view_definition_json)
                .expect("Failed to parse ViewDefinition");
        let bundle = serde_json::from_value::<helios_fhir::r4::Bundle>(bundle_json)
            .expect("Failed to parse Bundle");

        let sof_view = SofViewDefinition::R4(view_definition);
        let sof_bundle = SofBundle::R4(bundle);

        // Process the ViewDefinition
        let processed_result = process_view_definition(sof_view, sof_bundle)
            .expect("Failed to process ViewDefinition");

        // Test multi-file generation with a very small max file size to force splitting
        let parquet_options = ParquetOptions {
            row_group_size_mb: 64,
            page_size_kb: 256,
            compression: "snappy".to_string(),
            max_file_size_mb: Some(1), // Very small to potentially trigger multi-file
        };

        // Generate with a 1MB max file size
        let result = format_parquet_multi_file(
            processed_result,
            Some(&parquet_options),
            1024 * 1024, // 1 MB
        );

        assert!(
            result.is_ok(),
            "Multi-file Parquet generation failed: {:?}",
            result.err()
        );

        let file_buffers = result.unwrap();
        assert!(
            file_buffers.len() > 1,
            "Expected the 1 MB limit to split the output, got {} file(s)",
            file_buffers.len()
        );

        // Every row must land in exactly one file.
        let mut total_rows = 0;
        for buffer in &file_buffers {
            let builder = ParquetRecordBatchReaderBuilder::try_new(Bytes::from(buffer.clone()))
                .expect("each file should be readable Parquet");
            total_rows += builder.metadata().file_metadata().num_rows() as usize;
        }
        assert_eq!(total_rows, 20_000, "rows must be preserved across files");

        // Verify each file is valid Parquet
        for buffer in &file_buffers {
            assert!(
                buffer.starts_with(b"PAR1"),
                "Each file should start with PAR1"
            );
            assert!(buffer.ends_with(b"PAR1"), "Each file should end with PAR1");
        }
    }
}
