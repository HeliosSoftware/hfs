use std::fs;
use std::fs::File;
use std::io::copy;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

fn main() {
    // Only download R6 search parameters when R6 feature is enabled
    if !cfg!(feature = "R6") {
        return;
    }

    // Skip R6 download if skip-r6-download feature is enabled or DOCS_RS env var is set
    // This allows docs.rs builds and `cargo clippy --all-features` to succeed without downloading
    if cfg!(feature = "skip-r6-download") || std::env::var("DOCS_RS").is_ok() {
        return;
    }

    // Target path is the workspace data directory
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let data_dir = manifest_dir
        .parent() // crates/
        .and_then(|p| p.parent()) // workspace root
        .map(|p| p.join("data"))
        .expect("Failed to find workspace data directory");

    // Create the data directory if it doesn't exist
    fs::create_dir_all(&data_dir).expect("Failed to create data directory");

    let output_path = data_dir.join("search-parameters-r6.json");

    // Check if the file already exists and is recent (skip download if less than 24 hours old)
    if let Ok(metadata) = fs::metadata(&output_path) {
        if let Ok(modified) = metadata.modified() {
            if let Ok(duration) = modified.elapsed() {
                // Skip if file was modified less than 24 hours ago
                if duration.as_secs() < 86400 {
                    println!(
                        "cargo:warning=R6 search parameters file is recent, skipping download"
                    );
                    return;
                }
            }
        }
    }

    println!("cargo:warning=Downloading R6 search parameters from HL7 build server");

    let url = "https://build.fhir.org/search-parameters.json";

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

        match download_file(&client, url, &output_path) {
            Ok(bytes) => {
                println!(
                    "cargo:warning=Downloaded R6 search parameters ({} bytes) to {}",
                    bytes,
                    output_path.display()
                );
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
        // Don't fail the build - just warn. The server can still function with
        // the checked-in R4/R4B/R5 files or with minimal fallback params.
        println!(
            "cargo:warning=Failed to download R6 search parameters after {} attempts: {}. \
             The server will use minimal fallback parameters for R6.",
            MAX_RETRIES, error
        );
    }
}

fn download_file(
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

    // Verify content type is JSON
    if let Some(content_type) = response.headers().get("content-type") {
        let content_type_str = content_type.to_str().unwrap_or("");
        if !content_type_str.contains("json") && !content_type_str.contains("fhir") {
            return Err(format!(
                "Expected JSON file but got content-type: {}",
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

    // Verify the file is valid JSON by checking it starts with '{'
    let content = fs::read_to_string(output_path)
        .map_err(|e| format!("Failed to read downloaded file: {}", e))?;

    let content_trimmed = content.trim();
    if !content_trimmed.starts_with('{') {
        return Err(format!(
            "Downloaded file does not appear to be valid JSON (starts with: {})",
            &content_trimmed[..content_trimmed.len().min(50)]
        ));
    }

    Ok(bytes_copied)
}
