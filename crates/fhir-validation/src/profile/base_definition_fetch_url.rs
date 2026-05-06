//! Rewrite canonical `StructureDefinition.baseDefinition` URLs to **static JSON** paths where
//! publishers follow predictable conventions (NDHM `…-Type.json`, HL7 `…/R4/patient.profile.json`).

const SNAPSHOT_BASE_VERSION_EXT: &str =
    "http://hl7.org/fhir/tools/StructureDefinition/snapshot-base-version";

/// Map `StructureDefinition.snapshot.extension` `snapshot-base-version` `valueString` (e.g.
/// `4.0.1`) to the HL7 web package segment (`R4`, `R4B`, `R5`, …) used under `https://hl7.org/fhir/`.
pub fn hl7_web_package_segment_from_snapshot_base(version: &str) -> &'static str {
    let v = version.trim();
    if v.starts_with("4.0.") || v == "4.0" {
        return "R4";
    }
    if v.starts_with("4.1.") || v.starts_with("4.3.") {
        return "R4B";
    }
    if v.starts_with("4.") {
        return "R4";
    }
    if v.starts_with("5.") {
        return "R5";
    }
    if v.starts_with("6.") {
        return "R6";
    }
    "R4"
}

/// If `sd` has `snapshot.extension` with [`SNAPSHOT_BASE_VERSION_EXT`], return its `valueString`.
pub fn parse_snapshot_base_version_from_sd_json(
    sd: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    let snap = sd.get("snapshot")?.as_object()?;
    let exts = snap.get("extension")?.as_array()?;
    for e in exts {
        let o = e.as_object()?;
        if o.get("url").and_then(|u| u.as_str()) == Some(SNAPSHOT_BASE_VERSION_EXT) {
            return o
                .get("valueString")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
        }
    }
    None
}

/// Best-effort URL to `GET` for a `StructureDefinition` when the canonical `baseDefinition` URL
/// serves HTML or otherwise is not raw JSON.
///
/// * **NDHM** (`nrces.in/ndhm/fhir/r4/…/StructureDefinition/{Type}`) →
///   `…/StructureDefinition-{Type}.json`
/// * **HL7** (`http(s)://hl7.org/fhir/StructureDefinition/{Type}`) →
///   `https://hl7.org/fhir/{segment}/{type}.profile.json` where `segment` comes from
///   [`hl7_web_package_segment_from_snapshot_base`] when `snapshot_base_version` is set (e.g.
///   `4.0.1` → `R4`); otherwise defaults to **`R4`**.
///
/// If no rule matches, returns `base_definition_canonical` unchanged (strip `|` version only).
pub fn structure_definition_json_fetch_url(
    base_definition_canonical: &str,
    snapshot_base_version: Option<&str>,
) -> String {
    let base = base_definition_canonical
        .split('|')
        .next()
        .unwrap_or(base_definition_canonical)
        .trim();

    if base.ends_with(".json") || base.ends_with(".profile.json") {
        return base.to_string();
    }

    for prefix in [
        "https://nrces.in/ndhm/fhir/r4/StructureDefinition/",
        "http://nrces.in/ndhm/fhir/r4/StructureDefinition/",
    ] {
        if let Some(rest) = base.strip_prefix(prefix) {
            let type_name = rest.split('/').next().unwrap_or(rest);
            if type_name.is_empty() || type_name.contains('.') {
                break;
            }
            return format!("https://nrces.in/ndhm/fhir/r4/StructureDefinition-{type_name}.json");
        }
    }

    const HL7_HTTP: &str = "http://hl7.org/fhir/StructureDefinition/";
    const HL7_HTTPS: &str = "https://hl7.org/fhir/StructureDefinition/";
    let lc = base.to_ascii_lowercase();
    if lc.starts_with("http://hl7.org/fhir/structuredefinition/")
        || lc.starts_with("https://hl7.org/fhir/structuredefinition/")
    {
        let rest = if lc.starts_with("https://hl7.org/fhir/structuredefinition/") {
            &base[HL7_HTTPS.len()..]
        } else {
            &base[HL7_HTTP.len()..]
        };
        let type_name = rest.split('/').next().unwrap_or(rest);
        if type_name.is_empty() || type_name.contains('.') {
            return base.to_string();
        }
        let seg = snapshot_base_version
            .map(hl7_web_package_segment_from_snapshot_base)
            .unwrap_or("R4");
        let lc_type = type_name.to_ascii_lowercase();
        return format!("https://hl7.org/fhir/{seg}/{lc_type}.profile.json");
    }

    base.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ndhm_patient_canonical_to_json_file() {
        assert_eq!(
            structure_definition_json_fetch_url(
                "https://nrces.in/ndhm/fhir/r4/StructureDefinition/Patient",
                None
            ),
            "https://nrces.in/ndhm/fhir/r4/StructureDefinition-Patient.json"
        );
    }

    #[test]
    fn hl7_core_patient_uses_r4_when_version_hint_4_0_1() {
        assert_eq!(
            structure_definition_json_fetch_url(
                "http://hl7.org/fhir/StructureDefinition/Patient",
                Some("4.0.1")
            ),
            "https://hl7.org/fhir/R4/patient.profile.json"
        );
    }

    #[test]
    fn hl7_core_defaults_to_r4_without_hint() {
        assert_eq!(
            structure_definition_json_fetch_url(
                "http://hl7.org/fhir/StructureDefinition/Patient",
                None
            ),
            "https://hl7.org/fhir/R4/patient.profile.json"
        );
    }

    #[test]
    fn strips_pipe_version_on_input() {
        assert_eq!(
            structure_definition_json_fetch_url(
                "https://nrces.in/ndhm/fhir/r4/StructureDefinition/Patient|1.0",
                None
            ),
            "https://nrces.in/ndhm/fhir/r4/StructureDefinition-Patient.json"
        );
    }
}
