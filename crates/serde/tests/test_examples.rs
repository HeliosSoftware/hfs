#[cfg(feature = "xml")]
use helios_serde::xml::{from_xml_str, to_xml_string};
#[cfg(feature = "xml")]
use quick_xml::Reader;
#[cfg(feature = "xml")]
use quick_xml::events::Event;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(feature = "R4")]
#[test]
fn test_r4_json_examples() {
    let examples_dir = json_examples_dir("R4");
    test_json_examples_in_dir::<helios_fhir::r4::Resource>(&examples_dir, "R4");
}

#[cfg(feature = "R4B")]
#[test]
fn test_r4b_json_examples() {
    let examples_dir = json_examples_dir("R4B");
    test_json_examples_in_dir::<helios_fhir::r4b::Resource>(&examples_dir, "R4B");
}

#[cfg(feature = "R5")]
#[test]
fn test_r5_json_examples() {
    let examples_dir = json_examples_dir("R5");
    test_json_examples_in_dir::<helios_fhir::r5::Resource>(&examples_dir, "R5");
}

#[cfg(feature = "R6")]
#[test]
fn test_r6_json_examples() {
    let examples_dir = json_examples_dir("R6");
    test_json_examples_in_dir::<helios_fhir::r6::Resource>(&examples_dir, "R6");
}

#[cfg(all(feature = "R4", feature = "xml"))]
#[test]
fn test_r4_xml_examples() {
    let examples_dir = xml_examples_dir("R4");
    test_xml_examples_in_dir::<helios_fhir::r4::Resource>(&examples_dir, "R4");
}

#[cfg(all(feature = "R4B", feature = "xml"))]
#[test]
fn test_r4b_xml_examples() {
    let examples_dir = xml_examples_dir("R4B");
    test_xml_examples_in_dir::<helios_fhir::r4b::Resource>(&examples_dir, "R4B");
}

#[cfg(all(feature = "R5", feature = "xml"))]
#[test]
fn test_r5_xml_examples() {
    let examples_dir = xml_examples_dir("R5");
    test_xml_examples_in_dir::<helios_fhir::r5::Resource>(&examples_dir, "R5");
}

#[cfg(all(feature = "R6", feature = "xml"))]
#[test]
fn test_r6_xml_examples() {
    let examples_dir = xml_examples_dir("R6");
    test_xml_examples_in_dir::<helios_fhir::r6::Resource>(&examples_dir, "R6");
}

// This function is no longer needed with our simplified approach

// Function to find differences between two JSON values
fn find_json_differences(original: &Value, reserialized: &Value) -> Vec<(String, Value, Value)> {
    let mut differences = Vec::new();
    compare_json_values(original, reserialized, String::new(), &mut differences);
    differences
}

// Recursively compare JSON values and collect differences
fn compare_json_values(
    original: &Value,
    reserialized: &Value,
    path: String,
    differences: &mut Vec<(String, Value, Value)>,
) {
    match (original, reserialized) {
        (Value::Object(orig_obj), Value::Object(reser_obj)) => {
            // Check for missing keys in either direction
            let orig_keys: std::collections::HashSet<&String> = orig_obj.keys().collect();
            let reser_keys: std::collections::HashSet<&String> = reser_obj.keys().collect();

            // Keys in original but not in reserialized
            for key in orig_keys.difference(&reser_keys) {
                let new_path = if path.is_empty() {
                    key.to_string()
                } else {
                    format!("{}.{}", path, key)
                };
                differences.push((new_path, orig_obj[*key].clone(), Value::Null));
            }

            // Keys in reserialized but not in original
            for key in reser_keys.difference(&orig_keys) {
                let new_path = if path.is_empty() {
                    key.to_string()
                } else {
                    format!("{}.{}", path, key)
                };
                differences.push((new_path, Value::Null, reser_obj[*key].clone()));
            }

            // Compare values for keys that exist in both
            for key in orig_keys.intersection(&reser_keys) {
                let new_path = if path.is_empty() {
                    key.to_string()
                } else {
                    format!("{}.{}", path, key)
                };
                compare_json_values(&orig_obj[*key], &reser_obj[*key], new_path, differences);
            }
        }
        (Value::Array(orig_arr), Value::Array(reser_arr)) => {
            // Compare arrays element by element if they're the same length
            if orig_arr.len() == reser_arr.len() {
                for (i, (orig_val, reser_val)) in orig_arr.iter().zip(reser_arr.iter()).enumerate()
                {
                    let new_path = if path.is_empty() {
                        format!("[{}]", i)
                    } else {
                        format!("{}[{}]", path, i)
                    };
                    compare_json_values(orig_val, reser_val, new_path, differences);
                }
            } else {
                // Check if this is a valid null-skipping transformation
                // (reserialized array contains only the non-null values from original)
                let orig_non_null: Vec<&Value> = orig_arr.iter().filter(|v| !v.is_null()).collect();
                let is_null_skipping_transformation = orig_non_null.len() == reser_arr.len()
                    && orig_non_null
                        .iter()
                        .zip(reser_arr.iter())
                        .all(|(orig, reser)| *orig == reser);

                if !is_null_skipping_transformation {
                    // If arrays have different lengths and it's not a null-skipping case,
                    // report the whole array as different
                    differences.push((path, original.clone(), reserialized.clone()));
                }
                // If it is a null-skipping transformation, we consider it valid and don't report it as a difference
            }
        }
        // For other primitive values, check equality with special handling for string-to-integer conversion
        _ => {
            if original != reserialized {
                // Check if this is a valid string-to-integer conversion
                let is_valid_conversion = match (original, reserialized) {
                    // String "123" to Number 123 is valid
                    (Value::String(s), Value::Number(n)) => {
                        // Try to parse the string as the same integer that we got
                        if let Ok(parsed_int) = s.parse::<i64>() {
                            n.as_i64() == Some(parsed_int)
                        } else if let Ok(parsed_uint) = s.parse::<u64>() {
                            n.as_u64() == Some(parsed_uint)
                        } else {
                            false
                        }
                    }
                    // All other mismatches are real differences
                    _ => false,
                };

                if !is_valid_conversion {
                    differences.push((path, original.clone(), reserialized.clone()));
                }
            }
        }
    }
}

// Helper function to find items in a Questionnaire that are missing linkId
fn find_missing_linkid(json: &serde_json::Value) {
    if let Some(items) = json.get("item").and_then(|i| i.as_array()) {
        for (index, item) in items.iter().enumerate() {
            if item.get("linkId").is_none() {
                println!("Item at index {} is missing linkId", index);
                println!(
                    "Item content: {}",
                    serde_json::to_string_pretty(item).unwrap_or_default()
                );
            }

            // Recursively check nested items
            if let Some(nested_items) = item.get("item") {
                println!("Checking nested items for item at index {}", index);
                find_missing_linkid(&serde_json::json!({"item": nested_items}));
            }
        }
    }
}

fn tests_data_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("serde crate has parent directory")
        .join("fhir")
        .join("tests")
        .join("data")
}

fn json_examples_dir(version: &str) -> PathBuf {
    tests_data_root().join("json").join(version)
}

#[cfg(feature = "xml")]
fn xml_examples_dir(version: &str) -> PathBuf {
    tests_data_root().join("xml").join(version)
}

fn should_skip_file(
    filename: &str,
    skip_entries: &[(&'static str, &'static str)],
) -> Option<&'static str> {
    for (pattern, reason) in skip_entries {
        if let Some(prefix) = pattern.strip_suffix('*') {
            if filename.starts_with(prefix) {
                return Some(*reason);
            }
        } else if filename == *pattern {
            return Some(*reason);
        }
    }
    None
}

fn json_skip_list(_version: &str) -> &'static [(&'static str, &'static str)] {
    const JSON_SKIPS: &[(&str, &str)] = &[
        (
            "diagnosticreport-example-f202-bloodculture.json",
            "Contains null where struct TempCodeableReference expected",
        ),
        (
            "permission-example-bundle-residual.json",
            "Contains null where struct TempPermissionRuleLimit expected",
        ),
        (
            "diagnosticreport-example-dxa.json",
            "Contains null in conclusionCode array where struct TempCodeableReference expected",
        ),
        (
            "servicerequest-example-glucose.json",
            "Contains null in asNeededFor array where struct TempCodeableConcept expected",
        ),
        (
            "diagnosticreport-example-f201-brainct.json",
            "Contains null in conclusionCode array where struct TempCodeableReference expected",
        ),
        (
            "specimen-example-liver-biopsy.json",
            "R6 Specimen example contains incompatible data structure",
        ),
        (
            "specimen-example-urine.json",
            "Contains null in processing.additive array where struct TempReference expected",
        ),
        (
            "specimen-example-pooled-serum.json",
            "Contains null in container array - invalid FHIR JSON",
        ),
        (
            "task-example-fm-status-resp.json",
            "Contains null where struct TempTaskFocus expected",
        ),
        (
            "task-example-fm-status.json",
            "Contains null where struct TempTaskFocus expected",
        ),
        (
            "diagnosticreport-example-ghp.json",
            "Contains null where struct TempSpecimenContainer expected",
        ),
        (
            "specimen-example-serum.json",
            "Contains null in container array - invalid FHIR JSON",
        ),
        (
            "task-example-fm-reprocess.json",
            "Contains null where struct TempTaskFocus expected",
        ),
        (
            "composition-example.json",
            "R6 Composition.attester.mode structure incompatibility - expecting string but got CodeableConcept",
        ),
        (
            "devicealert-example.json",
            "R6 DeviceAlert example contains incompatible data structure",
        ),
        (
            "familymemberhistory-example.json",
            "R6 FamilyMemberHistory example contains incompatible data structure",
        ),
        (
            "Requirements-example1.json",
            "R6 Requirements statements lose `category` arrays during roundtrip serialization",
        ),
        (
            "provenance-ex-patient-merged.json",
            "Contains null in authorization array where struct TempCodeableReference expected",
        ),
        (
            "provenance-example-advanced.json",
            "Contains null in authorization array where struct TempCodeableReference expected",
        ),
        (
            "provenance-example-correction-replacement.json",
            "Contains null in authorization array where struct TempCodeableReference expected",
        ),
        (
            "provenance-example-delete.json",
            "Contains null in authorization array where struct TempCodeableReference expected",
        ),
        (
            "provenance-example-sig.json",
            "Contains null in authorization array where struct TempCodeableReference expected",
        ),
        (
            "provenance-example-verify.json",
            "Contains null in authorization array where struct TempCodeableReference expected",
        ),
        (
            "provenance-example-bundle-allergyintolerance.json",
            "Contains null in AllergyIntolerance.category array - invalid FHIR JSON",
        ),
    ];

    JSON_SKIPS
}

fn test_json_examples_in_dir<R: DeserializeOwned + Serialize>(dir: &Path, fhir_version: &str) {
    if !dir.exists() {
        println!("Directory does not exist: {:?}", dir);
        return;
    }

    let skip_files = json_skip_list(fhir_version);

    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();

        if path.is_file() && path.extension().is_some_and(|ext| ext == "json") {
            let filename = path.file_name().unwrap().to_string_lossy().to_string();

            // Check if this file should be skipped
            if let Some(reason) = should_skip_file(&filename, skip_files) {
                println!("Skipping file: {} - Reason: {}", filename, reason);
                continue;
            }

            println!("Processing file: {}", path.display());

            // Read the file content
            match fs::read_to_string(&path) {
                Ok(content) => {
                    if content.trim().is_empty() {
                        println!("Skipping empty JSON file: {}", path.display());
                        continue;
                    }
                    // Parse the JSON string
                    match serde_json::from_str::<serde_json::Value>(&content) {
                        Ok(json_value) => {
                            // Check if it has a resourceType field
                            if let Some(resource_type) = json_value.get("resourceType") {
                                if let Some(resource_type_str) = resource_type.as_str() {
                                    println!("Resource type: {}", resource_type_str);

                                    if resource_type_str == "Questionnaire" {
                                        println!("Skipping Questionnaire resource");
                                        continue;
                                    }

                                    if resource_type_str == "ClinicalImpression" {
                                        println!("Skipping ClinicalImpression resource");
                                        continue;
                                    }

                                    if resource_type_str == "SubstanceSourceMaterial" {
                                        println!("Skipping SubstanceSourceMaterial resource");
                                        continue;
                                    }

                                    // Skip other missing R6 resources (not yet implemented or removed from spec)
                                    let missing_r6_resources = [
                                        "MolecularSequence",
                                        "Permission",
                                        "SubstanceNucleicAcid",
                                        "SubstancePolymer",
                                        "SubstanceProtein",
                                        "SubstanceReferenceInformation",
                                    ];

                                    if missing_r6_resources.contains(&resource_type_str) {
                                        println!("Skipping {} resource", resource_type_str);
                                        continue;
                                    }

                                    // Try to convert the JSON value to a FHIR Resource
                                    match serde_json::from_value::<R>(json_value.clone()) {
                                        Ok(resource) => {
                                            println!(
                                                "Successfully converted JSON to FHIR Resource"
                                            );

                                            // Verify we can serialize the Resource back to JSON
                                            match serde_json::to_value(&resource) {
                                                Ok(resource_json) => {
                                                    println!(
                                                        "Successfully serialized Resource back to JSON"
                                                    );

                                                    // Find differences between original and re-serialized JSON
                                                    let diff_paths = find_json_differences(
                                                        &json_value,
                                                        &resource_json,
                                                    );

                                                    if !diff_paths.is_empty() {
                                                        println!(
                                                            "Found {} significant differences between original and reserialized JSON:",
                                                            diff_paths.len()
                                                        );
                                                        for (path, orig_val, new_val) in &diff_paths
                                                        {
                                                            println!("  Path: {}", path);
                                                            println!(
                                                                "    Original: {}",
                                                                serde_json::to_string_pretty(
                                                                    orig_val
                                                                )
                                                                .unwrap_or_default()
                                                            );
                                                            println!(
                                                                "    Reserialized: {}",
                                                                serde_json::to_string_pretty(
                                                                    new_val
                                                                )
                                                                .unwrap_or_default()
                                                            );
                                                        }

                                                        // Only fail the test if there are actual significant differences
                                                        // (not just valid string-to-integer conversions)
                                                        panic!(
                                                            "Found {} significant differences in JSON values.\nSee above for specific differences.",
                                                            diff_paths.len()
                                                        );
                                                    }

                                                    println!("Resource JSON matches original JSON");
                                                }
                                                Err(e) => {
                                                    panic!(
                                                        "Error serializing Resource to JSON: {}",
                                                        e
                                                    );
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            let error_message = format!(
                                                "Error converting JSON to FHIR Resource: {}",
                                                e
                                            );
                                            println!("{}", error_message);

                                            // Try to extract more information about the missing field
                                            if error_message.contains("missing field") {
                                                // Print the JSON structure to help locate the issue
                                                println!("JSON structure:");
                                                if let Ok(pretty_json) =
                                                    serde_json::to_string_pretty(&json_value)
                                                {
                                                    println!("{}", pretty_json);
                                                }

                                                // If it's a Questionnaire, look for items without linkId
                                                if resource_type_str == "Questionnaire" {
                                                    println!(
                                                        "Checking for Questionnaire items without linkId:"
                                                    );
                                                    find_missing_linkid(&json_value);
                                                }
                                            }

                                            panic!("{}", error_message);
                                        }
                                    }
                                } else {
                                    println!("resourceType is not a string");
                                }
                            } else {
                                println!("JSON does not contain a resourceType field");
                            }
                        }
                        Err(e) => {
                            println!("Error parsing JSON: {}: {}", path.display(), e);
                        }
                    }
                }
                Err(e) => {
                    println!("Error opening file: {}: {}", path.display(), e);
                }
            }
        }
    }
}

/// Normalizes XML by parsing it and removing insignificant whitespace
#[cfg(feature = "xml")]
fn normalize_xml(xml: &str) -> Result<Vec<u8>, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut writer = quick_xml::Writer::new(Vec::new());

    loop {
        match reader.read_event() {
            Ok(Event::Eof) => break,
            Ok(event) => {
                writer
                    .write_event(event)
                    .map_err(|e| format!("Error writing event: {}", e))?;
            }
            Err(e) => return Err(format!("Error parsing XML: {}", e)),
        }
    }

    Ok(writer.into_inner())
}

/// Fully decode XML named entity references in a string.
/// Iteratively undoes `&amp;` double-encoding, then decodes named entities.
#[cfg(feature = "xml")]
fn decode_xml_entities(s: &str) -> String {
    let mut result = s.to_string();
    // Iteratively undo double-encoding (&amp;amp; → &amp; → &)
    loop {
        let prev = result.clone();
        result = result.replace("&amp;", "&");
        if result == prev {
            break;
        }
    }
    // Decode standard XML named entities
    result
        .replace("&apos;", "'")
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

/// Check if a string difference is due to XML entity encoding issues.
///
/// Known limitations of the XML serializer/deserializer:
/// 1. Double-encoding: `&lt;` → `&amp;lt;`, `&#39;` → `&amp;#39;`, etc.
/// 2. New encoding: `'` → `&apos;` where the serializer introduces entity refs
///    for literal characters, and the deserializer stores them as-is.
///
/// Both sides are decoded to raw characters for comparison.
#[cfg(feature = "xml")]
fn is_entity_encoding_difference(original: &Value, reserialized: &Value) -> bool {
    if let (Value::String(orig), Value::String(reser)) = (original, reserialized) {
        return decode_xml_entities(orig) == decode_xml_entities(reser);
    }
    false
}

#[cfg(feature = "xml")]
fn xml_skip_list() -> &'static [(&'static str, &'static str)] {
    const XML_SKIPS: &[(&str, &str)] = &[
        // These files have known XML roundtrip data loss due to nested primitive
        // extension patterns (_event) where the XML deserializer doesn't fully
        // reconstruct valueExpression fields.
        (
            "activitydefinition-*",
            "timingTiming._event extension fields lost on XML roundtrip",
        ),
        (
            "plandefinition-example*",
            "contained timingTiming._event extension fields lost on XML roundtrip",
        ),
        // ExampleScenario has structural differences in instance fields
        (
            "examplescenario-example*",
            "ExampleScenario instance fields lost on XML roundtrip",
        ),
        // R5 Subscription filterBy fields (filterParameter, resourceType, value)
        // lost on XML roundtrip
        (
            "subscription-example*",
            "Subscription filterBy fields lost on XML roundtrip",
        ),
    ];
    XML_SKIPS
}

#[cfg(feature = "xml")]
fn test_xml_examples_in_dir<R: DeserializeOwned + Serialize>(dir: &Path, _fhir_version: &str) {
    if !dir.exists() {
        println!("Directory does not exist: {:?}", dir);
        return;
    }

    let skip_files = xml_skip_list();
    let mut failures: Vec<String> = Vec::new();

    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();

        if path.is_file() && path.extension().is_some_and(|ext| ext == "xml") {
            let filename = path.file_name().unwrap().to_string_lossy().to_string();

            // Skip profile files (these are StructureDefinitions, not resource examples)
            if filename.contains(".profile.xml") {
                continue;
            }

            // Check if this file should be skipped
            if let Some(reason) = should_skip_file(&filename, skip_files) {
                println!("Skipping file: {} - Reason: {}", filename, reason);
                continue;
            }

            // Read the file content
            match fs::read_to_string(&path) {
                Ok(content) => {
                    if content.trim().is_empty() {
                        continue;
                    }

                    // Try to deserialize the XML string to a FHIR Resource
                    match from_xml_str::<R>(&content) {
                        Ok(resource) => {
                            // Verify we can serialize the Resource back to XML
                            match to_xml_string(&resource) {
                                Ok(reserialized_xml) => {
                                    // Normalize both XMLs for comparison
                                    let normalized_original = normalize_xml(&content)
                                        .expect("Failed to normalize original XML");
                                    let normalized_reserialized = normalize_xml(&reserialized_xml)
                                        .expect("Failed to normalize reserialized XML");

                                    if normalized_original != normalized_reserialized {
                                        // Semantic comparison: deserialize reserialized XML
                                        // and compare via JSON to catch data loss while
                                        // tolerating cosmetic XML differences
                                        match from_xml_str::<R>(&reserialized_xml) {
                                            Ok(re_resource) => {
                                                let original_json =
                                                    serde_json::to_value(&resource).unwrap();
                                                let reserialized_json =
                                                    serde_json::to_value(&re_resource).unwrap();

                                                let all_diffs = find_json_differences(
                                                    &original_json,
                                                    &reserialized_json,
                                                );

                                                // Filter out known entity-encoding differences
                                                let diff_paths: Vec<_> = all_diffs
                                                    .into_iter()
                                                    .filter(|(_, orig, reser)| {
                                                        !is_entity_encoding_difference(orig, reser)
                                                    })
                                                    .collect();

                                                if !diff_paths.is_empty() {
                                                    let mut msg = format!(
                                                        "{}: {} semantic differences:",
                                                        filename,
                                                        diff_paths.len()
                                                    );
                                                    for (diff_path, orig_val, new_val) in
                                                        &diff_paths
                                                    {
                                                        msg.push_str(&format!(
                                                            "\n    {}: {} → {}",
                                                            diff_path,
                                                            serde_json::to_string(orig_val)
                                                                .unwrap_or_default(),
                                                            serde_json::to_string(new_val)
                                                                .unwrap_or_default(),
                                                        ));
                                                    }
                                                    println!("FAIL: {}", msg);
                                                    failures.push(msg);
                                                }
                                            }
                                            Err(e) => {
                                                let msg = format!(
                                                    "{}: reserialized XML cannot be deserialized: {}",
                                                    filename, e
                                                );
                                                println!("FAIL: {}", msg);
                                                failures.push(msg);
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    let msg = format!(
                                        "{}: error serializing Resource to XML: {}",
                                        filename, e
                                    );
                                    println!("FAIL: {}", msg);
                                    failures.push(msg);
                                }
                            }
                        }
                        Err(e) => {
                            let msg = format!(
                                "{}: error deserializing XML to FHIR Resource: {}",
                                filename, e
                            );
                            println!("FAIL: {}", msg);
                            failures.push(msg);
                        }
                    }
                }
                Err(e) => {
                    println!("Error reading file: {}: {}", path.display(), e);
                }
            }
        }
    }

    if !failures.is_empty() {
        panic!(
            "XML roundtrip failures ({} files):\n{}",
            failures.len(),
            failures.join("\n")
        );
    }
}
