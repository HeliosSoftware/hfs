use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Extension {}

#[derive(Debug, Serialize, Deserialize)]
pub struct Id {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub value: Option<std::string::String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Decimal {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub value: Option<std::string::String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Canonical {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub value: Option<std::string::String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Instant {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub value: Option<std::string::String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct String {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub value: Option<std::string::String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Boolean {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub value: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Coding {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub system: Option<Uri>,
    pub version: Option<String>,
    pub code: Option<Code>,
    pub display: Option<String>,
    #[serde(rename = "userSelected")]
    pub user_selected: Boolean,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Code {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub value: Option<std::string::String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Meta {
    #[serde(rename = "versionId")]
    pub version_id: Option<Id>,
    #[serde(rename = "lastUpdated")]
    pub last_updated: Option<Instant>,
    pub source: Option<Uri>,
    pub profile: Vec<Canonical>,
    pub security: Vec<Coding>,
    pub tag: Vec<Coding>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Uri {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub value: Option<std::string::String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Xhtml {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub value: Option<std::string::String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Markdown {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub value: Option<std::string::String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CodeableConcept {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub coding: Option<Coding>,
    pub text: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Period {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub start: Option<DateTime>,
    pub end: Option<DateTime>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DateTime {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub value: Option<std::string::String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Reference {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub reference: Option<String>,
    #[serde(rename = "type")]
    pub r#type: Option<Uri>,
    pub identifier: Option<Box<Identifier>>, // Use of Box here for recursive type
    pub display: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Identifier {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    #[serde(rename = "use")]
    pub r#use: Option<Code>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub system: Option<Uri>,
    pub value: Option<String>,
    pub period: Option<Period>,
    pub assigner: Option<Box<Reference>>, // Use of Box here for recursive type
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Narrative {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub status: Code,
    pub div: Xhtml,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContactDetail {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub name: Option<String>,
    pub telecom: Vec<ContactPoint>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContactPoint {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub system: Option<Code>,
    pub value: Option<String>,
    #[serde(rename = "use")]
    pub r#use: Option<Code>,
    pub rank: Option<PositiveInt>,
    pub period: Option<Period>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PositiveInt {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub value: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Range {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub low: Option<SimpleQuantity>,
    pub high: Option<SimpleQuantity>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Resource {}

#[derive(Debug, Serialize, Deserialize)]
pub struct UsageContext {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub coding: Coding,
    pub value: UsageContextValue,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum UsageContextValue {
    CodeableConcept(CodeableConcept),
    Quantity(Quantity),
    Range(Range),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Quantity {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub value: Option<Decimal>,
    pub comparator: Option<Code>,
    pub unit: Option<String>,
    pub system: Option<Uri>,
    pub code: Option<Code>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SimpleQuantity {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub value: Option<Decimal>,
    pub unit: Option<String>,
    pub system: Option<Uri>,
    pub code: Option<Code>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StructureDefinition {
    pub id: Option<Id>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Resource>,
    pub extension: Option<Extension>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Extension>,
    pub url: Uri,
    pub identifier: Vec<Identifier>,
    pub version: Option<String>,
    #[serde(rename = "versionAlgorithmString")]
    pub version_algorithm_string: Option<String>,
    #[serde(rename = "versionAlgorithmCoding")]
    pub version_algorithm_coding: Option<Coding>,
    pub name: String,
    pub title: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Vec<ContactDetail>,
    pub description: Option<Markdown>,
    #[serde(rename = "useContext")]
    pub use_context: Vec<UsageContext>,
    pub jurisdiction: Vec<CodeableConcept>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    #[serde(rename = "copyrightedLabel")]
    pub copyrighted_label: Option<String>,
    pub keyword: Vec<Coding>,
    #[serde(rename = "fhirVersion")]
    pub fhir_version: Option<Code>,
}
