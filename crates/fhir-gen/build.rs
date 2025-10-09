use serde_json::Value;
use std::fs;
use std::fs::File;
use std::io::copy;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;
use zip::ZipArchive;

fn main() {
    // Check if R6 feature is enabled
    if !cfg!(feature = "R6") {
        return;
    }

    // Skip R6 download if skip-r6-download feature is enabled or DOCS_RS env var is set
    // This allows docs.rs builds to succeed using the checked-in R6 resources
    if cfg!(feature = "skip-r6-download") || std::env::var("DOCS_RS").is_ok() {
        println!("cargo:warning=Skipping R6 download (using checked-in resources)");
        return;
    }

    let resources_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/R6");

    // Create the resources directory if it doesn't exist
    fs::create_dir_all(&resources_dir).expect("Failed to create resources directory");

    let url = "https://build.fhir.org/definitions.json.zip";

    let output_path = resources_dir.join("definitions.json.zip");

    println!("Downloading FHIR definitions...");

    // Create a client with custom headers and timeout
    let client = reqwest::blocking::Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko)")
        .timeout(Duration::from_secs(120))
        .build()
        .expect("Failed to create HTTP client");

    // Try downloading with retries
    const MAX_RETRIES: u32 = 3;
    let mut last_error = None;

    for attempt in 1..=MAX_RETRIES {
        println!("Download attempt {} of {}", attempt, MAX_RETRIES);

        match download_with_retry(&client, url, &output_path) {
            Ok(bytes) => {
                println!("Downloaded {} bytes", bytes);
                last_error = None;
                break;
            }
            Err(e) => {
                println!("Attempt {} failed: {}", attempt, e);
                last_error = Some(e);

                if attempt < MAX_RETRIES {
                    let wait_time = Duration::from_secs(5 * attempt as u64);
                    println!("Waiting {:?} before retry...", wait_time);
                    thread::sleep(wait_time);
                }
            }
        }
    }

    if let Some(error) = last_error {
        panic!(
            "Failed to download file after {} attempts: {}",
            MAX_RETRIES, error
        );
    }

    // Verify and extract the downloaded file
    let file = fs::File::open(&output_path).expect("Failed to open downloaded file");
    let metadata = file.metadata().expect("Failed to get file metadata");
    println!("File size on disk: {} bytes", metadata.len());

    if metadata.len() == 0 {
        panic!("Downloaded file is empty!");
    }

    let mut archive = ZipArchive::new(file).unwrap();

    // Extract everything
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).unwrap();
        let outpath = resources_dir.join(file.mangled_name());

        if file.name().ends_with('/') {
            fs::create_dir_all(&outpath).unwrap();
        } else {
            if let Some(p) = outpath.parent() {
                fs::create_dir_all(p).unwrap();
            }
            let mut outfile = fs::File::create(&outpath).unwrap();
            std::io::copy(&mut file, &mut outfile).unwrap();
        }
    }

    // Delete the zip file after extraction
    fs::remove_file(output_path).expect("Failed to delete zip file");

    // Insert ViewDefinition into profiles-resources.json
    insert_view_definition(&resources_dir).expect("Failed to insert ViewDefinition");

    println!("FHIR definitions downloaded successfully");
}

fn download_with_retry(
    client: &reqwest::blocking::Client,
    url: &str,
    output_path: &PathBuf,
) -> Result<u64, String> {
    // Download the file
    let response = client
        .get(url)
        .send()
        .map_err(|e| format!("Failed to GET from url: {}", e))?;

    // Check if request was successful
    if !response.status().is_success() {
        return Err(format!(
            "Download failed with status: {} for URL: {}",
            response.status(),
            url
        ));
    }

    // Verify content type
    if let Some(content_type) = response.headers().get("content-type") {
        let content_type_str = content_type.to_str().unwrap_or("");
        if !content_type_str.contains("zip") {
            return Err(format!(
                "Expected ZIP file but got content-type: {}",
                content_type_str
            ));
        }
    }

    let mut response = response;

    // Create the file
    let mut downloaded_file =
        File::create(output_path).map_err(|e| format!("Failed to create the file: {}", e))?;

    let bytes_copied = copy(&mut response, &mut downloaded_file)
        .map_err(|e| format!("Failed to copy the file: {}", e))?;

    // Ensure file is written to disk
    downloaded_file
        .sync_all()
        .map_err(|e| format!("Failed to flush file to disk: {}", e))?;

    Ok(bytes_copied)
}

/// Inserts the ViewDefinition resource into profiles-resources.json for R6 builds.
///
/// This is a temporary workaround because the ViewDefinition FHIR Resource is not yet
/// included in the latest R6 build from HL7's build server. This function should be
/// removed once ViewDefinition is officially added to the R6 specification.
fn insert_view_definition(resources_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let profiles_resources_path = resources_dir.join("profiles-resources.json");
    let view_definition_path = resources_dir.join("view-definition-bundle-entry.json");

    // Read the existing profiles-resources.json
    let profiles_content = fs::read_to_string(&profiles_resources_path)?;
    let mut profiles_json: Value = serde_json::from_str(&profiles_content)?;

    // Read the ViewDefinition bundle entry to be inserted
    let view_definition_content = fs::read_to_string(&view_definition_path)?;
    let view_definition_entry: Value = serde_json::from_str(&view_definition_content)?;

    // Insert the ViewDefinition bundle entry at the end of the entry array
    if let Some(entry_array) = profiles_json["entry"].as_array_mut() {
        entry_array.push(view_definition_entry);
        println!("Inserted ViewDefinition into profiles-resources.json");
    } else {
        return Err(
            "profiles-resources.json does not have expected 'entry' array structure".into(),
        );
    }

    // Write the modified JSON back to the file
    let updated_content = serde_json::to_string_pretty(&profiles_json)?;
    fs::write(&profiles_resources_path, updated_content)?;

    Ok(())
}
