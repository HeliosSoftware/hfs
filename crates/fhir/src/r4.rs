use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Base64Binary {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<std::string::String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Boolean {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Canonical {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<std::string::String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Code {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<std::string::String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Date {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<std::string::String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DateTime {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<std::string::String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Decimal {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<std::primitive::f64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Id {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<std::string::String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Instant {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<std::string::String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Integer {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<std::primitive::i32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Markdown {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<std::string::String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Oid {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<std::string::String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PositiveInt {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<std::string::String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct String {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<std::string::String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Time {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<std::string::String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UnsignedInt {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<std::string::String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Uri {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<std::string::String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Url {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<std::string::String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Uuid {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<std::string::String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Xhtml {
    pub id: Option<std::string::String>,
    pub extension: Option<Extension>,
    pub value: std::string::String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Address {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "use")]
    pub r#use: Option<Code>,
    #[serde(rename = "type")]
    pub r#type: Option<Code>,
    pub text: Option<String>,
    pub line: Option<Vec<String>>,
    pub city: Option<String>,
    pub district: Option<String>,
    pub state: Option<String>,
    pub postalCode: Option<String>,
    pub country: Option<String>,
    pub period: Option<Period>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Age {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<Decimal>,
    pub comparator: Option<Code>,
    pub unit: Option<String>,
    pub system: Option<Uri>,
    pub code: Option<Code>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AnnotationAuthor {
    Reference(Reference),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Annotation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub time: Option<DateTime>,
    pub text: Markdown,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Attachment {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub contentType: Option<Code>,
    pub language: Option<Code>,
    pub data: Option<Base64Binary>,
    pub url: Option<Url>,
    pub size: Option<UnsignedInt>,
    pub hash: Option<Base64Binary>,
    pub title: Option<String>,
    pub creation: Option<DateTime>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CodeableConcept {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub coding: Option<Vec<Coding>>,
    pub text: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Coding {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub system: Option<Uri>,
    pub version: Option<String>,
    pub code: Option<Code>,
    pub display: Option<String>,
    pub userSelected: Option<Boolean>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContactDetail {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub name: Option<String>,
    pub telecom: Option<Vec<ContactPoint>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContactPoint {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub system: Option<Code>,
    pub value: Option<String>,
    #[serde(rename = "use")]
    pub r#use: Option<Code>,
    pub rank: Option<PositiveInt>,
    pub period: Option<Period>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Contributor {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub name: String,
    pub contact: Option<Vec<ContactDetail>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Count {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<Decimal>,
    pub comparator: Option<Code>,
    pub unit: Option<String>,
    pub system: Option<Uri>,
    pub code: Option<Code>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DataRequirementSubject {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DataRequirement {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub profile: Option<Vec<Canonical>>,
    pub mustSupport: Option<Vec<String>>,
    pub codeFilter: Option<Vec<DataRequirementCodeFilter>>,
    pub dateFilter: Option<Vec<DataRequirementDateFilter>>,
    pub limit: Option<PositiveInt>,
    pub sort: Option<Vec<DataRequirementSort>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DataRequirementCodeFilter {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub path: Option<String>,
    pub searchParam: Option<String>,
    pub valueSet: Option<Canonical>,
    pub code: Option<Vec<Coding>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DataRequirementSort {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub path: String,
    pub direction: Code,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DataRequirementDateFilterValue {
    DateTime(DateTime),
    Period(Period),
    Duration(Duration),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DataRequirementDateFilter {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub path: Option<String>,
    pub searchParam: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Distance {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<Decimal>,
    pub comparator: Option<Code>,
    pub unit: Option<String>,
    pub system: Option<Uri>,
    pub code: Option<Code>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DosageDoseAndRateDose {
    Range(Range),
    Quantity(Quantity),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DosageDoseAndRate {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DosageAsNeeded {
    Boolean(Boolean),
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Dosage {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifierExtension: Option<Vec<Extension>>,
    pub sequence: Option<Integer>,
    pub text: Option<String>,
    pub additionalInstruction: Option<Vec<CodeableConcept>>,
    pub patientInstruction: Option<String>,
    pub timing: Option<Timing>,
    pub site: Option<CodeableConcept>,
    pub route: Option<CodeableConcept>,
    pub method: Option<CodeableConcept>,
    pub doseAndRate: Option<Vec<DosageDoseAndRate>>,
    pub maxDosePerPeriod: Option<Ratio>,
    pub maxDosePerAdministration: Option<Quantity>,
    pub maxDosePerLifetime: Option<Quantity>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Duration {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<Decimal>,
    pub comparator: Option<Code>,
    pub unit: Option<String>,
    pub system: Option<Uri>,
    pub code: Option<Code>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ElementDefinitionType {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub code: Uri,
    pub profile: Option<Vec<Canonical>>,
    pub targetProfile: Option<Vec<Canonical>>,
    pub aggregation: Option<Vec<Code>>,
    pub versioning: Option<Code>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ElementDefinitionConstraint {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub key: Id,
    pub requirements: Option<String>,
    pub severity: Code,
    pub human: String,
    pub expression: Option<String>,
    pub xpath: Option<String>,
    pub source: Option<Canonical>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ElementDefinitionSlicingDiscriminator {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub path: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ElementDefinitionExampleValue {
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
    Contributor(Contributor),
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
pub struct ElementDefinitionExample {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub label: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ElementDefinitionMapping {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub identity: Id,
    pub language: Option<Code>,
    pub map: String,
    pub comment: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
    Contributor(Contributor),
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
pub struct ElementDefinition {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifierExtension: Option<Vec<Extension>>,
    pub path: String,
    pub representation: Option<Vec<Code>>,
    pub sliceName: Option<String>,
    pub sliceIsConstraining: Option<Boolean>,
    pub label: Option<String>,
    pub code: Option<Vec<Coding>>,
    pub slicing: Option<ElementDefinitionSlicing>,
    pub short: Option<String>,
    pub definition: Option<Markdown>,
    pub comment: Option<Markdown>,
    pub requirements: Option<Markdown>,
    pub alias: Option<Vec<String>>,
    pub min: Option<UnsignedInt>,
    pub max: Option<String>,
    pub base: Option<ElementDefinitionBase>,
    pub contentReference: Option<Uri>,
    #[serde(rename = "type")]
    pub r#type: Option<Vec<ElementDefinitionType>>,
    pub meaningWhenMissing: Option<Markdown>,
    pub orderMeaning: Option<String>,
    pub example: Option<Vec<ElementDefinitionExample>>,
    pub maxLength: Option<Integer>,
    pub condition: Option<Vec<Id>>,
    pub constraint: Option<Vec<ElementDefinitionConstraint>>,
    pub mustSupport: Option<Boolean>,
    pub isModifier: Option<Boolean>,
    pub isModifierReason: Option<String>,
    pub isSummary: Option<Boolean>,
    pub binding: Option<ElementDefinitionBinding>,
    pub mapping: Option<Vec<ElementDefinitionMapping>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ElementDefinitionBase {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub path: String,
    pub min: UnsignedInt,
    pub max: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ElementDefinitionSlicing {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub discriminator: Option<Vec<ElementDefinitionSlicingDiscriminator>>,
    pub description: Option<String>,
    pub ordered: Option<Boolean>,
    pub rules: Code,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ElementDefinitionBinding {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub strength: Code,
    pub description: Option<String>,
    pub valueSet: Option<Canonical>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Expression {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    pub name: Option<Id>,
    pub language: Code,
    pub expression: Option<String>,
    pub reference: Option<Uri>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExtensionValue {
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
    Contributor(Contributor),
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
pub struct Extension {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub url: std::string::String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HumanName {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "use")]
    pub r#use: Option<Code>,
    pub text: Option<String>,
    pub family: Option<String>,
    pub given: Option<Vec<String>>,
    pub prefix: Option<Vec<String>>,
    pub suffix: Option<Vec<String>>,
    pub period: Option<Period>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Identifier {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "use")]
    pub r#use: Option<Code>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub system: Option<Uri>,
    pub value: Option<String>,
    pub period: Option<Period>,
    pub assigner: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MarketingStatus {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifierExtension: Option<Vec<Extension>>,
    pub country: CodeableConcept,
    pub jurisdiction: Option<CodeableConcept>,
    pub status: CodeableConcept,
    pub dateRange: Period,
    pub restoreDate: Option<DateTime>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Meta {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub versionId: Option<Id>,
    pub lastUpdated: Option<Instant>,
    pub source: Option<Uri>,
    pub profile: Option<Vec<Canonical>>,
    pub security: Option<Vec<Coding>>,
    pub tag: Option<Vec<Coding>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Money {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<Decimal>,
    pub currency: Option<Code>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Narrative {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub status: Code,
    pub div: Xhtml,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ParameterDefinition {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
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
pub struct Period {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub start: Option<DateTime>,
    pub end: Option<DateTime>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PopulationAge {
    Range(Range),
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Population {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifierExtension: Option<Vec<Extension>>,
    pub gender: Option<CodeableConcept>,
    pub race: Option<CodeableConcept>,
    pub physiologicalCondition: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProdCharacteristic {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifierExtension: Option<Vec<Extension>>,
    pub height: Option<Quantity>,
    pub width: Option<Quantity>,
    pub depth: Option<Quantity>,
    pub weight: Option<Quantity>,
    pub nominalVolume: Option<Quantity>,
    pub externalDiameter: Option<Quantity>,
    pub shape: Option<String>,
    pub color: Option<Vec<String>>,
    pub imprint: Option<Vec<String>>,
    pub image: Option<Vec<Attachment>>,
    pub scoring: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProductShelfLife {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifierExtension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub period: Quantity,
    pub specialPrecautionsForStorage: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Quantity {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<Decimal>,
    pub comparator: Option<Code>,
    pub unit: Option<String>,
    pub system: Option<Uri>,
    pub code: Option<Code>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Range {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub low: Option<Quantity>,
    pub high: Option<Quantity>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Ratio {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub numerator: Option<Quantity>,
    pub denominator: Option<Quantity>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Reference {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub reference: Option<String>,
    #[serde(rename = "type")]
    pub r#type: Option<Uri>,
    pub identifier: Option<Identifier>,
    pub display: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RelatedArtifact {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub label: Option<String>,
    pub display: Option<String>,
    pub citation: Option<Markdown>,
    pub url: Option<Url>,
    pub document: Option<Attachment>,
    pub resource: Option<Canonical>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SampledData {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub origin: Quantity,
    pub period: Decimal,
    pub factor: Option<Decimal>,
    pub lowerLimit: Option<Decimal>,
    pub upperLimit: Option<Decimal>,
    pub dimensions: PositiveInt,
    pub data: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Signature {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Option<Vec<Coding>>,
    pub when: Instant,
    pub who: Reference,
    pub onBehalfOf: Option<Reference>,
    pub targetFormat: Option<Code>,
    pub sigFormat: Option<Code>,
    pub data: Option<Base64Binary>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubstanceAmountAmount {
    Quantity(Quantity),
    Range(Range),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SubstanceAmount {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifierExtension: Option<Vec<Extension>>,
    pub amountType: Option<CodeableConcept>,
    pub amountText: Option<String>,
    pub referenceRange: Option<SubstanceAmountReferenceRange>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SubstanceAmountReferenceRange {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub lowLimit: Option<Quantity>,
    pub highLimit: Option<Quantity>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Timing {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifierExtension: Option<Vec<Extension>>,
    pub event: Option<Vec<DateTime>>,
    pub repeat: Option<TimingRepeat>,
    pub code: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TimingRepeatBounds {
    Duration(Duration),
    Range(Range),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TimingRepeat {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub count: Option<PositiveInt>,
    pub countMax: Option<PositiveInt>,
    pub duration: Option<Decimal>,
    pub durationMax: Option<Decimal>,
    pub durationUnit: Option<Code>,
    pub frequency: Option<PositiveInt>,
    pub frequencyMax: Option<PositiveInt>,
    pub period: Option<Decimal>,
    pub periodMax: Option<Decimal>,
    pub periodUnit: Option<Code>,
    pub dayOfWeek: Option<Vec<Code>>,
    pub timeOfDay: Option<Vec<Time>>,
    pub when: Option<Vec<Code>>,
    pub offset: Option<UnsignedInt>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TriggerDefinitionTiming {
    Timing(Timing),
    Reference(Reference),
    Date(Date),
    DateTime(DateTime),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TriggerDefinition {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub name: Option<String>,
    pub data: Option<Vec<DataRequirement>>,
    pub condition: Option<Expression>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UsageContextValue {
    CodeableConcept(CodeableConcept),
    Quantity(Quantity),
    Range(Range),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UsageContext {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub code: Coding,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Timing {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifierExtension: Option<Vec<Extension>>,
    pub event: Option<Vec<DateTime>>,
    pub repeat: Option<TimingRepeat>,
    pub code: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TimingRepeatBounds {
    Duration(Duration),
    Range(Range),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TimingRepeat {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub count: Option<PositiveInt>,
    pub countMax: Option<PositiveInt>,
    pub duration: Option<Decimal>,
    pub durationMax: Option<Decimal>,
    pub durationUnit: Option<Code>,
    pub frequency: Option<PositiveInt>,
    pub frequencyMax: Option<PositiveInt>,
    pub period: Option<Decimal>,
    pub periodMax: Option<Decimal>,
    pub periodUnit: Option<Code>,
    pub dayOfWeek: Option<Vec<Code>>,
    pub timeOfDay: Option<Vec<Time>>,
    pub when: Option<Vec<Code>>,
    pub offset: Option<UnsignedInt>,
}
