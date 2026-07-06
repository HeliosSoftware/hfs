//! StructureDefinition → FhirSchema conversion (fhir2schema).
//!
//! Consumes StructureDefinitions as **raw JSON** — deliberately not the
//! fhir-gen bootstrap model (whose build.rs performs the R6 download) and not
//! the typed models (uploaded profiles arrive as `Value` from the REST
//! extractor). Works from `snapshot.element` when present, otherwise
//! `differential.element`: because FHIR Schema is differential-by-design and
//! validation layers schemas cooperatively, **no snapshot generation is ever
//! needed** — sparse differential paths simply produce empty intermediate
//! nodes that the base layers fill in at validation time.
//!
//! Mapping summary (pinned by `tests/converter_tests.rs` goldens):
//!
//! | ElementDefinition | FhirSchema |
//! |---|---|
//! | `max == "0"` | parent `excluded` (element omitted) |
//! | `min >= 1` | parent `required` (choice base name for `foo[x]`) |
//! | `base.max` (else `max`) `*`/`>1` | `array: true` + numeric `min`/`max` |
//! | multi-type / `foo[x]` | `choices` declarer + one `choiceOf` branch per type |
//! | `contentReference: "#A.b"` | `elementReference: ["A", "elements", "b"]` |
//! | `slicing.discriminator` | `slicing.slices[].match` (pattern) — see `slicing` |
//! | extension slice + type profile | parent `extensions` sugar |
//! | `binding` | `{valueSet, strength}` carried for all strengths |
//! | `constraint[]` | `constraints` map (`ele-1`/`ext-1` dropped off non-root elements — they are enforced once via the `Element`/`Extension` type schemas) |
//! | `fixed[x]` / `pattern[x]` | `fixed` / `pattern` |
//! | `type[].targetProfile` | `refers` |
//! | primitive `<t>.value` regex extension | schema `regex` |
//!
//! Additionally, `resourceType: {type: "code"}` is injected into resource
//! schemas with `derivation != constraint` (StructureDefinitions never
//! declare `resourceType`; profiles inherit the injected element through
//! their base chain).

mod primitives;
mod slicing;
mod tree;

use crate::schema::FhirSchema;
use serde::Deserialize;
use serde_json::Value;
use std::fmt;

/// A successful conversion: the schema plus non-fatal warnings (unmappable
/// discriminators, dropped slice minimums, missing primitive regexes, ...).
#[derive(Debug)]
pub struct Conversion {
    pub schema: FhirSchema,
    pub warnings: Vec<String>,
}

/// A conversion failure.
#[derive(Debug)]
pub enum ConvertError {
    /// The input is not a StructureDefinition.
    NotAStructureDefinition,
    /// The StructureDefinition is missing a required part.
    Missing(&'static str),
    /// The element list is malformed.
    Malformed(String),
}

impl fmt::Display for ConvertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConvertError::NotAStructureDefinition => {
                write!(f, "input is not a StructureDefinition")
            }
            ConvertError::Missing(what) => write!(f, "StructureDefinition is missing {what}"),
            ConvertError::Malformed(detail) => write!(f, "malformed StructureDefinition: {detail}"),
        }
    }
}

impl std::error::Error for ConvertError {}

// ---------------------------------------------------------------------
// Lightweight ElementDefinition model (tolerant serde over raw JSON).
// ---------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub(crate) struct Ed {
    pub id: Option<String>,
    pub path: String,
    #[serde(rename = "sliceName")]
    pub slice_name: Option<String>,
    pub min: Option<u64>,
    pub max: Option<String>,
    pub base: Option<EdBase>,
    #[serde(rename = "type", default)]
    pub types: Vec<EdType>,
    #[serde(rename = "contentReference")]
    pub content_reference: Option<String>,
    pub slicing: Option<EdSlicing>,
    pub binding: Option<EdBinding>,
    #[serde(default)]
    pub constraint: Vec<EdConstraint>,
    /// Everything else — scanned for `fixed[x]` / `pattern[x]`.
    #[serde(flatten)]
    pub rest: serde_json::Map<String, Value>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct EdBase {
    pub max: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct EdType {
    pub code: String,
    #[serde(default)]
    pub profile: Vec<String>,
    #[serde(rename = "targetProfile", default)]
    pub target_profile: Vec<String>,
    #[serde(default)]
    pub extension: Vec<EdTypeExtension>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct EdTypeExtension {
    pub url: String,
    #[serde(rename = "valueString")]
    pub value_string: Option<String>,
    #[serde(rename = "valueUrl")]
    pub value_url: Option<String>,
    #[serde(rename = "valueUri")]
    pub value_uri: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct EdSlicing {
    #[serde(default)]
    pub discriminator: Vec<EdDiscriminator>,
    pub rules: Option<String>,
    pub ordered: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct EdDiscriminator {
    #[serde(rename = "type")]
    pub type_: String,
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct EdBinding {
    pub strength: Option<String>,
    #[serde(rename = "valueSet")]
    pub value_set: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct EdConstraint {
    pub key: String,
    pub severity: Option<String>,
    pub human: Option<String>,
    pub expression: Option<String>,
}

/// The FHIR type-code extensions carried on `type[].extension`.
const REGEX_EXTENSION: &str = "http://hl7.org/fhir/StructureDefinition/regex";
const FHIR_TYPE_EXTENSION: &str =
    "http://hl7.org/fhir/StructureDefinition/structuredefinition-fhir-type";

impl EdType {
    /// The effective type code: FHIRPath system types
    /// (`http://hl7.org/fhirpath/System.String`, ...) resolve through the
    /// `structuredefinition-fhir-type` extension, falling back to the
    /// lower-cased suffix.
    pub(crate) fn effective_code(&self) -> String {
        if let Some(suffix) = self.code.strip_prefix("http://hl7.org/fhirpath/System.") {
            for ext in &self.extension {
                if ext.url == FHIR_TYPE_EXTENSION
                    && let Some(v) = ext
                        .value_url
                        .as_deref()
                        .or(ext.value_uri.as_deref())
                        .or(ext.value_string.as_deref())
                {
                    return v.to_string();
                }
            }
            let mut chars = suffix.chars();
            return match chars.next() {
                Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
                None => suffix.to_string(),
            };
        }
        self.code.clone()
    }

    /// The regex constraint carried on primitive value elements.
    pub(crate) fn regex(&self) -> Option<&str> {
        self.extension
            .iter()
            .find(|e| e.url == REGEX_EXTENSION)
            .and_then(|e| e.value_string.as_deref())
    }
}

/// Convert a StructureDefinition (raw JSON, snapshot or differential) to a
/// FHIR Schema.
pub fn convert(sd: &Value) -> Result<Conversion, ConvertError> {
    if sd.get("resourceType").and_then(Value::as_str) != Some("StructureDefinition") {
        return Err(ConvertError::NotAStructureDefinition);
    }

    let get_str = |key: &str| sd.get(key).and_then(Value::as_str).map(str::to_string);
    let url = get_str("url").ok_or(ConvertError::Missing("url"))?;
    let name = get_str("name");
    let sd_type = get_str("type").ok_or(ConvertError::Missing("type"))?;
    let kind_raw = get_str("kind").ok_or(ConvertError::Missing("kind"))?;
    let derivation = get_str("derivation");
    let base = get_str("baseDefinition");

    let element_values = sd
        .get("snapshot")
        .and_then(|s| s.get("element"))
        .or_else(|| sd.get("differential").and_then(|d| d.get("element")))
        .and_then(Value::as_array)
        .ok_or(ConvertError::Missing("snapshot.element or differential.element"))?;

    let elements: Vec<Ed> = element_values
        .iter()
        .map(|v| {
            serde_json::from_value(v.clone())
                .map_err(|e| ConvertError::Malformed(format!("element: {e}")))
        })
        .collect::<Result<_, _>>()?;

    let mut warnings: Vec<String> = Vec::new();

    // Primitive types produce a leaf schema with a value regex.
    if kind_raw == "primitive-type" {
        let schema = primitives::convert_primitive(
            url, name, &sd_type, derivation, &elements, &mut warnings,
        );
        return Ok(Conversion { schema, warnings });
    }

    // `Extension`-constraining SDs get the dedicated kind.
    let kind = if sd_type == "Extension" && derivation.as_deref() == Some("constraint") {
        "extension".to_string()
    } else {
        kind_raw.clone()
    };

    // Build the nested element tree by navigating each ED's id segments.
    let mut root = tree::Node::new(sd_type.clone());
    for ed in &elements {
        // Navigation is id-based; hand-authored differentials may omit ids,
        // in which case the slice position is reconstructed from sliceName.
        let synthesized;
        let id = match (&ed.id, &ed.slice_name) {
            (Some(id), _) => id.as_str(),
            (None, Some(slice)) => {
                synthesized = format!("{}:{slice}", ed.path);
                &synthesized
            }
            (None, None) => &ed.path,
        };
        let segments = tree::parse_segments(id)
            .map_err(|e| ConvertError::Malformed(format!("element id '{id}': {e}")))?;
        if segments.is_empty() {
            // Root element: constraints attach at schema level (all kept —
            // dom-* live here).
            tree::apply_constraints(&mut root.schema, &ed.constraint, true);
            continue;
        }
        tree::apply(&mut root, &segments, ed, &mut warnings);
    }

    let mut schema = tree::finalize(root, &mut warnings).unwrap_or_default();

    // Header fields.
    schema.url = Some(url);
    schema.name = name;
    schema.base = base;
    schema.kind = Some(kind);
    schema.derivation = derivation.clone();
    schema.type_ = Some(sd_type);

    // StructureDefinitions never declare `resourceType`; inject it for
    // resource specializations (profiles inherit it via their base chain).
    if kind_raw == "resource" && derivation.as_deref() != Some("constraint") {
        let mut elements = indexmap::IndexMap::new();
        elements.insert(
            "resourceType".to_string(),
            std::sync::Arc::new(FhirSchema {
                type_: Some("code".to_string()),
                ..Default::default()
            }),
        );
        if let Some(existing) = schema.elements.take() {
            elements.extend(existing);
        }
        schema.elements = Some(elements);
    }

    Ok(Conversion { schema, warnings })
}
