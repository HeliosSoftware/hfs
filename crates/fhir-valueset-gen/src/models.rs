use helios_fhir_gen::initial_fhir_model::{
    CodeableConcept, Coding, ContactDetail, Extension, Identifier, Meta, Narrative, Range,
    Reference,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

#[derive(Debug, Serialize, Deserialize)]
pub struct Quantity {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<Decimal>,
    pub comparator: Option<String>,
    pub unit: Option<String>,
    pub system: Option<String>,
    pub code: Option<String>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct UsageContext {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub code: Option<Coding>,
    #[serde(flatten)]
    pub value: UsageContextValue,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum UsageContextValue {
    #[serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    #[serde(rename = "valueQuantity")]
    Quantity(Quantity),
    #[serde(rename = "valueRange")]
    Range(Range),
    #[serde(rename = "valueReference")]
    Reference(Reference),
}
#[derive(Debug, Deserialize)]
pub struct Bundle {
    #[serde(rename = "resourceType")]
    pub resource_type: Option<String>, // "Bundle"
    pub entry: Option<Vec<BundleEntry>>,
}

#[derive(Debug, Deserialize)]
pub struct BundleEntry {
    #[serde(rename = "fullUrl")]
    pub full_url: Option<String>,
    pub resource: Option<JsonValue>,
}
/// We don't need a full FHIR Resource enum here. Contained resources are not used
/// by the terminology generator, so keep them as untyped JSON to avoid circular deps.
pub type ContainedResource = JsonValue;

#[derive(Debug, Serialize, Deserialize)]
pub struct CodeSystem {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<String>,
    pub language: Option<String>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<ContainedResource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<String>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub status: Option<String>,
    pub experimental: Option<bool>,
    pub date: Option<String>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<String>,
    #[serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<String>,
    pub copyright: Option<String>,
    #[serde(rename = "caseSensitive")]
    pub case_sensitive: Option<bool>,
    #[serde(rename = "valueSet")]
    pub value_set: Option<String>,
    #[serde(rename = "hierarchyMeaning")]
    pub hierarchy_meaning: Option<String>,
    pub compositional: Option<bool>,
    #[serde(rename = "versionNeeded")]
    pub version_needed: Option<bool>,
    pub content: Option<String>,
    pub supplements: Option<String>,
    pub count: Option<u32>,
    pub filter: Option<Vec<CodeSystemFilter>>,
    pub property: Option<Vec<CodeSystemProperty>>,
    pub concept: Option<Vec<CodeSystemConcept>>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct CodeSystemConcept {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: String,
    pub display: Option<String>,
    pub definition: Option<String>,
    pub designation: Option<Vec<CodeSystemConceptDesignation>>,
    pub property: Option<Vec<CodeSystemConceptProperty>>,
    pub concept: Option<Vec<CodeSystemConcept>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CodeSystemConceptDesignation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub language: Option<String>,
    #[serde(rename = "use")]
    pub r#use: Option<Coding>,
    pub value: String,
}

/// Choice of types for the value\[x\] field in CodeSystemConceptProperty
#[derive(Debug, Serialize, Deserialize)]
pub enum CodeSystemConceptPropertyValue {
    /// Variant accepting the String type.
    #[serde(rename = "valueCode")]
    Code(String),
    /// Variant accepting the Coding type.
    #[serde(rename = "valueCoding")]
    Coding(Coding),
    /// Variant accepting the String type.
    #[serde(rename = "valueString")]
    String(String),
    /// Variant accepting the i32 type.
    #[serde(rename = "valueInteger")]
    Integer(i32),
    /// Variant accepting the bool type.
    #[serde(rename = "valueBoolean")]
    Boolean(bool),
    /// Variant accepting the String type.
    #[serde(rename = "valueDateTime")]
    DateTime(String),
    /// Variant accepting the String type.
    #[serde(rename = "valueDecimal")]
    Decimal(String),
}
#[derive(Debug, Serialize, Deserialize)]
pub struct CodeSystemConceptProperty {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: String,
    #[serde(flatten)]
    pub value: Option<CodeSystemConceptPropertyValue>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CodeSystemFilter {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: String,
    pub description: Option<String>,
    pub operator: Option<Vec<String>>,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CodeSystemProperty {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: String,
    pub uri: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub r#type: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ValueSet {
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<String>,
    pub language: Option<String>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<ContainedResource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<String>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub status: Option<String>,
    pub experimental: Option<bool>,
    pub date: Option<String>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<String>,
    #[serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub immutable: Option<bool>,
    pub purpose: Option<String>,
    pub copyright: Option<String>,
    pub compose: Option<ValueSetCompose>,
    pub expansion: Option<ValueSetExpansion>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct ValueSetCompose {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "lockedDate")]
    pub locked_date: Option<String>,
    pub inactive: Option<bool>,
    pub include: Option<Vec<ValueSetComposeInclude>>,
    pub exclude: Option<Vec<ValueSetComposeInclude>>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct ValueSetComposeInclude {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub system: Option<String>,
    pub version: Option<String>,
    pub concept: Option<Vec<ValueSetComposeIncludeConcept>>,
    pub filter: Option<Vec<ValueSetComposeIncludeFilter>>,
    #[serde(rename = "valueSet")]
    pub value_set: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ValueSetComposeIncludeConcept {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: String,
    pub display: Option<String>,
    pub designation: Option<Vec<ValueSetComposeIncludeConceptDesignation>>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct ValueSetComposeIncludeConceptDesignation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub language: Option<String>,
    #[serde(rename = "use")]
    pub r#use: Option<Coding>,
    pub value: String,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct ValueSetComposeIncludeFilter {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub property: String,
    pub op: String,
    pub value: String,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct ValueSetExpansion {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<String>,
    pub timestamp: Option<String>,
    pub total: Option<i32>,
    pub offset: Option<i32>,
    pub parameter: Option<Vec<ValueSetExpansionParameter>>,
    pub contains: Option<Vec<ValueSetExpansionContains>>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct ValueSetExpansionContains {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub system: Option<String>,
    #[serde(rename = "abstract")]
    pub r#abstract: Option<bool>,
    pub inactive: Option<bool>,
    pub version: Option<String>,
    pub code: Option<String>,
    pub display: Option<String>,
    pub designation: Option<Vec<ValueSetComposeIncludeConceptDesignation>>,
    pub contains: Option<Vec<ValueSetExpansionContains>>,
}
#[derive(Debug, Serialize, Deserialize)]
pub enum ValueSetExpansionParameterValue {
    /// Variant accepting the String type.
    #[serde(rename = "valueString")]
    String(String),
    /// Variant accepting the bool type.
    #[serde(rename = "valueBoolean")]
    Boolean(bool),
    /// Variant accepting the i32 type.
    #[serde(rename = "valueInteger")]
    Integer(i32),
    /// Variant accepting the String type.
    #[serde(rename = "valueDecimal")]
    Decimal(String),
    /// Variant accepting the String type.
    #[serde(rename = "valueUri")]
    Uri(String),
    /// Variant accepting the String type.
    #[serde(rename = "valueCode")]
    Code(String),
    /// Variant accepting the String type.
    #[serde(rename = "valueDateTime")]
    DateTime(String),
}
#[derive(Debug, Serialize, Deserialize)]
pub struct ValueSetExpansionParameter {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[serde(flatten)]
    pub value: Option<ValueSetExpansionParameterValue>,
}
