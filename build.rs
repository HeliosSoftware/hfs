use std::fs;
use std::path::Path;

#[tokio::main]
async fn main() {
    // Create resources directory if it doesn't exist
    let resources_dir = Path::new("crates/fhir_gen/resources/R4");
    fs::create_dir_all(resources_dir).expect("Failed to create resources directory");

    let url = "https://hl7.org/fhir/R4/definitions.json.zip";
    let output_path = resources_dir.join("definitions.json.zip");

    // Only download if file doesn't exist
    if !output_path.exists() {
        println!("Downloading FHIR definitions...");
        
        // Download the file
        let response = reqwest::get(url)
            .await
            .expect("Failed to GET from url");

        let bytes = response.bytes()
            .await
            .expect("Failed to get response bytes");

        // Write to file
        fs::write(&output_path, bytes)
            .expect("Failed to write zip file");
            
        println!("FHIR definitions downloaded successfully");
    }

    // Tell Cargo to rerun if the downloaded file changes or is deleted
    println!("cargo:rerun-if-changed=crates/fhir_gen/resources/R4/definitions.json.zip");
}
