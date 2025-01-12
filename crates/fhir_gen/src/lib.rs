
use std::fs::File;
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_parse_structure_definitions() {
        let test_file = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("test_data")
            .join("simple_bundle.json");
        
        let bundle = parse_structure_definitions(test_file).unwrap();
        assert!(!bundle.entry.is_empty());
    }
}
use std::io::BufReader;
use std::path::Path;
use serde_json::Result;

use crate::initial_fhir_model::{Bundle, StructureDefinition};

pub fn parse_structure_definitions<P: AsRef<Path>>(path: P) -> Result<Bundle> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    serde_json::from_reader(reader)
}
