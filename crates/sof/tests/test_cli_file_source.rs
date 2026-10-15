/// Integration tests for sof-cli with -s file:// parameter
///
/// These tests verify that the CLI can correctly load FHIR data from local files
/// using the file:// URL scheme with the -s/--source parameter.
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

/// Helper function to get the path to the sof-cli binary
fn get_cli_binary_path() -> PathBuf {
    // Use CARGO_BIN_EXE_<name> env var set by cargo during test execution
    // This works regardless of the target directory (e.g., llvm-cov-target)
    PathBuf::from(env!("CARGO_BIN_EXE_sof-cli"))
}

/// Helper function to create a test ViewDefinition
fn create_test_view_definition() -> String {
    r#"{
        "resourceType": "ViewDefinition",
        "status": "active",
        "name": "test_view",
        "resource": "Patient",
        "select": [
            {
                "column": [
                    {
                        "name": "id",
                        "path": "id"
                    },
                    {
                        "name": "family",
                        "path": "name.family"
                    }
                ]
            }
        ]
    }"#
    .to_string()
}

/// Helper function to create a test Bundle with Patient resources
fn create_test_bundle() -> String {
    r#"{
        "resourceType": "Bundle",
        "type": "collection",
        "entry": [
            {
                "resource": {
                    "resourceType": "Patient",
                    "id": "patient-1",
                    "name": [{
                        "family": "Smith",
                        "given": ["John"]
                    }]
                }
            },
            {
                "resource": {
                    "resourceType": "Patient",
                    "id": "patient-2",
                    "name": [{
                        "family": "Jones",
                        "given": ["Jane"]
                    }]
                }
            }
        ]
    }"#
    .to_string()
}

/// Helper function to create a single Patient resource
fn create_test_patient() -> String {
    r#"{
        "resourceType": "Patient",
        "id": "single-patient",
        "name": [{
            "family": "Doe",
            "given": ["John"]
        }]
    }"#
    .to_string()
}

/// Helper function to create an array of Patient resources
fn create_test_patient_array() -> String {
    r#"[
        {
            "resourceType": "Patient",
            "id": "array-patient-1",
            "name": [{
                "family": "Array1",
                "given": ["Patient"]
            }]
        },
        {
            "resourceType": "Patient",
            "id": "array-patient-2",
            "name": [{
                "family": "Array2",
                "given": ["Patient"]
            }]
        }
    ]"#
    .to_string()
}

#[test]
fn test_cli_with_file_source_bundle() {
    let cli_path = get_cli_binary_path();
    if !cli_path.exists() {
        panic!(
            "CLI binary not found at {:?}. Run 'cargo build' first.",
            cli_path
        );
    }

    // Create temporary files for ViewDefinition and Bundle
    let temp_dir = TempDir::new().unwrap();
    let view_path = temp_dir.path().join("view.json");
    let bundle_path = temp_dir.path().join("bundle.json");

    fs::write(&view_path, create_test_view_definition()).unwrap();
    fs::write(&bundle_path, create_test_bundle()).unwrap();

    // Convert to file:// URL
    let bundle_url = format!("file://{}", bundle_path.to_string_lossy());

    // Run the CLI with file:// source
    let output = Command::new(&cli_path)
        .arg("-v")
        .arg(&view_path)
        .arg("-s")
        .arg(&bundle_url)
        .arg("-f")
        .arg("csv")
        .output()
        .expect("Failed to execute CLI");

    // Check that the command succeeded
    if !output.status.success() {
        eprintln!("CLI output: {}", String::from_utf8_lossy(&output.stdout));
        eprintln!("CLI error: {}", String::from_utf8_lossy(&output.stderr));
        panic!("CLI command failed with status: {}", output.status);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Verify CSV output contains headers
    assert!(
        stdout.contains("id"),
        "Output should contain 'id' column header"
    );
    assert!(
        stdout.contains("family"),
        "Output should contain 'family' column header"
    );

    // Verify patient data is present
    assert!(
        stdout.contains("patient-1") || stdout.contains("Smith"),
        "Output should contain patient-1 data"
    );
    assert!(
        stdout.contains("patient-2") || stdout.contains("Jones"),
        "Output should contain patient-2 data"
    );
}

#[test]
fn test_cli_with_file_source_single_resource() {
    let cli_path = get_cli_binary_path();
    if !cli_path.exists() {
        panic!(
            "CLI binary not found at {:?}. Run 'cargo build' first.",
            cli_path
        );
    }

    let temp_dir = TempDir::new().unwrap();
    let view_path = temp_dir.path().join("view.json");
    let patient_path = temp_dir.path().join("patient.json");

    fs::write(&view_path, create_test_view_definition()).unwrap();
    fs::write(&patient_path, create_test_patient()).unwrap();

    let patient_url = format!("file://{}", patient_path.to_string_lossy());

    let output = Command::new(&cli_path)
        .arg("-v")
        .arg(&view_path)
        .arg("-s")
        .arg(&patient_url)
        .arg("-f")
        .arg("csv")
        .output()
        .expect("Failed to execute CLI");

    if !output.status.success() {
        eprintln!("CLI error: {}", String::from_utf8_lossy(&output.stderr));
        panic!("CLI command failed");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("id"));
    assert!(stdout.contains("single-patient") || stdout.contains("Doe"));
}

#[test]
fn test_cli_with_file_source_resource_array() {
    let cli_path = get_cli_binary_path();
    if !cli_path.exists() {
        panic!(
            "CLI binary not found at {:?}. Run 'cargo build' first.",
            cli_path
        );
    }

    let temp_dir = TempDir::new().unwrap();
    let view_path = temp_dir.path().join("view.json");
    let array_path = temp_dir.path().join("array.json");

    fs::write(&view_path, create_test_view_definition()).unwrap();
    fs::write(&array_path, create_test_patient_array()).unwrap();

    let array_url = format!("file://{}", array_path.to_string_lossy());

    let output = Command::new(&cli_path)
        .arg("-v")
        .arg(&view_path)
        .arg("-s")
        .arg(&array_url)
        .arg("-f")
        .arg("csv")
        .output()
        .expect("Failed to execute CLI");

    if !output.status.success() {
        eprintln!("CLI error: {}", String::from_utf8_lossy(&output.stderr));
        panic!("CLI command failed");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("array-patient-1") || stdout.contains("Array1"));
    assert!(stdout.contains("array-patient-2") || stdout.contains("Array2"));
}

#[test]
fn test_cli_with_file_source_and_bundle() {
    // Test combining -s file:// with -b bundle.json (merges both sources)
    let cli_path = get_cli_binary_path();
    if !cli_path.exists() {
        panic!(
            "CLI binary not found at {:?}. Run 'cargo build' first.",
            cli_path
        );
    }

    let temp_dir = TempDir::new().unwrap();
    let view_path = temp_dir.path().join("view.json");
    let source_path = temp_dir.path().join("source.json");
    let bundle_path = temp_dir.path().join("bundle.json");

    fs::write(&view_path, create_test_view_definition()).unwrap();

    // Source file has one patient
    fs::write(&source_path, create_test_patient()).unwrap();

    // Bundle file has two patients
    fs::write(&bundle_path, create_test_bundle()).unwrap();

    let source_url = format!("file://{}", source_path.to_string_lossy());

    let output = Command::new(&cli_path)
        .arg("-v")
        .arg(&view_path)
        .arg("-s")
        .arg(&source_url)
        .arg("-b")
        .arg(&bundle_path)
        .arg("-f")
        .arg("csv")
        .output()
        .expect("Failed to execute CLI");

    if !output.status.success() {
        eprintln!("CLI error: {}", String::from_utf8_lossy(&output.stderr));
        panic!("CLI command failed");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should contain data from both sources (3 patients total)
    assert!(
        stdout.contains("single-patient") || stdout.contains("Doe"),
        "Should contain patient from source file"
    );
    assert!(
        stdout.contains("patient-1") || stdout.contains("Smith"),
        "Should contain patient-1 from bundle"
    );
    assert!(
        stdout.contains("patient-2") || stdout.contains("Jones"),
        "Should contain patient-2 from bundle"
    );
}

#[test]
fn test_cli_with_file_source_not_found() {
    let cli_path = get_cli_binary_path();
    if !cli_path.exists() {
        panic!(
            "CLI binary not found at {:?}. Run 'cargo build' first.",
            cli_path
        );
    }

    let temp_dir = TempDir::new().unwrap();
    let view_path = temp_dir.path().join("view.json");
    fs::write(&view_path, create_test_view_definition()).unwrap();

    // Use a non-existent file with platform-appropriate path
    let nonexistent_path = if cfg!(windows) {
        // Windows requires drive letter for file URLs
        "file:///C:/nonexistent/path/to/file.json"
    } else {
        "file:///nonexistent/path/to/file.json"
    };

    let output = Command::new(&cli_path)
        .arg("-v")
        .arg(&view_path)
        .arg("-s")
        .arg(nonexistent_path)
        .arg("-f")
        .arg("csv")
        .output()
        .expect("Failed to execute CLI");

    // Command should fail
    assert!(
        !output.status.success(),
        "CLI should fail with non-existent file"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("File not found")
            || stderr.contains("not found")
            || stderr.contains("Invalid"),
        "Error message should indicate file not found or invalid path. Actual: {}",
        stderr
    );
}

#[test]
fn test_cli_with_file_source_invalid_json() {
    let cli_path = get_cli_binary_path();
    if !cli_path.exists() {
        panic!(
            "CLI binary not found at {:?}. Run 'cargo build' first.",
            cli_path
        );
    }

    let temp_dir = TempDir::new().unwrap();
    let view_path = temp_dir.path().join("view.json");
    let invalid_path = temp_dir.path().join("invalid.json");

    fs::write(&view_path, create_test_view_definition()).unwrap();
    fs::write(&invalid_path, "{ this is not valid json }").unwrap();

    let invalid_url = format!("file://{}", invalid_path.to_string_lossy());

    let output = Command::new(&cli_path)
        .arg("-v")
        .arg(&view_path)
        .arg("-s")
        .arg(&invalid_url)
        .arg("-f")
        .arg("csv")
        .output()
        .expect("Failed to execute CLI");

    // Command should fail
    assert!(
        !output.status.success(),
        "CLI should fail with invalid JSON"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("parse") || stderr.contains("JSON") || stderr.contains("error"),
        "Error message should indicate JSON parsing error"
    );
}

#[test]
fn test_cli_with_file_source_json_output() {
    let cli_path = get_cli_binary_path();
    if !cli_path.exists() {
        panic!(
            "CLI binary not found at {:?}. Run 'cargo build' first.",
            cli_path
        );
    }

    let temp_dir = TempDir::new().unwrap();
    let view_path = temp_dir.path().join("view.json");
    let bundle_path = temp_dir.path().join("bundle.json");

    fs::write(&view_path, create_test_view_definition()).unwrap();
    fs::write(&bundle_path, create_test_bundle()).unwrap();

    let bundle_url = format!("file://{}", bundle_path.to_string_lossy());

    let output = Command::new(&cli_path)
        .arg("-v")
        .arg(&view_path)
        .arg("-s")
        .arg(&bundle_url)
        .arg("-f")
        .arg("json")
        .output()
        .expect("Failed to execute CLI");

    if !output.status.success() {
        eprintln!("CLI error: {}", String::from_utf8_lossy(&output.stderr));
        panic!("CLI command failed");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Verify it's valid JSON
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&stdout);
    assert!(parsed.is_ok(), "Output should be valid JSON");

    // Verify it's an array
    let json_value = parsed.unwrap();
    assert!(json_value.is_array(), "JSON output should be an array");
}

#[test]
fn test_cli_with_file_source_output_to_file() {
    let cli_path = get_cli_binary_path();
    if !cli_path.exists() {
        panic!(
            "CLI binary not found at {:?}. Run 'cargo build' first.",
            cli_path
        );
    }

    let temp_dir = TempDir::new().unwrap();
    let view_path = temp_dir.path().join("view.json");
    let bundle_path = temp_dir.path().join("bundle.json");
    let output_path = temp_dir.path().join("output.csv");

    fs::write(&view_path, create_test_view_definition()).unwrap();
    fs::write(&bundle_path, create_test_bundle()).unwrap();

    let bundle_url = format!("file://{}", bundle_path.to_string_lossy());

    let output = Command::new(&cli_path)
        .arg("-v")
        .arg(&view_path)
        .arg("-s")
        .arg(&bundle_url)
        .arg("-f")
        .arg("csv")
        .arg("-o")
        .arg(&output_path)
        .output()
        .expect("Failed to execute CLI");

    if !output.status.success() {
        eprintln!("CLI error: {}", String::from_utf8_lossy(&output.stderr));
        panic!("CLI command failed");
    }

    // Verify output file was created
    assert!(output_path.exists(), "Output file should be created");

    // Verify output file content
    let content = fs::read_to_string(&output_path).unwrap();
    assert!(content.contains("id"));
    assert!(content.contains("patient-1") || content.contains("Smith"));
}

#[test]
fn test_cli_with_file_source_no_headers() {
    let cli_path = get_cli_binary_path();
    if !cli_path.exists() {
        panic!(
            "CLI binary not found at {:?}. Run 'cargo build' first.",
            cli_path
        );
    }

    let temp_dir = TempDir::new().unwrap();
    let view_path = temp_dir.path().join("view.json");
    let bundle_path = temp_dir.path().join("bundle.json");

    fs::write(&view_path, create_test_view_definition()).unwrap();
    fs::write(&bundle_path, create_test_bundle()).unwrap();

    let bundle_url = format!("file://{}", bundle_path.to_string_lossy());

    let output = Command::new(&cli_path)
        .arg("-v")
        .arg(&view_path)
        .arg("-s")
        .arg(&bundle_url)
        .arg("-f")
        .arg("csv")
        .arg("--no-headers")
        .output()
        .expect("Failed to execute CLI");

    if !output.status.success() {
        eprintln!("CLI error: {}", String::from_utf8_lossy(&output.stderr));
        panic!("CLI command failed");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should not start with headers
    let first_line = stdout.lines().next().unwrap_or("");
    assert!(
        !first_line.contains("id,family"),
        "First line should not be headers with --no-headers flag"
    );

    // But should still contain data
    assert!(stdout.contains("patient-1") || stdout.contains("Smith"));
}

/// Helper function to create NDJSON test data
fn create_test_ndjson() -> String {
    r#"{"resourceType": "Patient", "id": "ndjson-p1", "name": [{"family": "NdFirst", "given": ["Test"]}]}
{"resourceType": "Patient", "id": "ndjson-p2", "name": [{"family": "NdSecond", "given": ["Test"]}]}
{"resourceType": "Patient", "id": "ndjson-p3", "name": [{"family": "NdThird", "given": ["Test"]}]}"#.to_string()
}

#[test]
fn test_cli_with_ndjson_file_via_source() {
    let cli_path = get_cli_binary_path();
    if !cli_path.exists() {
        panic!(
            "CLI binary not found at {:?}. Run 'cargo build' first.",
            cli_path
        );
    }

    let temp_dir = TempDir::new().unwrap();
    let view_path = temp_dir.path().join("view.json");
    let ndjson_path = temp_dir.path().join("data.ndjson");

    fs::write(&view_path, create_test_view_definition()).unwrap();
    fs::write(&ndjson_path, create_test_ndjson()).unwrap();

    let ndjson_url = format!("file://{}", ndjson_path.to_string_lossy());

    let output = Command::new(&cli_path)
        .arg("-v")
        .arg(&view_path)
        .arg("-s")
        .arg(&ndjson_url)
        .arg("-f")
        .arg("csv")
        .output()
        .expect("Failed to execute CLI");

    if !output.status.success() {
        eprintln!("CLI error: {}", String::from_utf8_lossy(&output.stderr));
        panic!("CLI command failed");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Verify all three NDJSON patients are present
    assert!(
        stdout.contains("ndjson-p1") || stdout.contains("NdFirst"),
        "Should contain first NDJSON patient"
    );
    assert!(
        stdout.contains("ndjson-p2") || stdout.contains("NdSecond"),
        "Should contain second NDJSON patient"
    );
    assert!(
        stdout.contains("ndjson-p3") || stdout.contains("NdThird"),
        "Should contain third NDJSON patient"
    );
}

#[test]
fn test_cli_with_ndjson_file_via_bundle() {
    let cli_path = get_cli_binary_path();
    if !cli_path.exists() {
        panic!(
            "CLI binary not found at {:?}. Run 'cargo build' first.",
            cli_path
        );
    }

    let temp_dir = TempDir::new().unwrap();
    let view_path = temp_dir.path().join("view.json");
    let ndjson_path = temp_dir.path().join("data.ndjson");

    fs::write(&view_path, create_test_view_definition()).unwrap();
    fs::write(&ndjson_path, create_test_ndjson()).unwrap();

    let output = Command::new(&cli_path)
        .arg("-v")
        .arg(&view_path)
        .arg("-b")
        .arg(&ndjson_path)
        .arg("-f")
        .arg("csv")
        .output()
        .expect("Failed to execute CLI");

    if !output.status.success() {
        eprintln!("CLI error: {}", String::from_utf8_lossy(&output.stderr));
        panic!("CLI command failed");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Verify NDJSON content is loaded
    assert!(stdout.contains("ndjson-p1") || stdout.contains("NdFirst"));
    assert!(stdout.contains("ndjson-p2") || stdout.contains("NdSecond"));
    assert!(stdout.contains("ndjson-p3") || stdout.contains("NdThird"));
}

#[test]
fn test_cli_with_ndjson_content_fallback() {
    // Test that NDJSON content without .ndjson extension is detected via fallback
    let cli_path = get_cli_binary_path();
    if !cli_path.exists() {
        panic!(
            "CLI binary not found at {:?}. Run 'cargo build' first.",
            cli_path
        );
    }

    let temp_dir = TempDir::new().unwrap();
    let view_path = temp_dir.path().join("view.json");
    let data_path = temp_dir.path().join("data.json"); // .json extension, but NDJSON content

    fs::write(&view_path, create_test_view_definition()).unwrap();
    fs::write(&data_path, create_test_ndjson()).unwrap();

    let data_url = format!("file://{}", data_path.to_string_lossy());

    let output = Command::new(&cli_path)
        .arg("-v")
        .arg(&view_path)
        .arg("-s")
        .arg(&data_url)
        .arg("-f")
        .arg("csv")
        .output()
        .expect("Failed to execute CLI");

    if !output.status.success() {
        eprintln!("CLI error: {}", String::from_utf8_lossy(&output.stderr));
        panic!("CLI command failed - content-based NDJSON detection should work");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should successfully parse as NDJSON via content fallback
    assert!(stdout.contains("ndjson-p1") || stdout.contains("NdFirst"));
}

#[test]
fn test_cli_with_ndjson_and_bundle_mixed() {
    // Test combining NDJSON source with regular JSON bundle
    let cli_path = get_cli_binary_path();
    if !cli_path.exists() {
        panic!(
            "CLI binary not found at {:?}. Run 'cargo build' first.",
            cli_path
        );
    }

    let temp_dir = TempDir::new().unwrap();
    let view_path = temp_dir.path().join("view.json");
    let ndjson_path = temp_dir.path().join("data.ndjson");
    let bundle_path = temp_dir.path().join("bundle.json");

    fs::write(&view_path, create_test_view_definition()).unwrap();
    fs::write(&ndjson_path, create_test_ndjson()).unwrap();
    fs::write(&bundle_path, create_test_bundle()).unwrap();

    let ndjson_url = format!("file://{}", ndjson_path.to_string_lossy());

    let output = Command::new(&cli_path)
        .arg("-v")
        .arg(&view_path)
        .arg("-s")
        .arg(&ndjson_url)
        .arg("-b")
        .arg(&bundle_path)
        .arg("-f")
        .arg("csv")
        .output()
        .expect("Failed to execute CLI");

    if !output.status.success() {
        eprintln!("CLI error: {}", String::from_utf8_lossy(&output.stderr));
        panic!("CLI command failed");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should contain data from both NDJSON (3 patients) and Bundle (2 patients) = 5 total
    assert!(
        stdout.contains("ndjson-p1") || stdout.contains("NdFirst"),
        "Should contain NDJSON patients"
    );
    assert!(
        stdout.contains("patient-1") || stdout.contains("Smith"),
        "Should contain Bundle patients"
    );
}

#[test]
fn test_cli_with_ndjson_json_output() {
    let cli_path = get_cli_binary_path();
    if !cli_path.exists() {
        panic!(
            "CLI binary not found at {:?}. Run 'cargo build' first.",
            cli_path
        );
    }

    let temp_dir = TempDir::new().unwrap();
    let view_path = temp_dir.path().join("view.json");
    let ndjson_path = temp_dir.path().join("data.ndjson");

    fs::write(&view_path, create_test_view_definition()).unwrap();
    fs::write(&ndjson_path, create_test_ndjson()).unwrap();

    let ndjson_url = format!("file://{}", ndjson_path.to_string_lossy());

    let output = Command::new(&cli_path)
        .arg("-v")
        .arg(&view_path)
        .arg("-s")
        .arg(&ndjson_url)
        .arg("-f")
        .arg("json")
        .output()
        .expect("Failed to execute CLI");

    if !output.status.success() {
        eprintln!("CLI error: {}", String::from_utf8_lossy(&output.stderr));
        panic!("CLI command failed");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Verify it's valid JSON
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&stdout);
    assert!(parsed.is_ok(), "Output should be valid JSON");

    // Verify it contains the expected data
    let json_value = parsed.unwrap();
    assert!(json_value.is_array(), "JSON output should be an array");
    assert_eq!(
        json_value.as_array().unwrap().len(),
        3,
        "Should have 3 patients from NDJSON"
    );
}

/// A view whose column `tagged_col` carries an unknown `ansi/type` tag.
fn invalid_tag_view() -> String {
    r#"{
        "resourceType": "ViewDefinition", "status": "active", "resource": "Patient",
        "select": [{"column": [
            {"name": "id", "path": "id"},
            {"name": "tagged_col", "path": "birthDate", "type": "date",
             "tag": [{"name": "ansi/type", "value": "GEOMETRY"}]}
        ]}]
    }"#
    .to_string()
}

/// An invalid `ansi/type` tag must fail the CLI (non-zero exit) and name the
/// column, on the batch path and on the streaming NDJSON path
/// (`--bundle x.ndjson`), for every text format and the columnar ones.
#[test]
fn test_cli_invalid_ansi_type_tag_fails_naming_the_column() {
    let temp_dir = TempDir::new().unwrap();
    let view_path = temp_dir.path().join("view.json");
    let bundle_path = temp_dir.path().join("bundle.json");
    let ndjson_path = temp_dir.path().join("patients.ndjson");
    fs::write(&view_path, invalid_tag_view()).unwrap();
    fs::write(&bundle_path, create_test_bundle()).unwrap();
    fs::write(
        &ndjson_path,
        "{\"resourceType\":\"Patient\",\"id\":\"p1\",\"birthDate\":\"1990-01-01\"}\n",
    )
    .unwrap();

    for (label, data) in [("batch", &bundle_path), ("streaming", &ndjson_path)] {
        for format in ["csv", "json", "ndjson", "parquet", "arrow"] {
            let output = Command::new(get_cli_binary_path())
                .arg("-v")
                .arg(&view_path)
                .arg("-b")
                .arg(data)
                .arg("-f")
                .arg(format)
                .output()
                .expect("Failed to execute CLI");
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                !output.status.success(),
                "{label}/{format}: expected non-zero exit; stderr: {stderr}"
            );
            assert!(
                stderr.contains("tagged_col"),
                "{label}/{format}: stderr must name the column: {stderr}"
            );
        }
    }
}

#[test]
fn test_cli_rejects_non_string_ansi_type_tag_value_naming_the_column() {
    let temp_dir = TempDir::new().unwrap();
    let view_path = temp_dir.path().join("view.json");
    let bundle_path = temp_dir.path().join("bundle.json");
    fs::write(
        &view_path,
        r#"{"resourceType":"ViewDefinition","status":"active","resource":"Patient",
            "select":[{"column":[{"name":"birth_date","path":"birthDate",
            "tag":[{"name":"ansi/type","value":42}]}]}]}"#,
    )
    .unwrap();
    fs::write(&bundle_path, create_test_bundle()).unwrap();

    for format in ["csv", "json", "ndjson", "parquet"] {
        let output = Command::new(get_cli_binary_path())
            .arg("-v")
            .arg(&view_path)
            .arg("-b")
            .arg(&bundle_path)
            .arg("-f")
            .arg(format)
            .output()
            .expect("Failed to execute CLI");
        assert!(!output.status.success(), "{format}: must exit non-zero");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("birth_date"), "{format}: stderr: {stderr}");
    }
}
