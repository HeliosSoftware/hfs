


use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use serde_json::Result;

use crate::initial_fhir_model::{Bundle, StructureDefinition};

pub fn parse_structure_definitions<P: AsRef<Path>>(path: P) -> Result<Bundle> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    serde_json::from_reader(reader)
}
