use serde::{Serialize, Deserialize};

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
    #[serde(rename = "postalCode")]
    pub postal_code: Option<String>,
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
    #[serde(rename = "authorReference")]
    pub author_reference: Option<Reference>,
    #[serde(rename = "authorString")]
    pub author_string: Option<String>,
    pub time: Option<DateTime>,
    pub text: Markdown,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct Attachment {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "contentType")]
    pub content_type: Option<Code>,
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
    #[serde(rename = "userSelected")]
    pub user_selected: Option<Boolean>,
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
    #[serde(rename = "searchParam")]
    pub search_param: Option<String>,
    #[serde(rename = "valueDateTime")]
    pub value_date_time: Option<DateTime>,
    #[serde(rename = "valuePeriod")]
    pub value_period: Option<Period>,
    #[serde(rename = "valueDuration")]
    pub value_duration: Option<Duration>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DataRequirementCodeFilter {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub path: Option<String>,
    #[serde(rename = "searchParam")]
    pub search_param: Option<String>,
    #[serde(rename = "valueSet")]
    pub value_set: Option<Canonical>,
    pub code: Option<Vec<Coding>>,
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
    #[serde(rename = "subjectCodeableConcept")]
    pub subject_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "subjectReference")]
    pub subject_reference: Option<Reference>,
    #[serde(rename = "mustSupport")]
    pub must_support: Option<Vec<String>>,
    #[serde(rename = "codeFilter")]
    pub code_filter: Option<Vec<DataRequirementCodeFilter>>,
    #[serde(rename = "dateFilter")]
    pub date_filter: Option<Vec<DataRequirementDateFilter>>,
    pub limit: Option<PositiveInt>,
    pub sort: Option<Vec<DataRequirementSort>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DataRequirementSort {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub path: String,
    pub direction: Code,
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
pub enum DosageAsNeeded {
    Boolean(Boolean),
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Dosage {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: Option<Integer>,
    pub text: Option<String>,
    #[serde(rename = "additionalInstruction")]
    pub additional_instruction: Option<Vec<CodeableConcept>>,
    #[serde(rename = "patientInstruction")]
    pub patient_instruction: Option<String>,
    pub timing: Option<Timing>,
    #[serde(rename = "asNeededBoolean")]
    pub as_needed_boolean: Option<Boolean>,
    #[serde(rename = "asNeededCodeableConcept")]
    pub as_needed_codeable_concept: Option<CodeableConcept>,
    pub site: Option<CodeableConcept>,
    pub route: Option<CodeableConcept>,
    pub method: Option<CodeableConcept>,
    #[serde(rename = "doseAndRate")]
    pub dose_and_rate: Option<Vec<DosageDoseAndRate>>,
    #[serde(rename = "maxDosePerPeriod")]
    pub max_dose_per_period: Option<Ratio>,
    #[serde(rename = "maxDosePerAdministration")]
    pub max_dose_per_administration: Option<Quantity>,
    #[serde(rename = "maxDosePerLifetime")]
    pub max_dose_per_lifetime: Option<Quantity>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DosageDoseAndRateDose {
    Range(Range),
    Quantity(Quantity),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DosageDoseAndRateRate {
    Ratio(Ratio),
    Range(Range),
    Quantity(Quantity),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DosageDoseAndRate {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "doseRange")]
    pub dose_range: Option<Range>,
    #[serde(rename = "doseQuantity")]
    pub dose_quantity: Option<Quantity>,
    #[serde(rename = "rateRatio")]
    pub rate_ratio: Option<Ratio>,
    #[serde(rename = "rateRange")]
    pub rate_range: Option<Range>,
    #[serde(rename = "rateQuantity")]
    pub rate_quantity: Option<Quantity>,
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
pub struct ElementDefinitionSlicingDiscriminator {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub path: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ElementDefinitionBinding {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub strength: Code,
    pub description: Option<String>,
    #[serde(rename = "valueSet")]
    pub value_set: Option<Canonical>,
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
    #[serde(rename = "valueBase64Binary")]
    pub value_base64_binary: Base64Binary,
    #[serde(rename = "valueBoolean")]
    pub value_boolean: Boolean,
    #[serde(rename = "valueCanonical")]
    pub value_canonical: Canonical,
    #[serde(rename = "valueCode")]
    pub value_code: Code,
    #[serde(rename = "valueDate")]
    pub value_date: Date,
    #[serde(rename = "valueDateTime")]
    pub value_date_time: DateTime,
    #[serde(rename = "valueDecimal")]
    pub value_decimal: Decimal,
    #[serde(rename = "valueId")]
    pub value_id: Id,
    #[serde(rename = "valueInstant")]
    pub value_instant: Instant,
    #[serde(rename = "valueInteger")]
    pub value_integer: Integer,
    #[serde(rename = "valueMarkdown")]
    pub value_markdown: Markdown,
    #[serde(rename = "valueOid")]
    pub value_oid: Oid,
    #[serde(rename = "valuePositiveInt")]
    pub value_positive_int: PositiveInt,
    #[serde(rename = "valueString")]
    pub value_string: String,
    #[serde(rename = "valueTime")]
    pub value_time: Time,
    #[serde(rename = "valueUnsignedInt")]
    pub value_unsigned_int: UnsignedInt,
    #[serde(rename = "valueUri")]
    pub value_uri: Uri,
    #[serde(rename = "valueUrl")]
    pub value_url: Url,
    #[serde(rename = "valueUuid")]
    pub value_uuid: Uuid,
    #[serde(rename = "valueAddress")]
    pub value_address: Address,
    #[serde(rename = "valueAge")]
    pub value_age: Age,
    #[serde(rename = "valueAnnotation")]
    pub value_annotation: Annotation,
    #[serde(rename = "valueAttachment")]
    pub value_attachment: Attachment,
    #[serde(rename = "valueCodeableConcept")]
    pub value_codeable_concept: CodeableConcept,
    #[serde(rename = "valueCoding")]
    pub value_coding: Coding,
    #[serde(rename = "valueContactPoint")]
    pub value_contact_point: ContactPoint,
    #[serde(rename = "valueCount")]
    pub value_count: Count,
    #[serde(rename = "valueDistance")]
    pub value_distance: Distance,
    #[serde(rename = "valueDuration")]
    pub value_duration: Duration,
    #[serde(rename = "valueHumanName")]
    pub value_human_name: HumanName,
    #[serde(rename = "valueIdentifier")]
    pub value_identifier: Identifier,
    #[serde(rename = "valueMoney")]
    pub value_money: Money,
    #[serde(rename = "valuePeriod")]
    pub value_period: Period,
    #[serde(rename = "valueQuantity")]
    pub value_quantity: Quantity,
    #[serde(rename = "valueRange")]
    pub value_range: Range,
    #[serde(rename = "valueRatio")]
    pub value_ratio: Ratio,
    #[serde(rename = "valueReference")]
    pub value_reference: Reference,
    #[serde(rename = "valueSampledData")]
    pub value_sampled_data: SampledData,
    #[serde(rename = "valueSignature")]
    pub value_signature: Signature,
    #[serde(rename = "valueTiming")]
    pub value_timing: Timing,
    #[serde(rename = "valueContactDetail")]
    pub value_contact_detail: ContactDetail,
    #[serde(rename = "valueContributor")]
    pub value_contributor: Contributor,
    #[serde(rename = "valueDataRequirement")]
    pub value_data_requirement: DataRequirement,
    #[serde(rename = "valueExpression")]
    pub value_expression: Expression,
    #[serde(rename = "valueParameterDefinition")]
    pub value_parameter_definition: ParameterDefinition,
    #[serde(rename = "valueRelatedArtifact")]
    pub value_related_artifact: RelatedArtifact,
    #[serde(rename = "valueTriggerDefinition")]
    pub value_trigger_definition: TriggerDefinition,
    #[serde(rename = "valueUsageContext")]
    pub value_usage_context: UsageContext,
    #[serde(rename = "valueDosage")]
    pub value_dosage: Dosage,
    #[serde(rename = "valueMeta")]
    pub value_meta: Meta,
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
pub struct ElementDefinitionSlicing {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub discriminator: Option<Vec<ElementDefinitionSlicingDiscriminator>>,
    pub description: Option<String>,
    pub ordered: Option<Boolean>,
    pub rules: Code,
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
#[serde(rename_all = "camelCase")]
pub enum ElementDefinitionFixed {
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
#[serde(rename_all = "camelCase")]
pub enum ElementDefinitionPattern {
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
#[serde(rename_all = "camelCase")]
pub enum ElementDefinitionMinValue {
    Date(Date),
    DateTime(DateTime),
    Instant(Instant),
    Time(Time),
    Decimal(Decimal),
    Integer(Integer),
    PositiveInt(PositiveInt),
    UnsignedInt(UnsignedInt),
    Quantity(Quantity),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ElementDefinitionMaxValue {
    Date(Date),
    DateTime(DateTime),
    Instant(Instant),
    Time(Time),
    Decimal(Decimal),
    Integer(Integer),
    PositiveInt(PositiveInt),
    UnsignedInt(UnsignedInt),
    Quantity(Quantity),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ElementDefinition {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub path: String,
    pub representation: Option<Vec<Code>>,
    #[serde(rename = "sliceName")]
    pub slice_name: Option<String>,
    #[serde(rename = "sliceIsConstraining")]
    pub slice_is_constraining: Option<Boolean>,
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
    #[serde(rename = "contentReference")]
    pub content_reference: Option<Uri>,
    #[serde(rename = "type")]
    pub r#type: Option<Vec<ElementDefinitionType>>,
    #[serde(rename = "defaultValueBase64Binary")]
    pub default_value_base64_binary: Option<Base64Binary>,
    #[serde(rename = "defaultValueBoolean")]
    pub default_value_boolean: Option<Boolean>,
    #[serde(rename = "defaultValueCanonical")]
    pub default_value_canonical: Option<Canonical>,
    #[serde(rename = "defaultValueCode")]
    pub default_value_code: Option<Code>,
    #[serde(rename = "defaultValueDate")]
    pub default_value_date: Option<Date>,
    #[serde(rename = "defaultValueDateTime")]
    pub default_value_date_time: Option<DateTime>,
    #[serde(rename = "defaultValueDecimal")]
    pub default_value_decimal: Option<Decimal>,
    #[serde(rename = "defaultValueId")]
    pub default_value_id: Option<Id>,
    #[serde(rename = "defaultValueInstant")]
    pub default_value_instant: Option<Instant>,
    #[serde(rename = "defaultValueInteger")]
    pub default_value_integer: Option<Integer>,
    #[serde(rename = "defaultValueMarkdown")]
    pub default_value_markdown: Option<Markdown>,
    #[serde(rename = "defaultValueOid")]
    pub default_value_oid: Option<Oid>,
    #[serde(rename = "defaultValuePositiveInt")]
    pub default_value_positive_int: Option<PositiveInt>,
    #[serde(rename = "defaultValueString")]
    pub default_value_string: Option<String>,
    #[serde(rename = "defaultValueTime")]
    pub default_value_time: Option<Time>,
    #[serde(rename = "defaultValueUnsignedInt")]
    pub default_value_unsigned_int: Option<UnsignedInt>,
    #[serde(rename = "defaultValueUri")]
    pub default_value_uri: Option<Uri>,
    #[serde(rename = "defaultValueUrl")]
    pub default_value_url: Option<Url>,
    #[serde(rename = "defaultValueUuid")]
    pub default_value_uuid: Option<Uuid>,
    #[serde(rename = "defaultValueAddress")]
    pub default_value_address: Option<Address>,
    #[serde(rename = "defaultValueAge")]
    pub default_value_age: Option<Age>,
    #[serde(rename = "defaultValueAnnotation")]
    pub default_value_annotation: Option<Annotation>,
    #[serde(rename = "defaultValueAttachment")]
    pub default_value_attachment: Option<Attachment>,
    #[serde(rename = "defaultValueCodeableConcept")]
    pub default_value_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "defaultValueCoding")]
    pub default_value_coding: Option<Coding>,
    #[serde(rename = "defaultValueContactPoint")]
    pub default_value_contact_point: Option<ContactPoint>,
    #[serde(rename = "defaultValueCount")]
    pub default_value_count: Option<Count>,
    #[serde(rename = "defaultValueDistance")]
    pub default_value_distance: Option<Distance>,
    #[serde(rename = "defaultValueDuration")]
    pub default_value_duration: Option<Duration>,
    #[serde(rename = "defaultValueHumanName")]
    pub default_value_human_name: Option<HumanName>,
    #[serde(rename = "defaultValueIdentifier")]
    pub default_value_identifier: Option<Identifier>,
    #[serde(rename = "defaultValueMoney")]
    pub default_value_money: Option<Money>,
    #[serde(rename = "defaultValuePeriod")]
    pub default_value_period: Option<Period>,
    #[serde(rename = "defaultValueQuantity")]
    pub default_value_quantity: Option<Quantity>,
    #[serde(rename = "defaultValueRange")]
    pub default_value_range: Option<Range>,
    #[serde(rename = "defaultValueRatio")]
    pub default_value_ratio: Option<Ratio>,
    #[serde(rename = "defaultValueReference")]
    pub default_value_reference: Option<Reference>,
    #[serde(rename = "defaultValueSampledData")]
    pub default_value_sampled_data: Option<SampledData>,
    #[serde(rename = "defaultValueSignature")]
    pub default_value_signature: Option<Signature>,
    #[serde(rename = "defaultValueTiming")]
    pub default_value_timing: Option<Timing>,
    #[serde(rename = "defaultValueContactDetail")]
    pub default_value_contact_detail: Option<ContactDetail>,
    #[serde(rename = "defaultValueContributor")]
    pub default_value_contributor: Option<Contributor>,
    #[serde(rename = "defaultValueDataRequirement")]
    pub default_value_data_requirement: Option<DataRequirement>,
    #[serde(rename = "defaultValueExpression")]
    pub default_value_expression: Option<Expression>,
    #[serde(rename = "defaultValueParameterDefinition")]
    pub default_value_parameter_definition: Option<ParameterDefinition>,
    #[serde(rename = "defaultValueRelatedArtifact")]
    pub default_value_related_artifact: Option<RelatedArtifact>,
    #[serde(rename = "defaultValueTriggerDefinition")]
    pub default_value_trigger_definition: Option<TriggerDefinition>,
    #[serde(rename = "defaultValueUsageContext")]
    pub default_value_usage_context: Option<UsageContext>,
    #[serde(rename = "defaultValueDosage")]
    pub default_value_dosage: Option<Dosage>,
    #[serde(rename = "defaultValueMeta")]
    pub default_value_meta: Option<Meta>,
    #[serde(rename = "meaningWhenMissing")]
    pub meaning_when_missing: Option<Markdown>,
    #[serde(rename = "orderMeaning")]
    pub order_meaning: Option<String>,
    #[serde(rename = "fixedBase64Binary")]
    pub fixed_base64_binary: Option<Base64Binary>,
    #[serde(rename = "fixedBoolean")]
    pub fixed_boolean: Option<Boolean>,
    #[serde(rename = "fixedCanonical")]
    pub fixed_canonical: Option<Canonical>,
    #[serde(rename = "fixedCode")]
    pub fixed_code: Option<Code>,
    #[serde(rename = "fixedDate")]
    pub fixed_date: Option<Date>,
    #[serde(rename = "fixedDateTime")]
    pub fixed_date_time: Option<DateTime>,
    #[serde(rename = "fixedDecimal")]
    pub fixed_decimal: Option<Decimal>,
    #[serde(rename = "fixedId")]
    pub fixed_id: Option<Id>,
    #[serde(rename = "fixedInstant")]
    pub fixed_instant: Option<Instant>,
    #[serde(rename = "fixedInteger")]
    pub fixed_integer: Option<Integer>,
    #[serde(rename = "fixedMarkdown")]
    pub fixed_markdown: Option<Markdown>,
    #[serde(rename = "fixedOid")]
    pub fixed_oid: Option<Oid>,
    #[serde(rename = "fixedPositiveInt")]
    pub fixed_positive_int: Option<PositiveInt>,
    #[serde(rename = "fixedString")]
    pub fixed_string: Option<String>,
    #[serde(rename = "fixedTime")]
    pub fixed_time: Option<Time>,
    #[serde(rename = "fixedUnsignedInt")]
    pub fixed_unsigned_int: Option<UnsignedInt>,
    #[serde(rename = "fixedUri")]
    pub fixed_uri: Option<Uri>,
    #[serde(rename = "fixedUrl")]
    pub fixed_url: Option<Url>,
    #[serde(rename = "fixedUuid")]
    pub fixed_uuid: Option<Uuid>,
    #[serde(rename = "fixedAddress")]
    pub fixed_address: Option<Address>,
    #[serde(rename = "fixedAge")]
    pub fixed_age: Option<Age>,
    #[serde(rename = "fixedAnnotation")]
    pub fixed_annotation: Option<Annotation>,
    #[serde(rename = "fixedAttachment")]
    pub fixed_attachment: Option<Attachment>,
    #[serde(rename = "fixedCodeableConcept")]
    pub fixed_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "fixedCoding")]
    pub fixed_coding: Option<Coding>,
    #[serde(rename = "fixedContactPoint")]
    pub fixed_contact_point: Option<ContactPoint>,
    #[serde(rename = "fixedCount")]
    pub fixed_count: Option<Count>,
    #[serde(rename = "fixedDistance")]
    pub fixed_distance: Option<Distance>,
    #[serde(rename = "fixedDuration")]
    pub fixed_duration: Option<Duration>,
    #[serde(rename = "fixedHumanName")]
    pub fixed_human_name: Option<HumanName>,
    #[serde(rename = "fixedIdentifier")]
    pub fixed_identifier: Option<Identifier>,
    #[serde(rename = "fixedMoney")]
    pub fixed_money: Option<Money>,
    #[serde(rename = "fixedPeriod")]
    pub fixed_period: Option<Period>,
    #[serde(rename = "fixedQuantity")]
    pub fixed_quantity: Option<Quantity>,
    #[serde(rename = "fixedRange")]
    pub fixed_range: Option<Range>,
    #[serde(rename = "fixedRatio")]
    pub fixed_ratio: Option<Ratio>,
    #[serde(rename = "fixedReference")]
    pub fixed_reference: Option<Reference>,
    #[serde(rename = "fixedSampledData")]
    pub fixed_sampled_data: Option<SampledData>,
    #[serde(rename = "fixedSignature")]
    pub fixed_signature: Option<Signature>,
    #[serde(rename = "fixedTiming")]
    pub fixed_timing: Option<Timing>,
    #[serde(rename = "fixedContactDetail")]
    pub fixed_contact_detail: Option<ContactDetail>,
    #[serde(rename = "fixedContributor")]
    pub fixed_contributor: Option<Contributor>,
    #[serde(rename = "fixedDataRequirement")]
    pub fixed_data_requirement: Option<DataRequirement>,
    #[serde(rename = "fixedExpression")]
    pub fixed_expression: Option<Expression>,
    #[serde(rename = "fixedParameterDefinition")]
    pub fixed_parameter_definition: Option<ParameterDefinition>,
    #[serde(rename = "fixedRelatedArtifact")]
    pub fixed_related_artifact: Option<RelatedArtifact>,
    #[serde(rename = "fixedTriggerDefinition")]
    pub fixed_trigger_definition: Option<TriggerDefinition>,
    #[serde(rename = "fixedUsageContext")]
    pub fixed_usage_context: Option<UsageContext>,
    #[serde(rename = "fixedDosage")]
    pub fixed_dosage: Option<Dosage>,
    #[serde(rename = "fixedMeta")]
    pub fixed_meta: Option<Meta>,
    #[serde(rename = "patternBase64Binary")]
    pub pattern_base64_binary: Option<Base64Binary>,
    #[serde(rename = "patternBoolean")]
    pub pattern_boolean: Option<Boolean>,
    #[serde(rename = "patternCanonical")]
    pub pattern_canonical: Option<Canonical>,
    #[serde(rename = "patternCode")]
    pub pattern_code: Option<Code>,
    #[serde(rename = "patternDate")]
    pub pattern_date: Option<Date>,
    #[serde(rename = "patternDateTime")]
    pub pattern_date_time: Option<DateTime>,
    #[serde(rename = "patternDecimal")]
    pub pattern_decimal: Option<Decimal>,
    #[serde(rename = "patternId")]
    pub pattern_id: Option<Id>,
    #[serde(rename = "patternInstant")]
    pub pattern_instant: Option<Instant>,
    #[serde(rename = "patternInteger")]
    pub pattern_integer: Option<Integer>,
    #[serde(rename = "patternMarkdown")]
    pub pattern_markdown: Option<Markdown>,
    #[serde(rename = "patternOid")]
    pub pattern_oid: Option<Oid>,
    #[serde(rename = "patternPositiveInt")]
    pub pattern_positive_int: Option<PositiveInt>,
    #[serde(rename = "patternString")]
    pub pattern_string: Option<String>,
    #[serde(rename = "patternTime")]
    pub pattern_time: Option<Time>,
    #[serde(rename = "patternUnsignedInt")]
    pub pattern_unsigned_int: Option<UnsignedInt>,
    #[serde(rename = "patternUri")]
    pub pattern_uri: Option<Uri>,
    #[serde(rename = "patternUrl")]
    pub pattern_url: Option<Url>,
    #[serde(rename = "patternUuid")]
    pub pattern_uuid: Option<Uuid>,
    #[serde(rename = "patternAddress")]
    pub pattern_address: Option<Address>,
    #[serde(rename = "patternAge")]
    pub pattern_age: Option<Age>,
    #[serde(rename = "patternAnnotation")]
    pub pattern_annotation: Option<Annotation>,
    #[serde(rename = "patternAttachment")]
    pub pattern_attachment: Option<Attachment>,
    #[serde(rename = "patternCodeableConcept")]
    pub pattern_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "patternCoding")]
    pub pattern_coding: Option<Coding>,
    #[serde(rename = "patternContactPoint")]
    pub pattern_contact_point: Option<ContactPoint>,
    #[serde(rename = "patternCount")]
    pub pattern_count: Option<Count>,
    #[serde(rename = "patternDistance")]
    pub pattern_distance: Option<Distance>,
    #[serde(rename = "patternDuration")]
    pub pattern_duration: Option<Duration>,
    #[serde(rename = "patternHumanName")]
    pub pattern_human_name: Option<HumanName>,
    #[serde(rename = "patternIdentifier")]
    pub pattern_identifier: Option<Identifier>,
    #[serde(rename = "patternMoney")]
    pub pattern_money: Option<Money>,
    #[serde(rename = "patternPeriod")]
    pub pattern_period: Option<Period>,
    #[serde(rename = "patternQuantity")]
    pub pattern_quantity: Option<Quantity>,
    #[serde(rename = "patternRange")]
    pub pattern_range: Option<Range>,
    #[serde(rename = "patternRatio")]
    pub pattern_ratio: Option<Ratio>,
    #[serde(rename = "patternReference")]
    pub pattern_reference: Option<Reference>,
    #[serde(rename = "patternSampledData")]
    pub pattern_sampled_data: Option<SampledData>,
    #[serde(rename = "patternSignature")]
    pub pattern_signature: Option<Signature>,
    #[serde(rename = "patternTiming")]
    pub pattern_timing: Option<Timing>,
    #[serde(rename = "patternContactDetail")]
    pub pattern_contact_detail: Option<ContactDetail>,
    #[serde(rename = "patternContributor")]
    pub pattern_contributor: Option<Contributor>,
    #[serde(rename = "patternDataRequirement")]
    pub pattern_data_requirement: Option<DataRequirement>,
    #[serde(rename = "patternExpression")]
    pub pattern_expression: Option<Expression>,
    #[serde(rename = "patternParameterDefinition")]
    pub pattern_parameter_definition: Option<ParameterDefinition>,
    #[serde(rename = "patternRelatedArtifact")]
    pub pattern_related_artifact: Option<RelatedArtifact>,
    #[serde(rename = "patternTriggerDefinition")]
    pub pattern_trigger_definition: Option<TriggerDefinition>,
    #[serde(rename = "patternUsageContext")]
    pub pattern_usage_context: Option<UsageContext>,
    #[serde(rename = "patternDosage")]
    pub pattern_dosage: Option<Dosage>,
    #[serde(rename = "patternMeta")]
    pub pattern_meta: Option<Meta>,
    pub example: Option<Vec<ElementDefinitionExample>>,
    #[serde(rename = "minValueDate")]
    pub min_value_date: Option<Date>,
    #[serde(rename = "minValueDateTime")]
    pub min_value_date_time: Option<DateTime>,
    #[serde(rename = "minValueInstant")]
    pub min_value_instant: Option<Instant>,
    #[serde(rename = "minValueTime")]
    pub min_value_time: Option<Time>,
    #[serde(rename = "minValueDecimal")]
    pub min_value_decimal: Option<Decimal>,
    #[serde(rename = "minValueInteger")]
    pub min_value_integer: Option<Integer>,
    #[serde(rename = "minValuePositiveInt")]
    pub min_value_positive_int: Option<PositiveInt>,
    #[serde(rename = "minValueUnsignedInt")]
    pub min_value_unsigned_int: Option<UnsignedInt>,
    #[serde(rename = "minValueQuantity")]
    pub min_value_quantity: Option<Quantity>,
    #[serde(rename = "maxValueDate")]
    pub max_value_date: Option<Date>,
    #[serde(rename = "maxValueDateTime")]
    pub max_value_date_time: Option<DateTime>,
    #[serde(rename = "maxValueInstant")]
    pub max_value_instant: Option<Instant>,
    #[serde(rename = "maxValueTime")]
    pub max_value_time: Option<Time>,
    #[serde(rename = "maxValueDecimal")]
    pub max_value_decimal: Option<Decimal>,
    #[serde(rename = "maxValueInteger")]
    pub max_value_integer: Option<Integer>,
    #[serde(rename = "maxValuePositiveInt")]
    pub max_value_positive_int: Option<PositiveInt>,
    #[serde(rename = "maxValueUnsignedInt")]
    pub max_value_unsigned_int: Option<UnsignedInt>,
    #[serde(rename = "maxValueQuantity")]
    pub max_value_quantity: Option<Quantity>,
    #[serde(rename = "maxLength")]
    pub max_length: Option<Integer>,
    pub condition: Option<Vec<Id>>,
    pub constraint: Option<Vec<ElementDefinitionConstraint>>,
    #[serde(rename = "mustSupport")]
    pub must_support: Option<Boolean>,
    #[serde(rename = "isModifier")]
    pub is_modifier: Option<Boolean>,
    #[serde(rename = "isModifierReason")]
    pub is_modifier_reason: Option<String>,
    #[serde(rename = "isSummary")]
    pub is_summary: Option<Boolean>,
    pub binding: Option<ElementDefinitionBinding>,
    pub mapping: Option<Vec<ElementDefinitionMapping>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ElementDefinitionType {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub code: Uri,
    pub profile: Option<Vec<Canonical>>,
    #[serde(rename = "targetProfile")]
    pub target_profile: Option<Vec<Canonical>>,
    pub aggregation: Option<Vec<Code>>,
    pub versioning: Option<Code>,
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
    #[serde(rename = "valueBase64Binary")]
    pub value_base64_binary: Option<Base64Binary>,
    #[serde(rename = "valueBoolean")]
    pub value_boolean: Option<Boolean>,
    #[serde(rename = "valueCanonical")]
    pub value_canonical: Option<Canonical>,
    #[serde(rename = "valueCode")]
    pub value_code: Option<Code>,
    #[serde(rename = "valueDate")]
    pub value_date: Option<Date>,
    #[serde(rename = "valueDateTime")]
    pub value_date_time: Option<DateTime>,
    #[serde(rename = "valueDecimal")]
    pub value_decimal: Option<Decimal>,
    #[serde(rename = "valueId")]
    pub value_id: Option<Id>,
    #[serde(rename = "valueInstant")]
    pub value_instant: Option<Instant>,
    #[serde(rename = "valueInteger")]
    pub value_integer: Option<Integer>,
    #[serde(rename = "valueMarkdown")]
    pub value_markdown: Option<Markdown>,
    #[serde(rename = "valueOid")]
    pub value_oid: Option<Oid>,
    #[serde(rename = "valuePositiveInt")]
    pub value_positive_int: Option<PositiveInt>,
    #[serde(rename = "valueString")]
    pub value_string: Option<String>,
    #[serde(rename = "valueTime")]
    pub value_time: Option<Time>,
    #[serde(rename = "valueUnsignedInt")]
    pub value_unsigned_int: Option<UnsignedInt>,
    #[serde(rename = "valueUri")]
    pub value_uri: Option<Uri>,
    #[serde(rename = "valueUrl")]
    pub value_url: Option<Url>,
    #[serde(rename = "valueUuid")]
    pub value_uuid: Option<Uuid>,
    #[serde(rename = "valueAddress")]
    pub value_address: Option<Address>,
    #[serde(rename = "valueAge")]
    pub value_age: Option<Age>,
    #[serde(rename = "valueAnnotation")]
    pub value_annotation: Option<Annotation>,
    #[serde(rename = "valueAttachment")]
    pub value_attachment: Option<Attachment>,
    #[serde(rename = "valueCodeableConcept")]
    pub value_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "valueCoding")]
    pub value_coding: Option<Coding>,
    #[serde(rename = "valueContactPoint")]
    pub value_contact_point: Option<ContactPoint>,
    #[serde(rename = "valueCount")]
    pub value_count: Option<Count>,
    #[serde(rename = "valueDistance")]
    pub value_distance: Option<Distance>,
    #[serde(rename = "valueDuration")]
    pub value_duration: Option<Duration>,
    #[serde(rename = "valueHumanName")]
    pub value_human_name: Option<HumanName>,
    #[serde(rename = "valueIdentifier")]
    pub value_identifier: Option<Identifier>,
    #[serde(rename = "valueMoney")]
    pub value_money: Option<Money>,
    #[serde(rename = "valuePeriod")]
    pub value_period: Option<Period>,
    #[serde(rename = "valueQuantity")]
    pub value_quantity: Option<Quantity>,
    #[serde(rename = "valueRange")]
    pub value_range: Option<Range>,
    #[serde(rename = "valueRatio")]
    pub value_ratio: Option<Ratio>,
    #[serde(rename = "valueReference")]
    pub value_reference: Option<Reference>,
    #[serde(rename = "valueSampledData")]
    pub value_sampled_data: Option<SampledData>,
    #[serde(rename = "valueSignature")]
    pub value_signature: Option<Signature>,
    #[serde(rename = "valueTiming")]
    pub value_timing: Option<Timing>,
    #[serde(rename = "valueContactDetail")]
    pub value_contact_detail: Option<ContactDetail>,
    #[serde(rename = "valueContributor")]
    pub value_contributor: Option<Contributor>,
    #[serde(rename = "valueDataRequirement")]
    pub value_data_requirement: Option<DataRequirement>,
    #[serde(rename = "valueExpression")]
    pub value_expression: Option<Expression>,
    #[serde(rename = "valueParameterDefinition")]
    pub value_parameter_definition: Option<ParameterDefinition>,
    #[serde(rename = "valueRelatedArtifact")]
    pub value_related_artifact: Option<RelatedArtifact>,
    #[serde(rename = "valueTriggerDefinition")]
    pub value_trigger_definition: Option<TriggerDefinition>,
    #[serde(rename = "valueUsageContext")]
    pub value_usage_context: Option<UsageContext>,
    #[serde(rename = "valueDosage")]
    pub value_dosage: Option<Dosage>,
    #[serde(rename = "valueMeta")]
    pub value_meta: Option<Meta>,
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
    pub assigner: Option<Box<Reference>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct MarketingStatus {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub country: CodeableConcept,
    pub jurisdiction: Option<CodeableConcept>,
    pub status: CodeableConcept,
    #[serde(rename = "dateRange")]
    pub date_range: Period,
    #[serde(rename = "restoreDate")]
    pub restore_date: Option<DateTime>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct Meta {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "versionId")]
    pub version_id: Option<Id>,
    #[serde(rename = "lastUpdated")]
    pub last_updated: Option<Instant>,
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
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "ageRange")]
    pub age_range: Option<Range>,
    #[serde(rename = "ageCodeableConcept")]
    pub age_codeable_concept: Option<CodeableConcept>,
    pub gender: Option<CodeableConcept>,
    pub race: Option<CodeableConcept>,
    #[serde(rename = "physiologicalCondition")]
    pub physiological_condition: Option<CodeableConcept>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct ProdCharacteristic {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub height: Option<Quantity>,
    pub width: Option<Quantity>,
    pub depth: Option<Quantity>,
    pub weight: Option<Quantity>,
    #[serde(rename = "nominalVolume")]
    pub nominal_volume: Option<Quantity>,
    #[serde(rename = "externalDiameter")]
    pub external_diameter: Option<Quantity>,
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
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub period: Quantity,
    #[serde(rename = "specialPrecautionsForStorage")]
    pub special_precautions_for_storage: Option<Vec<CodeableConcept>>,
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
    pub identifier: Option<Box<Identifier>>,
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
    #[serde(rename = "lowerLimit")]
    pub lower_limit: Option<Decimal>,
    #[serde(rename = "upperLimit")]
    pub upper_limit: Option<Decimal>,
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
    #[serde(rename = "onBehalfOf")]
    pub on_behalf_of: Option<Reference>,
    #[serde(rename = "targetFormat")]
    pub target_format: Option<Code>,
    #[serde(rename = "sigFormat")]
    pub sig_format: Option<Code>,
    pub data: Option<Base64Binary>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct SubstanceAmountReferenceRange {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "lowLimit")]
    pub low_limit: Option<Quantity>,
    #[serde(rename = "highLimit")]
    pub high_limit: Option<Quantity>,
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
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "amountQuantity")]
    pub amount_quantity: Option<Quantity>,
    #[serde(rename = "amountRange")]
    pub amount_range: Option<Range>,
    #[serde(rename = "amountString")]
    pub amount_string: Option<String>,
    #[serde(rename = "amountType")]
    pub amount_type: Option<CodeableConcept>,
    #[serde(rename = "amountText")]
    pub amount_text: Option<String>,
    #[serde(rename = "referenceRange")]
    pub reference_range: Option<SubstanceAmountReferenceRange>,
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
    #[serde(rename = "boundsDuration")]
    pub bounds_duration: Option<Duration>,
    #[serde(rename = "boundsRange")]
    pub bounds_range: Option<Range>,
    #[serde(rename = "boundsPeriod")]
    pub bounds_period: Option<Period>,
    pub count: Option<PositiveInt>,
    #[serde(rename = "countMax")]
    pub count_max: Option<PositiveInt>,
    pub duration: Option<Decimal>,
    #[serde(rename = "durationMax")]
    pub duration_max: Option<Decimal>,
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
    pub day_of_week: Option<Vec<Code>>,
    #[serde(rename = "timeOfDay")]
    pub time_of_day: Option<Vec<Time>>,
    pub when: Option<Vec<Code>>,
    pub offset: Option<UnsignedInt>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Timing {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub event: Option<Vec<DateTime>>,
    pub repeat: Option<TimingRepeat>,
    pub code: Option<CodeableConcept>,
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
    #[serde(rename = "timingTiming")]
    pub timing_timing: Option<Timing>,
    #[serde(rename = "timingReference")]
    pub timing_reference: Option<Reference>,
    #[serde(rename = "timingDate")]
    pub timing_date: Option<Date>,
    #[serde(rename = "timingDateTime")]
    pub timing_date_time: Option<DateTime>,
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
    #[serde(rename = "valueCodeableConcept")]
    pub value_codeable_concept: CodeableConcept,
    #[serde(rename = "valueQuantity")]
    pub value_quantity: Quantity,
    #[serde(rename = "valueRange")]
    pub value_range: Range,
    #[serde(rename = "valueReference")]
    pub value_reference: Reference,
}


