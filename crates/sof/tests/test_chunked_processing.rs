//! Tests for chunked NDJSON processing functionality.
//!
//! These tests verify that the streaming/chunked processing produces
//! the same results as batch processing while using bounded memory.

use helios_sof::{
    ChunkConfig, ContentType, NdjsonChunkReader, PreparedViewDefinition, ResourceChunk,
    SofViewDefinition, process_ndjson_chunked,
};
use std::io::{BufReader, Cursor, Write};
use tempfile::NamedTempFile;

/// Helper to create a test ViewDefinition that extracts Patient id and gender
#[cfg(feature = "R4")]
fn create_patient_view_definition() -> SofViewDefinition {
    let view_json = serde_json::json!({
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
    let view_def: helios_fhir::r4::ViewDefinition = serde_json::from_value(view_json).unwrap();
    SofViewDefinition::R4(view_def)
}

/// Test basic NdjsonChunkReader functionality
#[test]
#[cfg(feature = "R4")]
fn test_chunk_reader_basic() {
    let ndjson = r#"{"resourceType": "Patient", "id": "p1", "gender": "male"}
{"resourceType": "Patient", "id": "p2", "gender": "female"}
{"resourceType": "Patient", "id": "p3", "gender": "other"}"#;

    let reader = BufReader::new(Cursor::new(ndjson));
    let config = ChunkConfig {
        chunk_size: 10, // Large enough to read all in one chunk
        skip_invalid_lines: false,
    };

    let mut chunk_reader = NdjsonChunkReader::new(reader, config);

    // Should get one chunk with all 3 resources
    let chunk = chunk_reader.next().unwrap().unwrap();
    assert_eq!(chunk.resources.len(), 3);
    assert_eq!(chunk.chunk_index, 0);
    assert!(chunk.is_last);

    // No more chunks
    assert!(chunk_reader.next().is_none());
}

/// Test chunked reading with small chunk size
#[test]
#[cfg(feature = "R4")]
fn test_chunk_reader_multiple_chunks() {
    let ndjson = r#"{"resourceType": "Patient", "id": "p1"}
{"resourceType": "Patient", "id": "p2"}
{"resourceType": "Patient", "id": "p3"}
{"resourceType": "Patient", "id": "p4"}
{"resourceType": "Patient", "id": "p5"}"#;

    let reader = BufReader::new(Cursor::new(ndjson));
    let config = ChunkConfig {
        chunk_size: 2,
        skip_invalid_lines: false,
    };

    let mut chunk_reader = NdjsonChunkReader::new(reader, config);

    // First chunk: 2 resources
    let chunk1 = chunk_reader.next().unwrap().unwrap();
    assert_eq!(chunk1.resources.len(), 2);
    assert_eq!(chunk1.chunk_index, 0);
    assert!(!chunk1.is_last);

    // Second chunk: 2 resources
    let chunk2 = chunk_reader.next().unwrap().unwrap();
    assert_eq!(chunk2.resources.len(), 2);
    assert_eq!(chunk2.chunk_index, 1);
    assert!(!chunk2.is_last);

    // Third chunk: 1 resource (last)
    let chunk3 = chunk_reader.next().unwrap().unwrap();
    assert_eq!(chunk3.resources.len(), 1);
    assert_eq!(chunk3.chunk_index, 2);
    assert!(chunk3.is_last);

    // No more chunks
    assert!(chunk_reader.next().is_none());
}

/// Test chunk reader with empty lines
#[test]
#[cfg(feature = "R4")]
fn test_chunk_reader_empty_lines() {
    let ndjson = r#"{"resourceType": "Patient", "id": "p1"}

{"resourceType": "Patient", "id": "p2"}

{"resourceType": "Patient", "id": "p3"}"#;

    let reader = BufReader::new(Cursor::new(ndjson));
    let config = ChunkConfig::default();

    let mut chunk_reader = NdjsonChunkReader::new(reader, config);

    // Should get all 3 resources, empty lines ignored
    let chunk = chunk_reader.next().unwrap().unwrap();
    assert_eq!(chunk.resources.len(), 3);
}

/// Test chunk reader with resource type filter
#[test]
#[cfg(feature = "R4")]
fn test_chunk_reader_resource_type_filter() {
    let ndjson = r#"{"resourceType": "Patient", "id": "p1"}
{"resourceType": "Observation", "id": "obs1", "status": "final", "code": {"text": "Test"}}
{"resourceType": "Patient", "id": "p2"}
{"resourceType": "Condition", "id": "cond1", "clinicalStatus": {"text": "active"}, "code": {"text": "Test"}}
{"resourceType": "Patient", "id": "p3"}"#;

    let reader = BufReader::new(Cursor::new(ndjson));
    let config = ChunkConfig::default();

    let mut chunk_reader = NdjsonChunkReader::new(reader, config)
        .with_resource_type_filter(Some("Patient".to_string()));

    // Should only get 3 Patient resources
    let chunk = chunk_reader.next().unwrap().unwrap();
    assert_eq!(chunk.resources.len(), 3);

    // Verify all are Patient resources
    for resource in &chunk.resources {
        assert_eq!(resource["resourceType"], "Patient");
    }
}

/// Test skip_invalid_lines option
#[test]
#[cfg(feature = "R4")]
fn test_chunk_reader_skip_invalid() {
    let ndjson = r#"{"resourceType": "Patient", "id": "p1"}
{invalid json line}
{"resourceType": "Patient", "id": "p2"}
not json at all
{"resourceType": "Patient", "id": "p3"}"#;

    let reader = BufReader::new(Cursor::new(ndjson));
    let config = ChunkConfig {
        chunk_size: 1000,
        skip_invalid_lines: true, // Skip invalid lines
    };

    let mut chunk_reader = NdjsonChunkReader::new(reader, config);

    // Should get 3 valid resources
    let chunk = chunk_reader.next().unwrap().unwrap();
    assert_eq!(chunk.resources.len(), 3);
}

/// Test error on invalid JSON when skip_invalid_lines is false
#[test]
#[cfg(feature = "R4")]
fn test_chunk_reader_error_on_invalid() {
    let ndjson = r#"{"resourceType": "Patient", "id": "p1"}
{invalid json line}
{"resourceType": "Patient", "id": "p2"}"#;

    let reader = BufReader::new(Cursor::new(ndjson));
    let config = ChunkConfig {
        chunk_size: 1000,
        skip_invalid_lines: false, // Don't skip - should error
    };

    let mut chunk_reader = NdjsonChunkReader::new(reader, config);

    // Should get an error
    let result = chunk_reader.next().unwrap();
    assert!(result.is_err());
}

/// Test PreparedViewDefinition creation and column extraction
#[test]
#[cfg(feature = "R4")]
fn test_prepared_view_definition() {
    let view_def = create_patient_view_definition();
    let prepared = PreparedViewDefinition::new(view_def).unwrap();

    assert_eq!(prepared.target_resource_type(), "Patient");
    assert_eq!(prepared.columns(), &["id", "gender"]);
}

/// Test processing a chunk through PreparedViewDefinition
#[test]
#[cfg(feature = "R4")]
fn test_process_chunk() {
    let view_def = create_patient_view_definition();
    let prepared = PreparedViewDefinition::new(view_def).unwrap();

    // Create a resource chunk
    let resources = vec![
        serde_json::json!({"resourceType": "Patient", "id": "p1", "gender": "male"}),
        serde_json::json!({"resourceType": "Patient", "id": "p2", "gender": "female"}),
    ];

    let chunk = ResourceChunk {
        resources,
        chunk_index: 0,
        is_last: true,
    };

    let result = prepared.process_chunk(chunk).unwrap();

    assert_eq!(result.columns, vec!["id", "gender"]);
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.chunk_index, 0);
    assert!(result.is_last);

    // Check first row values
    assert_eq!(result.rows[0].values[0], Some(serde_json::json!("p1")));
    assert_eq!(result.rows[0].values[1], Some(serde_json::json!("male")));
}

/// Test full chunked processing pipeline to CSV
#[test]
#[cfg(feature = "R4")]
fn test_process_ndjson_chunked_csv() {
    let ndjson = r#"{"resourceType": "Patient", "id": "p1", "gender": "male"}
{"resourceType": "Patient", "id": "p2", "gender": "female"}
{"resourceType": "Patient", "id": "p3", "gender": "other"}"#;

    let view_def = create_patient_view_definition();
    let input = BufReader::new(Cursor::new(ndjson));
    let mut output = Vec::new();

    let config = ChunkConfig {
        chunk_size: 2,
        skip_invalid_lines: false,
    };

    let stats = process_ndjson_chunked(
        view_def,
        input,
        &mut output,
        ContentType::CsvWithHeader,
        config,
    )
    .unwrap();

    // Verify stats
    assert_eq!(stats.total_lines_read, 3);
    assert_eq!(stats.resources_processed, 3);
    assert_eq!(stats.output_rows, 3);
    assert_eq!(stats.skipped_lines, 0);
    assert_eq!(stats.chunks_processed, 2); // 2 resources + 1 resource

    // Verify output
    let output_str = String::from_utf8(output).unwrap();
    assert!(output_str.starts_with("id,gender\n"));
    assert!(output_str.contains("p1,male"));
    assert!(output_str.contains("p2,female"));
    assert!(output_str.contains("p3,other"));
}

/// #1569: a resource without a value for a column yields an empty CSV cell in
/// the chunked path, the same as the buffered one, not the text null.
#[test]
#[cfg(feature = "R4")]
fn test_process_ndjson_chunked_csv_writes_a_missing_value_as_an_empty_cell() {
    let ndjson = r#"{"resourceType": "Patient", "id": "p1", "gender": "male"}
{"resourceType": "Patient", "id": "p2"}"#;

    let view_def = create_patient_view_definition();
    let input = BufReader::new(Cursor::new(ndjson));
    let mut output = Vec::new();
    let config = ChunkConfig {
        chunk_size: 1,
        skip_invalid_lines: false,
    };
    process_ndjson_chunked(
        view_def,
        input,
        &mut output,
        ContentType::CsvWithHeader,
        config,
    )
    .unwrap();

    let output_str = String::from_utf8(output).unwrap();
    assert_eq!(
        output_str,
        "id,gender
p1,male
p2,
",
        "{output_str}"
    );
}

/// Test chunked processing to NDJSON output
#[test]
#[cfg(feature = "R4")]
fn test_process_ndjson_chunked_ndjson() {
    let ndjson = r#"{"resourceType": "Patient", "id": "p1", "gender": "male"}
{"resourceType": "Patient", "id": "p2", "gender": "female"}"#;

    let view_def = create_patient_view_definition();
    let input = BufReader::new(Cursor::new(ndjson));
    let mut output = Vec::new();

    let config = ChunkConfig::default();

    let stats =
        process_ndjson_chunked(view_def, input, &mut output, ContentType::NdJson, config).unwrap();

    assert_eq!(stats.output_rows, 2);

    // Verify output is valid NDJSON
    let output_str = String::from_utf8(output).unwrap();
    let lines: Vec<&str> = output_str.trim().lines().collect();
    assert_eq!(lines.len(), 2);

    // Parse each line as JSON
    let row1: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(row1["id"], "p1");
    assert_eq!(row1["gender"], "male");
}

/// Test that chunked processing produces same output as batch processing
#[test]
#[cfg(feature = "R4")]
fn test_chunked_vs_batch_equivalence() {
    use helios_sof::{SofBundle, run_view_definition};

    let ndjson = r#"{"resourceType": "Patient", "id": "p1", "gender": "male"}
{"resourceType": "Patient", "id": "p2", "gender": "female"}
{"resourceType": "Patient", "id": "p3", "gender": "other"}"#;

    // Create a bundle for batch processing
    let bundle_json = serde_json::json!({
        "resourceType": "Bundle",
        "type": "collection",
        "entry": [
            {"resource": {"resourceType": "Patient", "id": "p1", "gender": "male"}},
            {"resource": {"resourceType": "Patient", "id": "p2", "gender": "female"}},
            {"resource": {"resourceType": "Patient", "id": "p3", "gender": "other"}}
        ]
    });
    let bundle: helios_fhir::r4::Bundle = serde_json::from_value(bundle_json).unwrap();

    // Run batch processing
    let view_def = create_patient_view_definition();
    let batch_output = run_view_definition(
        view_def.clone(),
        SofBundle::R4(bundle),
        ContentType::CsvWithHeader,
    )
    .unwrap();

    // Run chunked processing
    let view_def = create_patient_view_definition();
    let input = BufReader::new(Cursor::new(ndjson));
    let mut chunked_output = Vec::new();

    process_ndjson_chunked(
        view_def,
        input,
        &mut chunked_output,
        ContentType::CsvWithHeader,
        ChunkConfig::default(),
    )
    .unwrap();

    // Compare outputs (they should be identical)
    assert_eq!(
        String::from_utf8(batch_output).unwrap(),
        String::from_utf8(chunked_output).unwrap()
    );
}

/// Test processing with file input/output
#[test]
#[cfg(feature = "R4")]
fn test_chunked_processing_file_io() {
    // Create input NDJSON file
    let ndjson = r#"{"resourceType": "Patient", "id": "p1", "gender": "male"}
{"resourceType": "Patient", "id": "p2", "gender": "female"}"#;

    let mut input_file = NamedTempFile::with_suffix(".ndjson").unwrap();
    input_file.write_all(ndjson.as_bytes()).unwrap();
    input_file.flush().unwrap();

    // Create output file
    let output_file = NamedTempFile::with_suffix(".csv").unwrap();

    // Process
    let view_def = create_patient_view_definition();
    let input = std::io::BufReader::new(std::fs::File::open(input_file.path()).unwrap());
    let output = std::fs::File::create(output_file.path()).unwrap();

    let stats = process_ndjson_chunked(
        view_def,
        input,
        output,
        ContentType::CsvWithHeader,
        ChunkConfig::default(),
    )
    .unwrap();

    assert_eq!(stats.output_rows, 2);

    // Verify output file contents
    let output_content = std::fs::read_to_string(output_file.path()).unwrap();
    assert!(output_content.contains("id,gender"));
    assert!(output_content.contains("p1,male"));
    assert!(output_content.contains("p2,female"));
}

/// Test empty input handling
#[test]
#[cfg(feature = "R4")]
fn test_chunked_empty_input() {
    let view_def = create_patient_view_definition();
    let input = BufReader::new(Cursor::new(""));
    let mut output = Vec::new();

    let stats = process_ndjson_chunked(
        view_def,
        input,
        &mut output,
        ContentType::CsvWithHeader,
        ChunkConfig::default(),
    )
    .unwrap();

    assert_eq!(stats.total_lines_read, 0);
    assert_eq!(stats.resources_processed, 0);
    assert_eq!(stats.output_rows, 0);
    assert_eq!(stats.chunks_processed, 0);

    // Should still have header
    let output_str = String::from_utf8(output).unwrap();
    assert_eq!(output_str.trim(), "id,gender");
}

/// Test processing with forEach in ViewDefinition
#[test]
#[cfg(feature = "R4")]
fn test_chunked_foreach() {
    let view_json = serde_json::json!({
        "resourceType": "ViewDefinition",
        "status": "active",
        "resource": "Patient",
        "select": [{
            "column": [{"name": "id", "path": "id"}]
        }, {
            "forEach": "name",
            "column": [
                {"name": "family", "path": "family"},
                {"name": "given", "path": "given.first()"}
            ]
        }]
    });
    let view_def: helios_fhir::r4::ViewDefinition = serde_json::from_value(view_json).unwrap();
    let view_def = SofViewDefinition::R4(view_def);

    let ndjson = r#"{"resourceType": "Patient", "id": "p1", "name": [{"family": "Smith", "given": ["John"]}, {"family": "Smith", "given": ["Johnny"]}]}"#;

    let input = BufReader::new(Cursor::new(ndjson));
    let mut output = Vec::new();

    let stats = process_ndjson_chunked(
        view_def,
        input,
        &mut output,
        ContentType::CsvWithHeader,
        ChunkConfig::default(),
    )
    .unwrap();

    // One patient with 2 names = 2 rows
    assert_eq!(stats.output_rows, 2);

    let output_str = String::from_utf8(output).unwrap();
    assert!(output_str.contains("Smith,John"));
    assert!(output_str.contains("Smith,Johnny"));
}

/// Test large dataset processing (stress test)
#[test]
#[cfg(feature = "R4")]
fn test_chunked_large_dataset() {
    // Generate 1000 patients
    let mut ndjson = String::new();
    for i in 0..1000 {
        ndjson.push_str(&format!(
            r#"{{"resourceType": "Patient", "id": "p{}", "gender": "{}"}}"#,
            i,
            if i % 2 == 0 { "male" } else { "female" }
        ));
        ndjson.push('\n');
    }

    let view_def = create_patient_view_definition();
    let input = BufReader::new(Cursor::new(ndjson));
    let mut output = Vec::new();

    let config = ChunkConfig {
        chunk_size: 100, // Process in 10 chunks of 100
        skip_invalid_lines: false,
    };

    let stats = process_ndjson_chunked(
        view_def,
        input,
        &mut output,
        ContentType::CsvWithHeader,
        config,
    )
    .unwrap();

    assert_eq!(stats.total_lines_read, 1000);
    assert_eq!(stats.resources_processed, 1000);
    assert_eq!(stats.output_rows, 1000);
    assert_eq!(stats.chunks_processed, 10);

    // Verify output has correct line count (header + 1000 data rows)
    let output_str = String::from_utf8(output).unwrap();
    assert_eq!(output_str.lines().count(), 1001);
}
