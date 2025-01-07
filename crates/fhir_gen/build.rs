use std::fs;
use std::fs::File;
use std::path::Path;
use std::io::copy;
use zip::ZipArchive;

fn main() {

    println!("Start");

    // Create resources directory if it doesn't exist
    let resources_dir = Path::new("resources/R4");
    fs::create_dir_all(resources_dir).expect("Failed to create resources directory");

    let url = "https://hl7.org/fhir/R4/definitions.json.zip";
    let output_path = resources_dir.join("definitions.json.zip");

    println!("Downloading FHIR definitions...");

    // Download the file
    let mut response = reqwest::blocking::get(url)
        .expect("Failed to GET from url");
    
    // Create the file
    let mut downloaded_file = File::create(output_path.clone())
        .expect("Failed to create the file");

    copy(&mut response, &mut downloaded_file)
        .expect("Failed to copy the file");

    let file = fs::File::open(output_path).unwrap();

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

    println!("FHIR definitions downloaded successfully");

}
