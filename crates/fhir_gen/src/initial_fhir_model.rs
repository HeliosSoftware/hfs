use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Extension {
    // TODO
}

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
pub struct Uuid {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub value: Option<std::string::String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Url {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub value: Option<std::string::String>,
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
pub struct UnsignedInt {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub value: Option<u32>,
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
pub enum Resource {
    StructureDefinition(StructureDefinition),
}

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
pub struct StructureDefinitionMapping {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Vec<Extension>,
    pub identity: Id,
    pub uri: Option<Uri>,
    pub name: Option<String>,
    pub comment: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StructureDefinitionContext {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Vec<Extension>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub expression: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StructureDefinitionSnapshotOrDifferential {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Vec<Extension>,
    pub element: Vec<ElementDefinition>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ElementDefinitionSlicingDescriminator {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub path: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ElementDefinitionSlicing {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub descriminator: Vec<ElementDefinitionSlicingDescriminator>,
    pub description: Option<String>,
    pub ordered: Option<Boolean>,
    pub rules: Code,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ElementDefinitionBase {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub string: String,
    pub min: UnsignedInt,
    pub max: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ElementDefinitionType {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub code: Uri,
    pub profile: Option<Canonical>,
    #[serde(rename = "targetProfile")]
    pub target_profile: Option<Canonical>,
    pub aggregation: Option<Code>,
    pub versioning: Option<Code>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Oid {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub value: Option<std::string::String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Base64Binary {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub value: Option<std::string::String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Time {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub value: Option<std::string::String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Date {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub value: Option<std::string::String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Integer {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub value: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Integer64 {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub value: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Address {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    #[serde(rename = "use")]
    pub r#use: Option<Code>,
    #[serde(rename = "type")]
    pub r#type: Option<Code>,
    pub text: Option<String>,
    pub line: Vec<String>,
    pub city: Option<String>,
    pub district: Option<String>,
    pub state: Option<String>,
    #[serde(rename = "postalCode")]
    pub postal_code: Option<String>,
    pub county: Option<String>,
    pub period: Option<Period>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Duration {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub value: Option<Decimal>,
    pub comparator: Option<Code>,
    pub unit: Option<String>,
    pub system: Option<Uri>,
    pub code: Option<Code>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Count {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub value: Option<Decimal>,
    pub comparator: Option<Code>,
    pub unit: Option<String>,
    pub system: Option<Uri>,
    pub code: Option<Code>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Distance {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub value: Option<Decimal>,
    pub comparator: Option<Code>,
    pub unit: Option<String>,
    pub system: Option<Uri>,
    pub code: Option<Code>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Age {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub value: Option<Decimal>,
    pub comparator: Option<Code>,
    pub unit: Option<String>,
    pub system: Option<Uri>,
    pub code: Option<Code>,
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
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub author: Option<AnnotationAuthor>,
    pub time: Option<DateTime>,
    pub text: Markdown,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Attachment {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    #[serde(rename = "contentType")]
    pub content_type: Option<Code>,
    pub language: Option<Code>,
    pub data: Option<Base64Binary>,
    pub url: Option<Url>,
    pub size: Option<Integer64>,
    pub hash: Option<Base64Binary>,
    pub title: Option<String>,
    pub creation: Option<DateTime>,
    pub height: Option<PositiveInt>,
    pub width: Option<PositiveInt>,
    pub frames: Option<PositiveInt>,
    pub duration: Option<Decimal>,
    pub pages: Option<PositiveInt>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HumanName {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    #[serde(rename = "use")]
    pub r#use: Option<Code>,
    pub text: Option<String>,
    pub family: Option<String>,
    pub given: Vec<String>,
    pub prefix: Vec<String>,
    pub suffix: Vec<String>,
    pub period: Option<Period>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Money {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub value: Option<Decimal>,
    pub currency: Option<Code>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Ratio {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub numerator: Option<Quantity>,
    pub denominator: Option<SimpleQuantity>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SampledData {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub origin: SimpleQuantity,
    pub interval: Option<Decimal>,
    #[serde(rename = "intervalUnit")]
    pub interval_unit: Code,
    pub factor: Option<Decimal>,
    #[serde(rename = "lowerLimit")]
    pub lower_limit: Option<Decimal>,
    #[serde(rename = "upperLimit")]
    pub upper_limit: Option<Decimal>,
    pub dimensions: PositiveInt,
    #[serde(rename = "codeMap")]
    pub code_map: Option<Canonical>,
    pub offsets: Option<String>,
    pub data: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Signature {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    #[serde(rename = "type")]
    pub r#type: Vec<Coding>,
    pub when: Option<Instant>,
    pub who: Option<Reference>,
    #[serde(rename = "onBehalfOf")]
    pub on_behalf_of: Option<Reference>,
    #[serde(rename = "targetFormat")]
    pub target_format: Option<Code>,
    #[serde(rename = "sigFormat")]
    pub sig_format: Option<Code>,
    pub data: Option<Base64Binary>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum TimingRepeatBounds {
    Duration(Duration),
    Range(Range),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TimingRepeat {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub bounds: Option<TimingRepeatBounds>,
    pub count: Option<PositiveInt>,
    #[serde(rename = "countMax")]
    pub count_max: Option<PositiveInt>,
    pub duration: Option<Decimal>,
    #[serde(rename = "durationMax")]
    pub duration_max: Option<PositiveInt>,
    #[serde(rename = "durationUnit")]
    pub duration_unit: Option<Code>,
    pub frequency: Option<PositiveInt>,
    #[serde(rename = "frequencyMax")]
    pub frequency_max: Option<PositiveInt>,
    pub period: Option<Decimal>,
    #[serde(rename = "periodMax")]
    pub period_max: Option<Decimal>,
    #[serde(rename = "periodUnit")]
    pub period_unit: Option<Code>,
    #[serde(rename = "dayOfWeek")]
    pub day_of_week: Vec<Code>,
    #[serde(rename = "timeOfDay")]
    pub time_of_day: Vec<Time>,
    pub when: Vec<Code>,
    pub offset: Option<UnsignedInt>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Timing {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub event: Vec<DateTime>,
    pub repeat: Option<TimingRepeat>,
    pub code: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DataRequirement {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    // TODO - more
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Dosage {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    // TODO - more
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Expression {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub description: Option<String>,
    pub name: Option<Code>,
    pub language: Option<Code>,
    pub expression: Option<Code>,
    pub reference: Option<Uri>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ParameterDefinition {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub name: Option<Code>,
    #[serde(rename = "use")]
    pub r#use: Code,
    pub min: Option<Integer>,
    pub max: Option<String>,
    pub documentation: Option<String>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub profile: Option<Canonical>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RelatedArtifact {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub classifier: Vec<CodeableConcept>,
    pub label: Option<String>,
    pub display: Option<String>,
    pub citation: Option<Markdown>,
    pub document: Option<Attachment>,
    pub resource: Option<Canonical>,
    #[serde(rename = "resourceReference")]
    pub resource_reference: Option<Reference>,
    #[serde(rename = "publicationStatus")]
    pub publication_status: Option<Code>,
    #[serde(rename = "publicationDate")]
    pub publication_date: Option<Date>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum TriggerDefinitionTiming {
    Timing(Timing),
    Reference(Reference),
    Date(Date),
    DateTime(DateTime),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TriggerDefinition {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub name: Option<String>,
    pub code: Option<CodeableConcept>,
    #[serde(rename = "subscriptionTopic")]
    pub subscription_topic: Option<Canonical>,
    pub timing: Option<TriggerDefinitionTiming>,
    pub data: Vec<DataRequirement>,
    pub condition: Option<Expression>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ElementDefinitionDefaultValue {
    Base64Binary(Base64Binary),
    Boolean(Boolean),
    Canonical(Canonical),
    Code(Code),
    Date(Date),
    DateTime(DateTime),
    Decimal(Decimal),
    Id(Id),
    Instant(Instant),
    Integer(Integer),
    Markdown(Markdown),
    Oid(Oid),
    PositiveInt(PositiveInt),
    String(String),
    Time(Time),
    UnsignedInt(UnsignedInt),
    Uri(Uri),
    Url(Url),
    Uuid(Uuid),
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
    Date(Date),
    DateTime(DateTime),
    Instant(Instant),
    Time(Time),
    Decimal(Decimal),
    Integer(Integer),
    Integer64(Integer64),
    PositiveInt(PositiveInt),
    UnsignedInt(UnsignedInt),
    Quantity(Quantity),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ElementDefinition {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub path: String,
    pub representation: Vec<Code>,
    #[serde(rename = "sliceName")]
    pub slice_name: Option<String>,
    #[serde(rename = "sliceIsConstraining")]
    pub slice_is_constraining: Option<Boolean>,
    pub label: Option<String>,
    pub code: Vec<Coding>,
    pub slicing: Option<ElementDefinitionSlicing>,
    pub short: Option<String>,
    pub definition: Option<Markdown>,
    pub comment: Option<Markdown>,
    pub requirements: Option<Markdown>,
    pub alias: Vec<String>,
    pub min: Option<UnsignedInt>,
    pub max: Option<String>,
    pub base: Option<ElementDefinitionBase>,
    #[serde(rename = "contentReference")]
    pub content_reference: Option<Uri>,
    #[serde(rename = "type")]
    pub r#type: Option<ElementDefinitionType>,
    #[serde(rename = "defaultValue")]
    pub default_value: Option<ElementDefinitionDefaultValue>,
    #[serde(rename = "meaningWhenMissing")]
    pub meaning_when_missing: Option<Markdown>,
    #[serde(rename = "orderMeaning")]
    pub order_meaning: Option<String>,
    pub fixed: Option<ElementDefinitionDefaultValue>,
    pub pattern: Option<ElementDefinitionDefaultValue>,
    pub example: Vec<ElementDefinitionExample>,
    #[serde(rename = "minValue")]
    pub min_value: Option<ElementDefinitionMinMaxValue>,
    #[serde(rename = "maxValue")]
    pub max_value: Option<ElementDefinitionMinMaxValue>,
    #[serde(rename = "maxLength")]
    pub max_length: Option<Integer>,
    pub condition: Vec<Id>,
    pub constraint: Vec<ElementDefinitionConstraint>,
    #[serde(rename = "mustHaveValue")]
    pub must_have_value: Option<Boolean>,
    #[serde(rename = "valueAlternatives")]
    pub value_alternatives: Vec<Canonical>,
    #[serde(rename = "mustSupport")]
    pub must_support: Option<Boolean>,
    #[serde(rename = "isModifier")]
    pub is_modifier: Option<Boolean>,
    #[serde(rename = "isModifierReason")]
    pub is_modifier_reason: Option<String>,
    #[serde(rename = "isSummary")]
    pub is_summary: Option<Boolean>,
    pub binding: Option<ElementDefinitionBinding>,
    pub mapping: Vec<ElementDefinitionMapping>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ElementDefinitionMapping {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub identity: Id,
    pub language: Option<Code>,
    pub map: String,
    pub comment: Option<Markdown>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ElementDefinitionBinding {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub strength: Code,
    pub description: Option<Markdown>,
    #[serde(rename = "valueSet")]
    pub value_set: Option<Canonical>,
    pub additional: Vec<ElementDefinitionBindingAdditional>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ElementDefinitionBindingAdditional {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub purpose: Code,
    #[serde(rename = "valueSet")]
    pub value_set: Canonical,
    pub documentation: Option<Markdown>,
    #[serde(rename = "shortDoco")]
    pub short_doco: Option<String>,
    pub usage: Vec<UsageContext>,
    pub any: Option<Boolean>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ElementDefinitionExample {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub label: String,
    pub value: ElementDefinitionDefaultValue,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ElementDefinitionConstraint {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    pub key: Id,
    pub requirements: Option<Markdown>,
    pub severity: Code,
    pub suppress: Option<Boolean>,
    pub human: String,
    pub expression: Option<String>,
    pub source: Option<Canonical>,
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
    pub modifier_extension: Vec<Extension>,
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
    pub mapping: Vec<StructureDefinitionMapping>,
    pub kind: Code,
    #[serde(rename = "abstract")]
    pub r#abstract: Boolean,
    pub context: Vec<StructureDefinitionContext>,
    #[serde(rename = "contextInvariant")]
    pub context_invariant: Vec<String>,
    #[serde(rename = "type")]
    pub r#type: Uri,
    #[serde(rename = "baseDefinition")]
    pub base_definition: Option<Canonical>,
    pub derivation: Option<Code>,
    pub snapshot: Vec<StructureDefinitionSnapshotOrDifferential>,
    pub differential: Vec<StructureDefinitionSnapshotOrDifferential>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BundleLink {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Vec<Extension>,
    pub relation: String,
    pub url: Uri,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BundleEntry {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Vec<Extension>,
    pub link: Vec<BundleLink>,
    #[serde(rename = "fullUrl")]
    pub full_url: Option<Uri>,
    pub resource: Option<Resource>,
    pub search: Option<BundleEntrySearch>,
    pub request: Option<BundleEntryRequest>,
    pub response: Option<BundleEntryResponse>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BundleEntrySearch {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Vec<Extension>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BundleEntryRequest {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Vec<Extension>,
    pub method: Code,
    pub url: Uri,
    #[serde(rename = "ifNoneMatch")]
    pub if_none_match: Option<String>,
    #[serde(rename = "ifModifiedSince")]
    pub if_modified_since: Option<Instant>,
    #[serde(rename = "ifMatch")]
    pub if_match: Option<String>,
    #[serde(rename = "ifNoneExist")]
    pub if_none_exist: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BundleEntryResponse {
    pub id: Option<std::string::String>,
    pub extension: Vec<Extension>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Vec<Extension>,
    pub status: String,
    pub location: Option<Uri>,
    pub etag: Option<String>,
    #[serde(rename = "lastModified")]
    pub last_modified: Option<Instant>,
    pub outcome: Option<Resource>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Bundle {
    pub id: Option<Id>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub identifier: Option<Identifier>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub timestamp: Option<Instant>,
    pub total: Option<UnsignedInt>,
    pub link: Vec<BundleLink>,
    pub entry: Vec<BundleEntry>,
    pub signature: Option<Signature>,
}
