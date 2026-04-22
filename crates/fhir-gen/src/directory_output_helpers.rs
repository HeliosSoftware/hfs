use std::io;
use std::io::Write;
use std::path::Path;

pub(crate) fn module_file_stem(name: &str) -> String {
    // Convert FHIR type names like `Observation`, `MedicationRequest`, `xhtml`, `unsignedInt`
    // into snake_case Rust module file names.
    let mut out = String::new();
    let mut prev_is_lower_or_digit = false;

    for ch in name.chars() {
        let is_upper = ch.is_ascii_uppercase();
        let is_lower = ch.is_ascii_lowercase();
        let is_digit = ch.is_ascii_digit();

        if ch == '-' || ch == ' ' {
            if !out.ends_with('_') {
                out.push('_');
            }
            prev_is_lower_or_digit = false;
            continue;
        }

        if is_upper {
            if prev_is_lower_or_digit && !out.ends_with('_') {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
            prev_is_lower_or_digit = true;
            continue;
        }

        if is_lower || is_digit {
            out.push(ch);
            prev_is_lower_or_digit = true;
            continue;
        }

        // For any other char, replace with underscore.
        if !out.ends_with('_') {
            out.push('_');
        }
        prev_is_lower_or_digit = false;
    }

    // Trim leading/trailing underscores
    while out.starts_with('_') {
        out.remove(0);
    }
    while out.ends_with('_') {
        out.pop();
    }

    if out.is_empty() {
        "type_".to_string()
    } else {
        out
    }
}

pub(crate) fn write_mod_index(mod_rs_path: &Path, modules: &[String]) -> io::Result<()> {
    // Deterministic ordering
    let mut modules = modules.to_vec();
    modules.sort();
    modules.dedup();

    let mut file = std::fs::File::create(mod_rs_path)?;
    for m in modules {
        writeln!(file, "pub mod {m};")?;
        writeln!(file, "pub use {m}::*;")?;
        writeln!(file)?;
    }
    Ok(())
}
