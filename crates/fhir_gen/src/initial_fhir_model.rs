use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Extension {
    pub id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Coding {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub system: Option<String>,
    pub version: Option<String>,
    pub code: Option<String>,
    pub display: Option<String>,
    #[serde(rename = "userSelected")]
    pub user_selected: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Meta {
    #[serde(rename = "versionString")]
    pub version_id: Option<String>,
    #[serde(rename = "lastUpdated")]
    pub last_updated: Option<String>,
    pub source: Option<String>,
    pub profile: Option<Vec<String>>,
    pub security: Option<Vec<Coding>>,
    pub tag: Option<Vec<Coding>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CodeableConcept {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub coding: Option<Coding>,
    pub text: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Period {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub start: Option<String>,
    pub end: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Reference {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub reference: Option<String>,
    #[serde(rename = "type")]
    pub r#type: Option<String>,
    pub identifier: Option<Box<Identifier>>, // Use of Box here for recursive type
    pub display: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Identifier {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "use")]
    pub r#use: Option<String>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub system: Option<String>,
    pub value: Option<String>,
    pub period: Option<Period>,
    pub assigner: Option<Box<Reference>>, // Use of Box here for recursive type
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Narrative {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub status: String,
    pub div: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContactDetail {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub name: Option<String>,
    pub telecom: Option<Vec<ContactPoint>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContactPoint {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub system: Option<String>,
    pub value: Option<String>,
    #[serde(rename = "use")]
    pub r#use: Option<String>,
    pub rank: Option<u32>,
    pub period: Option<Period>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Range {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub low: Option<SimpleQuantity>,
    pub high: Option<SimpleQuantity>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "resourceType")]
pub enum Resource {
    StructureDefinition(StructureDefinition),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UsageContext {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
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
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<String>,
    pub comparator: Option<String>,
    pub unit: Option<String>,
    pub system: Option<String>,
    pub code: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SimpleQuantity {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<String>,
    pub unit: Option<String>,
    pub system: Option<String>,
    pub code: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StructureDefinitionMapping {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identity: String,
    pub uri: Option<String>,
    pub name: Option<String>,
    pub comment: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StructureDefinitionContext {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: String,
    pub expression: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StructureDefinitionSnapshotOrDifferential {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub element: Option<Vec<ElementDefinition>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ElementDefinitionSlicingDescriminator {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: String,
    pub path: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ElementDefinitionSlicing {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub descriminator: Option<Vec<ElementDefinitionSlicingDescriminator>>,
    pub description: Option<String>,
    pub ordered: Option<bool>,
    pub rules: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ElementDefinitionBase {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub string: String,
    pub min: u32,
    pub max: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ElementDefinitionType {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub code: String,
    pub profile: Option<Vec<String>>,
    #[serde(rename = "targetProfile")]
    pub target_profile: Option<Vec<String>>,
    pub aggregation: Option<Vec<String>>,
    pub versioning: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Address {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "use")]
    pub r#use: Option<String>,
    #[serde(rename = "type")]
    pub r#type: Option<String>,
    pub text: Option<String>,
    pub line: Option<Vec<String>>,
    pub city: Option<String>,
    pub district: Option<String>,
    pub state: Option<String>,
    #[serde(rename = "postalString")]
    pub postal_code: Option<String>,
    pub county: Option<String>,
    pub period: Option<Period>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Duration {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<String>,
    pub comparator: Option<String>,
    pub unit: Option<String>,
    pub system: Option<String>,
    pub code: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Count {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<String>,
    pub comparator: Option<String>,
    pub unit: Option<String>,
    pub system: Option<String>,
    pub code: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Distance {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<String>,
    pub comparator: Option<String>,
    pub unit: Option<String>,
    pub system: Option<String>,
    pub code: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Age {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<String>,
    pub comparator: Option<String>,
    pub unit: Option<String>,
    pub system: Option<String>,
    pub code: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum AnnotationAuthor {
    #[serde(rename = "authorReference")]
    AuthorReference(Reference),
    #[serde(rename = "authorString")]
    AuthorString(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Annotation {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub author: Option<AnnotationAuthor>,
    pub time: Option<String>,
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Attachment {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "contentType")]
    pub content_type: Option<String>,
    pub language: Option<String>,
    pub data: Option<String>,
    pub url: Option<String>,
    pub size: Option<i64>,
    pub hash: Option<String>,
    pub title: Option<String>,
    pub creation: Option<String>,
    pub height: Option<u32>,
    pub width: Option<u32>,
    pub frames: Option<u32>,
    pub duration: Option<String>,
    pub pages: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HumanName {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "use")]
    pub r#use: Option<String>,
    pub text: Option<String>,
    pub family: Option<String>,
    pub given: Option<Vec<String>>,
    pub prefix: Option<Vec<String>>,
    pub suffix: Option<Vec<String>>,
    pub period: Option<Period>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Money {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<String>,
    pub currency: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Ratio {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub numerator: Option<Quantity>,
    pub denominator: Option<SimpleQuantity>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SampledData {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub origin: SimpleQuantity,
    pub interval: Option<String>,
    #[serde(rename = "intervalUnit")]
    pub interval_unit: String,
    pub factor: Option<String>,
    #[serde(rename = "lowerLimit")]
    pub lower_limit: Option<String>,
    #[serde(rename = "upperLimit")]
    pub upper_limit: Option<String>,
    pub dimensions: u32,
    #[serde(rename = "codeMap")]
    pub code_map: Option<String>,
    pub offsets: Option<String>,
    pub data: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Signature {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Option<Vec<Coding>>,
    pub when: Option<String>,
    pub who: Option<Reference>,
    #[serde(rename = "onBehalfOf")]
    pub on_behalf_of: Option<Reference>,
    #[serde(rename = "targetFormat")]
    pub target_format: Option<String>,
    #[serde(rename = "sigFormat")]
    pub sig_format: Option<String>,
    pub data: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum TimingRepeatBounds {
    Duration(Duration),
    Range(Range),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TimingRepeat {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub bounds: Option<TimingRepeatBounds>,
    pub count: Option<u32>,
    #[serde(rename = "countMax")]
    pub count_max: Option<u32>,
    pub duration: Option<String>,
    #[serde(rename = "durationMax")]
    pub duration_max: Option<u32>,
    #[serde(rename = "durationUnit")]
    pub duration_unit: Option<String>,
    pub frequency: Option<u32>,
    #[serde(rename = "frequencyMax")]
    pub frequency_max: Option<u32>,
    pub period: Option<String>,
    #[serde(rename = "periodMax")]
    pub period_max: Option<String>,
    #[serde(rename = "periodUnit")]
    pub period_unit: Option<String>,
    #[serde(rename = "dayOfWeek")]
    pub day_of_week: Option<Vec<String>>,
    #[serde(rename = "timeOfDay")]
    pub time_of_day: Option<Vec<String>>,
    pub when: Option<Vec<String>>,
    pub offset: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Timing {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub event: Option<Vec<String>>,
    pub repeat: Option<TimingRepeat>,
    pub code: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DataRequirement {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    // TODO - more
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Dosage {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    // TODO - more
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Expression {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    pub name: Option<String>,
    pub language: Option<String>,
    pub expression: Option<String>,
    pub reference: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ParameterDefinition {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub name: Option<String>,
    #[serde(rename = "use")]
    pub r#use: String,
    pub min: Option<i32>,
    pub max: Option<String>,
    pub documentation: Option<String>,
    #[serde(rename = "type")]
    pub r#type: String,
    pub profile: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RelatedArtifact {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: String,
    pub classifier: Option<Vec<CodeableConcept>>,
    pub label: Option<String>,
    pub display: Option<String>,
    pub citation: Option<String>,
    pub document: Option<Attachment>,
    pub resource: Option<String>,
    #[serde(rename = "resourceReference")]
    pub resource_reference: Option<Reference>,
    #[serde(rename = "publicationStatus")]
    pub publication_status: Option<String>,
    #[serde(rename = "publicationDate")]
    pub publication_date: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum TriggerDefinitionTiming {
    Timing(Timing),
    Reference(Reference),
    Date(String),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TriggerDefinition {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: String,
    pub name: Option<String>,
    pub code: Option<CodeableConcept>,
    #[serde(rename = "subscriptionTopic")]
    pub subscription_topic: Option<String>,
    pub timing: Option<TriggerDefinitionTiming>,
    pub data: Option<Vec<DataRequirement>>,
    pub condition: Option<Expression>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ElementDefinitionDefaultValue {
    Base64Binary(String),
    Boolean(bool),
    Canonical(String),
    Code(String),
    Date(String),
    DateTime(String),
    Decimal(String),
    Id(String),
    Instant(String),
    Integer(i32),
    Markdown(String),
    Oid(String),
    PositiveInt(u32),
    String(String),
    Time(String),
    UnsignedInt(u32),
    Uri(String),
    Url(String),
    Uuid(String),
    Address(Address),
    Age(Age),
    Annotation(Annotation),
    Attachment(Attachment),
    CodeableConcept(CodeableConcept),
    Coding(Coding),
    ContactPoint(ContactPoint),
    Count(Count),
    Distance(Distance),
    Duration(Duration),
    HumanName(HumanName),
    Identifier(Identifier),
    Money(Money),
    Period(Period),
    Quantity(Quantity),
    Range(Range),
    Ratio(Ratio),
    Reference(Reference),
    SampledData(SampledData),
    Signature(Signature),
    Timing(Timing),
    ContactDetail(ContactDetail),
    DataRequirement(DataRequirement),
    Expression(Expression),
    ParameterDefinition(ParameterDefinition),
    RelatedArtifact(RelatedArtifact),
    TriggerDefinition(TriggerDefinition),
    UsageContext(UsageContext),
    Dosage(Dosage),
    Meta(Meta),
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ElementDefinitionMinMaxValue {
    Date(String),
    DateTime(String),
    Instant(String),
    Time(String),
    Decimal(String),
    Integer(i32),
    Integer64(i64),
    PositiveInt(u32),
    UnsignedInt(u32),
    Quantity(Quantity),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ElementDefinition {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub path: String,
    pub representation: Option<Vec<String>>,
    #[serde(rename = "sliceName")]
    pub slice_name: Option<String>,
    #[serde(rename = "sliceIsConstraining")]
    pub slice_is_constraining: Option<bool>,
    pub label: Option<String>,
    pub code: Option<Vec<Coding>>,
    pub slicing: Option<ElementDefinitionSlicing>,
    pub short: Option<String>,
    pub definition: Option<String>,
    pub comment: Option<String>,
    pub requirements: Option<String>,
    pub alias: Option<Vec<String>>,
    pub min: Option<u32>,
    pub max: Option<String>,
    pub base: Option<ElementDefinitionBase>,
    #[serde(rename = "contentReference")]
    pub content_reference: Option<String>,
    #[serde(rename = "type")]
    pub r#type: Option<Vec<ElementDefinitionType>>,
    #[serde(rename = "defaultValue")]
    pub default_value: Option<ElementDefinitionDefaultValue>,
    #[serde(rename = "meaningWhenMissing")]
    pub meaning_when_missing: Option<String>,
    #[serde(rename = "orderMeaning")]
    pub order_meaning: Option<String>,
    pub fixed: Option<ElementDefinitionDefaultValue>,
    pub pattern: Option<ElementDefinitionDefaultValue>,
    pub example: Option<Vec<ElementDefinitionExample>>,
    #[serde(rename = "minValue")]
    pub min_value: Option<ElementDefinitionMinMaxValue>,
    #[serde(rename = "maxValue")]
    pub max_value: Option<ElementDefinitionMinMaxValue>,
    #[serde(rename = "maxLength")]
    pub max_length: Option<i32>,
    pub condition: Option<Vec<String>>,
    pub constraint: Option<Vec<ElementDefinitionConstraint>>,
    #[serde(rename = "mustHaveValue")]
    pub must_have_value: Option<bool>,
    #[serde(rename = "valueAlternatives")]
    pub value_alternatives: Option<Vec<String>>,
    #[serde(rename = "mustSupport")]
    pub must_support: Option<bool>,
    #[serde(rename = "isModifier")]
    pub is_modifier: Option<bool>,
    #[serde(rename = "isModifierReason")]
    pub is_modifier_reason: Option<String>,
    #[serde(rename = "isSummary")]
    pub is_summary: Option<bool>,
    pub binding: Option<ElementDefinitionBinding>,
    pub mapping: Option<Vec<ElementDefinitionMapping>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ElementDefinitionMapping {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub identity: String,
    pub language: Option<String>,
    pub map: String,
    pub comment: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ElementDefinitionBinding {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub strength: String,
    pub description: Option<String>,
    #[serde(rename = "valueSet")]
    pub value_set: Option<String>,
    pub additional: Option<Vec<ElementDefinitionBindingAdditional>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ElementDefinitionBindingAdditional {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub purpose: String,
    #[serde(rename = "valueSet")]
    pub value_set: String,
    pub documentation: Option<String>,
    #[serde(rename = "shortDoco")]
    pub short_doco: Option<String>,
    pub usage: Option<Vec<UsageContext>>,
    pub any: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ElementDefinitionExample {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub label: String,
    pub value: ElementDefinitionDefaultValue,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ElementDefinitionConstraint {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    pub key: String,
    pub requirements: Option<String>,
    pub severity: String,
    pub suppress: Option<bool>,
    pub human: String,
    pub expression: Option<String>,
    pub source: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StructureDefinition {
    #[serde(rename = "resourceType")]
    pub resource_type: String,
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<String>,
    pub language: Option<String>,
    pub text: Option<Narrative>,
    pub contained: Option<Box<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: String,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    #[serde(rename = "versionAlgorithmString")]
    pub version_algorithm_string: Option<String>,
    #[serde(rename = "versionAlgorithmCoding")]
    pub version_algorithm_coding: Option<Coding>,
    pub name: String,
    pub title: Option<String>,
    pub status: String,
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
    #[serde(rename = "copyrightLabel")]
    pub copyright_label: Option<String>,
    pub keyword: Option<Vec<Coding>>,
    #[serde(rename = "fhirVersion")]
    pub fhir_version: Option<String>,
    pub mapping: Option<Vec<StructureDefinitionMapping>>,
    pub kind: String,
    #[serde(rename = "abstract")]
    pub r#abstract: bool,
    pub context: Option<Vec<StructureDefinitionContext>>,
    #[serde(rename = "contextInvariant")]
    pub context_invariant: Option<Vec<String>>,
    #[serde(rename = "type")]
    pub r#type: String,
    #[serde(rename = "baseDefinition")]
    pub base_definition: Option<String>,
    pub derivation: Option<String>,
    pub snapshot: Option<StructureDefinitionSnapshotOrDifferential>,
    pub differential: Option<StructureDefinitionSnapshotOrDifferential>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BundleLink {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub relation: String,
    pub url: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BundleEntry {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub link: Option<Vec<BundleLink>>,
    #[serde(rename = "fullUrl")]
    pub full_url: Option<String>,
    pub resource: Option<Resource>,
    pub search: Option<BundleEntrySearch>,
    pub request: Option<BundleEntryRequest>,
    pub response: Option<BundleEntryResponse>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BundleEntrySearch {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BundleEntryRequest {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub method: String,
    pub url: String,
    #[serde(rename = "ifNoneMatch")]
    pub if_none_match: Option<String>,
    #[serde(rename = "ifModifiedSince")]
    pub if_modified_since: Option<String>,
    #[serde(rename = "ifMatch")]
    pub if_match: Option<String>,
    #[serde(rename = "ifNoneExist")]
    pub if_none_exist: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BundleEntryResponse {
    pub id: Option<String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub status: String,
    pub location: Option<String>,
    pub etag: Option<String>,
    #[serde(rename = "lastModified")]
    pub last_modified: Option<String>,
    pub outcome: Option<Resource>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Bundle {
    #[serde(rename = "resourceType")]
    pub resource_type: String,
    pub id: Option<String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<String>,
    pub language: Option<String>,
    pub identifier: Option<Identifier>,
    #[serde(rename = "type")]
    pub r#type: String,
    pub timestamp: Option<String>,
    pub total: Option<u32>,
    pub link: Option<Vec<BundleLink>>,
    pub entry: Option<Vec<BundleEntry>>,
    pub signature: Option<Signature>,
}
