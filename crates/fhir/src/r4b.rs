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
pub struct CodeableReference {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub concept: Option<CodeableConcept>,
    pub reference: Option<Reference>,
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
pub struct ElementDefinitionMapping {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub identity: Id,
    pub language: Option<Code>,
    pub map: String,
    pub comment: Option<String>,
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
pub struct ElementDefinitionSlicingDiscriminator {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub path: String,
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
    CodeableReference(CodeableReference),
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
    RatioRange(RatioRange),
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
    #[serde(rename = "valueCodeableReference")]
    pub value_codeable_reference: CodeableReference,
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
    #[serde(rename = "valueRatioRange")]
    pub value_ratio_range: RatioRange,
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
pub struct ElementDefinitionBinding {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub strength: Code,
    pub description: Option<String>,
    #[serde(rename = "valueSet")]
    pub value_set: Option<Canonical>,
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
    CodeableReference(CodeableReference),
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
    RatioRange(RatioRange),
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
    CodeableReference(CodeableReference),
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
    RatioRange(RatioRange),
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
    CodeableReference(CodeableReference),
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
    RatioRange(RatioRange),
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
    #[serde(rename = "defaultValueCodeableReference")]
    pub default_value_codeable_reference: Option<CodeableReference>,
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
    #[serde(rename = "defaultValueRatioRange")]
    pub default_value_ratio_range: Option<RatioRange>,
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
    #[serde(rename = "fixedCodeableReference")]
    pub fixed_codeable_reference: Option<CodeableReference>,
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
    #[serde(rename = "fixedRatioRange")]
    pub fixed_ratio_range: Option<RatioRange>,
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
    #[serde(rename = "patternCodeableReference")]
    pub pattern_codeable_reference: Option<CodeableReference>,
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
    #[serde(rename = "patternRatioRange")]
    pub pattern_ratio_range: Option<RatioRange>,
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
pub struct ElementDefinitionSlicing {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub discriminator: Option<Vec<ElementDefinitionSlicingDiscriminator>>,
    pub description: Option<String>,
    pub ordered: Option<Boolean>,
    pub rules: Code,
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
    CodeableReference(CodeableReference),
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
    RatioRange(RatioRange),
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
    #[serde(rename = "valueCodeableReference")]
    pub value_codeable_reference: Option<CodeableReference>,
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
    #[serde(rename = "valueRatioRange")]
    pub value_ratio_range: Option<RatioRange>,
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
    pub country: Option<CodeableConcept>,
    pub jurisdiction: Option<CodeableConcept>,
    pub status: CodeableConcept,
    #[serde(rename = "dateRange")]
    pub date_range: Option<Period>,
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
pub struct RatioRange {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "lowNumerator")]
    pub low_numerator: Option<Quantity>,
    #[serde(rename = "highNumerator")]
    pub high_numerator: Option<Quantity>,
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


#[derive(Debug, Serialize, Deserialize)]
pub struct AccountCoverage {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub coverage: Reference,
    pub priority: Option<PositiveInt>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AccountGuarantor {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub party: Reference,
    #[serde(rename = "onHold")]
    pub on_hold: Option<Boolean>,
    pub period: Option<Period>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Account {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub name: Option<String>,
    pub subject: Option<Vec<Reference>>,
    #[serde(rename = "servicePeriod")]
    pub service_period: Option<Period>,
    pub coverage: Option<Vec<AccountCoverage>>,
    pub owner: Option<Reference>,
    pub description: Option<String>,
    pub guarantor: Option<Vec<AccountGuarantor>>,
    #[serde(rename = "partOf")]
    pub part_of: Option<Reference>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct ActivityDefinitionParticipant {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub role: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ActivityDefinitionDynamicValue {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub path: String,
    pub expression: Expression,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActivityDefinitionSubject {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
    Canonical(Canonical),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActivityDefinitionTiming {
    Timing(Timing),
    DateTime(DateTime),
    Age(Age),
    Period(Period),
    Range(Range),
    Duration(Duration),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActivityDefinitionProduct {
    Reference(Reference),
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ActivityDefinition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    #[serde(rename = "subjectCodeableConcept")]
    pub subject_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "subjectReference")]
    pub subject_reference: Option<Reference>,
    #[serde(rename = "subjectCanonical")]
    pub subject_canonical: Option<Canonical>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub usage: Option<String>,
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod")]
    pub effective_period: Option<Period>,
    pub topic: Option<Vec<CodeableConcept>>,
    pub author: Option<Vec<ContactDetail>>,
    pub editor: Option<Vec<ContactDetail>>,
    pub reviewer: Option<Vec<ContactDetail>>,
    pub endorser: Option<Vec<ContactDetail>>,
    #[serde(rename = "relatedArtifact")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    pub library: Option<Vec<Canonical>>,
    pub kind: Option<Code>,
    pub profile: Option<Canonical>,
    pub code: Option<CodeableConcept>,
    pub intent: Option<Code>,
    pub priority: Option<Code>,
    #[serde(rename = "doNotPerform")]
    pub do_not_perform: Option<Boolean>,
    #[serde(rename = "timingTiming")]
    pub timing_timing: Option<Timing>,
    #[serde(rename = "timingDateTime")]
    pub timing_date_time: Option<DateTime>,
    #[serde(rename = "timingAge")]
    pub timing_age: Option<Age>,
    #[serde(rename = "timingPeriod")]
    pub timing_period: Option<Period>,
    #[serde(rename = "timingRange")]
    pub timing_range: Option<Range>,
    #[serde(rename = "timingDuration")]
    pub timing_duration: Option<Duration>,
    pub location: Option<Reference>,
    pub participant: Option<Vec<ActivityDefinitionParticipant>>,
    #[serde(rename = "productReference")]
    pub product_reference: Option<Reference>,
    #[serde(rename = "productCodeableConcept")]
    pub product_codeable_concept: Option<CodeableConcept>,
    pub quantity: Option<Quantity>,
    pub dosage: Option<Vec<Dosage>>,
    #[serde(rename = "bodySite")]
    pub body_site: Option<Vec<CodeableConcept>>,
    #[serde(rename = "specimenRequirement")]
    pub specimen_requirement: Option<Vec<Reference>>,
    #[serde(rename = "observationRequirement")]
    pub observation_requirement: Option<Vec<Reference>>,
    #[serde(rename = "observationResultRequirement")]
    pub observation_result_requirement: Option<Vec<Reference>>,
    pub transform: Option<Canonical>,
    #[serde(rename = "dynamicValue")]
    pub dynamic_value: Option<Vec<ActivityDefinitionDynamicValue>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AdministrableProductDefinitionPropertyValue {
    CodeableConcept(CodeableConcept),
    Quantity(Quantity),
    Date(Date),
    Boolean(Boolean),
    Attachment(Attachment),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AdministrableProductDefinitionProperty {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(rename = "valueCodeableConcept")]
    pub value_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "valueQuantity")]
    pub value_quantity: Option<Quantity>,
    #[serde(rename = "valueDate")]
    pub value_date: Option<Date>,
    #[serde(rename = "valueBoolean")]
    pub value_boolean: Option<Boolean>,
    #[serde(rename = "valueAttachment")]
    pub value_attachment: Option<Attachment>,
    pub status: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AdministrableProductDefinitionRouteOfAdministration {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    #[serde(rename = "firstDose")]
    pub first_dose: Option<Quantity>,
    #[serde(rename = "maxSingleDose")]
    pub max_single_dose: Option<Quantity>,
    #[serde(rename = "maxDosePerDay")]
    pub max_dose_per_day: Option<Quantity>,
    #[serde(rename = "maxDosePerTreatmentPeriod")]
    pub max_dose_per_treatment_period: Option<Ratio>,
    #[serde(rename = "maxTreatmentPeriod")]
    pub max_treatment_period: Option<Duration>,
    #[serde(rename = "targetSpecies")]
    pub target_species: Option<Vec<AdministrableProductDefinitionRouteOfAdministrationTargetSpecies>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AdministrableProductDefinitionRouteOfAdministrationTargetSpecies {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    #[serde(rename = "withdrawalPeriod")]
    pub withdrawal_period: Option<Vec<AdministrableProductDefinitionRouteOfAdministrationTargetSpeciesWithdrawalPeriod>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AdministrableProductDefinition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "formOf")]
    pub form_of: Option<Vec<Reference>>,
    #[serde(rename = "administrableDoseForm")]
    pub administrable_dose_form: Option<CodeableConcept>,
    #[serde(rename = "unitOfPresentation")]
    pub unit_of_presentation: Option<CodeableConcept>,
    #[serde(rename = "producedFrom")]
    pub produced_from: Option<Vec<Reference>>,
    pub ingredient: Option<Vec<CodeableConcept>>,
    pub device: Option<Reference>,
    pub property: Option<Vec<AdministrableProductDefinitionProperty>>,
    #[serde(rename = "routeOfAdministration")]
    pub route_of_administration: Option<Vec<AdministrableProductDefinitionRouteOfAdministration>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AdministrableProductDefinitionRouteOfAdministrationTargetSpeciesWithdrawalPeriod {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub tissue: CodeableConcept,
    pub value: Quantity,
    #[serde(rename = "supportingInformation")]
    pub supporting_information: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct AdverseEventSuspectEntityCausality {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub assessment: Option<CodeableConcept>,
    #[serde(rename = "productRelatedness")]
    pub product_relatedness: Option<String>,
    pub author: Option<Reference>,
    pub method: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AdverseEventSuspectEntity {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub instance: Reference,
    pub causality: Option<Vec<AdverseEventSuspectEntityCausality>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AdverseEvent {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    pub actuality: Code,
    pub category: Option<Vec<CodeableConcept>>,
    pub event: Option<CodeableConcept>,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    pub date: Option<DateTime>,
    pub detected: Option<DateTime>,
    #[serde(rename = "recordedDate")]
    pub recorded_date: Option<DateTime>,
    #[serde(rename = "resultingCondition")]
    pub resulting_condition: Option<Vec<Reference>>,
    pub location: Option<Reference>,
    pub seriousness: Option<CodeableConcept>,
    pub severity: Option<CodeableConcept>,
    pub outcome: Option<CodeableConcept>,
    pub recorder: Option<Reference>,
    pub contributor: Option<Vec<Reference>>,
    #[serde(rename = "suspectEntity")]
    pub suspect_entity: Option<Vec<AdverseEventSuspectEntity>>,
    #[serde(rename = "subjectMedicalHistory")]
    pub subject_medical_history: Option<Vec<Reference>>,
    #[serde(rename = "referenceDocument")]
    pub reference_document: Option<Vec<Reference>>,
    pub study: Option<Vec<Reference>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct AllergyIntoleranceReaction {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub substance: Option<CodeableConcept>,
    pub manifestation: Option<Vec<CodeableConcept>>,
    pub description: Option<String>,
    pub onset: Option<DateTime>,
    pub severity: Option<Code>,
    #[serde(rename = "exposureRoute")]
    pub exposure_route: Option<CodeableConcept>,
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AllergyIntoleranceOnset {
    DateTime(DateTime),
    Age(Age),
    Period(Period),
    Range(Range),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AllergyIntolerance {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "clinicalStatus")]
    pub clinical_status: Option<CodeableConcept>,
    #[serde(rename = "verificationStatus")]
    pub verification_status: Option<CodeableConcept>,
    #[serde(rename = "type")]
    pub r#type: Option<Code>,
    pub category: Option<Vec<Code>>,
    pub criticality: Option<Code>,
    pub code: Option<CodeableConcept>,
    pub patient: Reference,
    pub encounter: Option<Reference>,
    #[serde(rename = "onsetDateTime")]
    pub onset_date_time: Option<DateTime>,
    #[serde(rename = "onsetAge")]
    pub onset_age: Option<Age>,
    #[serde(rename = "onsetPeriod")]
    pub onset_period: Option<Period>,
    #[serde(rename = "onsetRange")]
    pub onset_range: Option<Range>,
    #[serde(rename = "onsetString")]
    pub onset_string: Option<String>,
    #[serde(rename = "recordedDate")]
    pub recorded_date: Option<DateTime>,
    pub recorder: Option<Reference>,
    pub asserter: Option<Reference>,
    #[serde(rename = "lastOccurrence")]
    pub last_occurrence: Option<DateTime>,
    pub note: Option<Vec<Annotation>>,
    pub reaction: Option<Vec<AllergyIntoleranceReaction>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct AppointmentParticipant {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    pub actor: Option<Reference>,
    pub required: Option<Code>,
    pub status: Code,
    pub period: Option<Period>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Appointment {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "cancelationReason")]
    pub cancelation_reason: Option<CodeableConcept>,
    #[serde(rename = "serviceCategory")]
    pub service_category: Option<Vec<CodeableConcept>>,
    #[serde(rename = "serviceType")]
    pub service_type: Option<Vec<CodeableConcept>>,
    pub specialty: Option<Vec<CodeableConcept>>,
    #[serde(rename = "appointmentType")]
    pub appointment_type: Option<CodeableConcept>,
    #[serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    pub priority: Option<UnsignedInt>,
    pub description: Option<String>,
    #[serde(rename = "supportingInformation")]
    pub supporting_information: Option<Vec<Reference>>,
    pub start: Option<Instant>,
    pub end: Option<Instant>,
    #[serde(rename = "minutesDuration")]
    pub minutes_duration: Option<PositiveInt>,
    pub slot: Option<Vec<Reference>>,
    pub created: Option<DateTime>,
    pub comment: Option<String>,
    #[serde(rename = "patientInstruction")]
    pub patient_instruction: Option<String>,
    #[serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    pub participant: Option<Vec<AppointmentParticipant>>,
    #[serde(rename = "requestedPeriod")]
    pub requested_period: Option<Vec<Period>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct AppointmentResponse {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub appointment: Reference,
    pub start: Option<Instant>,
    pub end: Option<Instant>,
    #[serde(rename = "participantType")]
    pub participant_type: Option<Vec<CodeableConcept>>,
    pub actor: Option<Reference>,
    #[serde(rename = "participantStatus")]
    pub participant_status: Code,
    pub comment: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Coding,
    pub subtype: Option<Vec<Coding>>,
    pub action: Option<Code>,
    pub period: Option<Period>,
    pub recorded: Instant,
    pub outcome: Option<Code>,
    #[serde(rename = "outcomeDesc")]
    pub outcome_desc: Option<String>,
    #[serde(rename = "purposeOfEvent")]
    pub purpose_of_event: Option<Vec<CodeableConcept>>,
    pub agent: Option<Vec<AuditEventAgent>>,
    pub source: AuditEventSource,
    pub entity: Option<Vec<AuditEventEntity>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AuditEventEntity {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub what: Option<Reference>,
    #[serde(rename = "type")]
    pub r#type: Option<Coding>,
    pub role: Option<Coding>,
    pub lifecycle: Option<Coding>,
    #[serde(rename = "securityLabel")]
    pub security_label: Option<Vec<Coding>>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub query: Option<Base64Binary>,
    pub detail: Option<Vec<AuditEventEntityDetail>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AuditEventAgent {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub role: Option<Vec<CodeableConcept>>,
    pub who: Option<Reference>,
    #[serde(rename = "altId")]
    pub alt_id: Option<String>,
    pub name: Option<String>,
    pub requestor: Boolean,
    pub location: Option<Reference>,
    pub policy: Option<Vec<Uri>>,
    pub media: Option<Coding>,
    pub network: Option<AuditEventAgentNetwork>,
    #[serde(rename = "purposeOfUse")]
    pub purpose_of_use: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AuditEventAgentNetwork {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub address: Option<String>,
    #[serde(rename = "type")]
    pub r#type: Option<Code>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AuditEventSource {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub site: Option<String>,
    pub observer: Reference,
    #[serde(rename = "type")]
    pub r#type: Option<Vec<Coding>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AuditEventEntityDetailValue {
    String(String),
    Base64Binary(Base64Binary),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AuditEventEntityDetail {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: String,
    #[serde(rename = "valueString")]
    pub value_string: String,
    #[serde(rename = "valueBase64Binary")]
    pub value_base64_binary: Base64Binary,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct Basic {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub code: CodeableConcept,
    pub subject: Option<Reference>,
    pub created: Option<Date>,
    pub author: Option<Reference>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct Binary {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    #[serde(rename = "contentType")]
    pub content_type: Code,
    #[serde(rename = "securityContext")]
    pub security_context: Option<Reference>,
    pub data: Option<Base64Binary>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct BiologicallyDerivedProductStorage {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    pub temperature: Option<Decimal>,
    pub scale: Option<Code>,
    pub duration: Option<Period>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BiologicallyDerivedProduct {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "productCategory")]
    pub product_category: Option<Code>,
    #[serde(rename = "productCode")]
    pub product_code: Option<CodeableConcept>,
    pub status: Option<Code>,
    pub request: Option<Vec<Reference>>,
    pub quantity: Option<Integer>,
    pub parent: Option<Vec<Reference>>,
    pub collection: Option<BiologicallyDerivedProductCollection>,
    pub processing: Option<Vec<BiologicallyDerivedProductProcessing>>,
    pub manipulation: Option<BiologicallyDerivedProductManipulation>,
    pub storage: Option<Vec<BiologicallyDerivedProductStorage>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BiologicallyDerivedProductCollectionCollected {
    DateTime(DateTime),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BiologicallyDerivedProductCollection {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub collector: Option<Reference>,
    pub source: Option<Reference>,
    #[serde(rename = "collectedDateTime")]
    pub collected_date_time: Option<DateTime>,
    #[serde(rename = "collectedPeriod")]
    pub collected_period: Option<Period>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BiologicallyDerivedProductManipulationTime {
    DateTime(DateTime),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BiologicallyDerivedProductManipulation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    #[serde(rename = "timeDateTime")]
    pub time_date_time: Option<DateTime>,
    #[serde(rename = "timePeriod")]
    pub time_period: Option<Period>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BiologicallyDerivedProductProcessingTime {
    DateTime(DateTime),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BiologicallyDerivedProductProcessing {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    pub procedure: Option<CodeableConcept>,
    pub additive: Option<Reference>,
    #[serde(rename = "timeDateTime")]
    pub time_date_time: Option<DateTime>,
    #[serde(rename = "timePeriod")]
    pub time_period: Option<Period>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct BodyStructure {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub active: Option<Boolean>,
    pub morphology: Option<CodeableConcept>,
    pub location: Option<CodeableConcept>,
    #[serde(rename = "locationQualifier")]
    pub location_qualifier: Option<Vec<CodeableConcept>>,
    pub description: Option<String>,
    pub image: Option<Vec<Attachment>>,
    pub patient: Reference,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct BundleEntryResponse {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub status: String,
    pub location: Option<Uri>,
    pub etag: Option<String>,
    #[serde(rename = "lastModified")]
    pub last_modified: Option<Instant>,
    pub outcome: Option<Resource>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BundleLink {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub relation: String,
    pub url: Uri,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BundleEntryRequest {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
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
pub struct BundleEntrySearch {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub mode: Option<Code>,
    pub score: Option<Decimal>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Bundle {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub identifier: Option<Identifier>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub timestamp: Option<Instant>,
    pub total: Option<UnsignedInt>,
    pub link: Option<Vec<BundleLink>>,
    pub entry: Option<Vec<BundleEntry>>,
    pub signature: Option<Signature>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BundleEntry {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub link: Option<Vec<BundleLink>>,
    #[serde(rename = "fullUrl")]
    pub full_url: Option<Uri>,
    pub resource: Option<Resource>,
    pub search: Option<BundleEntrySearch>,
    pub request: Option<BundleEntryRequest>,
    pub response: Option<BundleEntryResponse>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct CapabilityStatementImplementation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: String,
    pub url: Option<Url>,
    pub custodian: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CapabilityStatementRestResourceOperation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub definition: Canonical,
    pub documentation: Option<Markdown>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CapabilityStatementMessagingSupportedMessage {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub mode: Code,
    pub definition: Canonical,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CapabilityStatementRestSecurity {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub cors: Option<Boolean>,
    pub service: Option<Vec<CodeableConcept>>,
    pub description: Option<Markdown>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CapabilityStatementRestResourceInteraction {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub documentation: Option<Markdown>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CapabilityStatement {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: DateTime,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    pub kind: Code,
    pub instantiates: Option<Vec<Canonical>>,
    pub imports: Option<Vec<Canonical>>,
    pub software: Option<CapabilityStatementSoftware>,
    pub implementation: Option<CapabilityStatementImplementation>,
    #[serde(rename = "fhirVersion")]
    pub fhir_version: Code,
    pub format: Option<Vec<Code>>,
    #[serde(rename = "patchFormat")]
    pub patch_format: Option<Vec<Code>>,
    #[serde(rename = "implementationGuide")]
    pub implementation_guide: Option<Vec<Canonical>>,
    pub rest: Option<Vec<CapabilityStatementRest>>,
    pub messaging: Option<Vec<CapabilityStatementMessaging>>,
    pub document: Option<Vec<CapabilityStatementDocument>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CapabilityStatementSoftware {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub version: Option<String>,
    #[serde(rename = "releaseDate")]
    pub release_date: Option<DateTime>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CapabilityStatementRest {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub mode: Code,
    pub documentation: Option<Markdown>,
    pub security: Option<CapabilityStatementRestSecurity>,
    pub resource: Option<Vec<CapabilityStatementRestResource>>,
    pub interaction: Option<Vec<CapabilityStatementRestInteraction>>,
    #[serde(rename = "searchParam")]
    pub search_param: Option<Vec<CapabilityStatementRestResourceSearchParam>>,
    pub operation: Option<Vec<CapabilityStatementRestResourceOperation>>,
    pub compartment: Option<Vec<Canonical>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CapabilityStatementRestInteraction {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub documentation: Option<Markdown>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CapabilityStatementMessagingEndpoint {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub protocol: Coding,
    pub address: Url,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CapabilityStatementDocument {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub mode: Code,
    pub documentation: Option<Markdown>,
    pub profile: Canonical,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CapabilityStatementRestResourceSearchParam {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub definition: Option<Canonical>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub documentation: Option<Markdown>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CapabilityStatementMessaging {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub endpoint: Option<Vec<CapabilityStatementMessagingEndpoint>>,
    #[serde(rename = "reliableCache")]
    pub reliable_cache: Option<UnsignedInt>,
    pub documentation: Option<Markdown>,
    #[serde(rename = "supportedMessage")]
    pub supported_message: Option<Vec<CapabilityStatementMessagingSupportedMessage>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CapabilityStatementRestResource {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub profile: Option<Canonical>,
    #[serde(rename = "supportedProfile")]
    pub supported_profile: Option<Vec<Canonical>>,
    pub documentation: Option<Markdown>,
    pub interaction: Option<Vec<CapabilityStatementRestResourceInteraction>>,
    pub versioning: Option<Code>,
    #[serde(rename = "readHistory")]
    pub read_history: Option<Boolean>,
    #[serde(rename = "updateCreate")]
    pub update_create: Option<Boolean>,
    #[serde(rename = "conditionalCreate")]
    pub conditional_create: Option<Boolean>,
    #[serde(rename = "conditionalRead")]
    pub conditional_read: Option<Code>,
    #[serde(rename = "conditionalUpdate")]
    pub conditional_update: Option<Boolean>,
    #[serde(rename = "conditionalDelete")]
    pub conditional_delete: Option<Code>,
    #[serde(rename = "referencePolicy")]
    pub reference_policy: Option<Vec<Code>>,
    #[serde(rename = "searchInclude")]
    pub search_include: Option<Vec<String>>,
    #[serde(rename = "searchRevInclude")]
    pub search_rev_include: Option<Vec<String>>,
    #[serde(rename = "searchParam")]
    pub search_param: Option<Vec<CapabilityStatementRestResourceSearchParam>>,
    pub operation: Option<Vec<CapabilityStatementRestResourceOperation>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct CarePlanActivity {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "outcomeCodeableConcept")]
    pub outcome_codeable_concept: Option<Vec<CodeableConcept>>,
    #[serde(rename = "outcomeReference")]
    pub outcome_reference: Option<Vec<Reference>>,
    pub progress: Option<Vec<Annotation>>,
    pub reference: Option<Reference>,
    pub detail: Option<CarePlanActivityDetail>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CarePlan {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "instantiatesCanonical")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[serde(rename = "instantiatesUri")]
    pub instantiates_uri: Option<Vec<Uri>>,
    #[serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    pub replaces: Option<Vec<Reference>>,
    #[serde(rename = "partOf")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    pub intent: Code,
    pub category: Option<Vec<CodeableConcept>>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    pub period: Option<Period>,
    pub created: Option<DateTime>,
    pub author: Option<Reference>,
    pub contributor: Option<Vec<Reference>>,
    #[serde(rename = "careTeam")]
    pub care_team: Option<Vec<Reference>>,
    pub addresses: Option<Vec<Reference>>,
    #[serde(rename = "supportingInfo")]
    pub supporting_info: Option<Vec<Reference>>,
    pub goal: Option<Vec<Reference>>,
    pub activity: Option<Vec<CarePlanActivity>>,
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CarePlanActivityDetailScheduled {
    Timing(Timing),
    Period(Period),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CarePlanActivityDetailProduct {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CarePlanActivityDetail {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub kind: Option<Code>,
    #[serde(rename = "instantiatesCanonical")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[serde(rename = "instantiatesUri")]
    pub instantiates_uri: Option<Vec<Uri>>,
    pub code: Option<CodeableConcept>,
    #[serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    pub goal: Option<Vec<Reference>>,
    pub status: Code,
    #[serde(rename = "statusReason")]
    pub status_reason: Option<CodeableConcept>,
    #[serde(rename = "doNotPerform")]
    pub do_not_perform: Option<Boolean>,
    #[serde(rename = "scheduledTiming")]
    pub scheduled_timing: Option<Timing>,
    #[serde(rename = "scheduledPeriod")]
    pub scheduled_period: Option<Period>,
    #[serde(rename = "scheduledString")]
    pub scheduled_string: Option<String>,
    pub location: Option<Reference>,
    pub performer: Option<Vec<Reference>>,
    #[serde(rename = "productCodeableConcept")]
    pub product_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "productReference")]
    pub product_reference: Option<Reference>,
    #[serde(rename = "dailyAmount")]
    pub daily_amount: Option<Quantity>,
    pub quantity: Option<Quantity>,
    pub description: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct CareTeamParticipant {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub role: Option<Vec<CodeableConcept>>,
    pub member: Option<Reference>,
    #[serde(rename = "onBehalfOf")]
    pub on_behalf_of: Option<Reference>,
    pub period: Option<Period>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CareTeam {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Option<Code>,
    pub category: Option<Vec<CodeableConcept>>,
    pub name: Option<String>,
    pub subject: Option<Reference>,
    pub encounter: Option<Reference>,
    pub period: Option<Period>,
    pub participant: Option<Vec<CareTeamParticipant>>,
    #[serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(rename = "managingOrganization")]
    pub managing_organization: Option<Vec<Reference>>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub note: Option<Vec<Annotation>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub orderable: Boolean,
    #[serde(rename = "referencedItem")]
    pub referenced_item: Reference,
    #[serde(rename = "additionalIdentifier")]
    pub additional_identifier: Option<Vec<Identifier>>,
    pub classification: Option<Vec<CodeableConcept>>,
    pub status: Option<Code>,
    #[serde(rename = "validityPeriod")]
    pub validity_period: Option<Period>,
    #[serde(rename = "validTo")]
    pub valid_to: Option<DateTime>,
    #[serde(rename = "lastUpdated")]
    pub last_updated: Option<DateTime>,
    #[serde(rename = "additionalCharacteristic")]
    pub additional_characteristic: Option<Vec<CodeableConcept>>,
    #[serde(rename = "additionalClassification")]
    pub additional_classification: Option<Vec<CodeableConcept>>,
    #[serde(rename = "relatedEntry")]
    pub related_entry: Option<Vec<CatalogEntryRelatedEntry>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CatalogEntryRelatedEntry {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub relationtype: Code,
    pub item: Reference,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChargeItemOccurrence {
    DateTime(DateTime),
    Period(Period),
    Timing(Timing),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChargeItemProduct {
    Reference(Reference),
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChargeItem {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "definitionUri")]
    pub definition_uri: Option<Vec<Uri>>,
    #[serde(rename = "definitionCanonical")]
    pub definition_canonical: Option<Vec<Canonical>>,
    pub status: Code,
    #[serde(rename = "partOf")]
    pub part_of: Option<Vec<Reference>>,
    pub code: CodeableConcept,
    pub subject: Reference,
    pub context: Option<Reference>,
    #[serde(rename = "occurrenceDateTime")]
    pub occurrence_date_time: Option<DateTime>,
    #[serde(rename = "occurrencePeriod")]
    pub occurrence_period: Option<Period>,
    #[serde(rename = "occurrenceTiming")]
    pub occurrence_timing: Option<Timing>,
    pub performer: Option<Vec<ChargeItemPerformer>>,
    #[serde(rename = "performingOrganization")]
    pub performing_organization: Option<Reference>,
    #[serde(rename = "requestingOrganization")]
    pub requesting_organization: Option<Reference>,
    #[serde(rename = "costCenter")]
    pub cost_center: Option<Reference>,
    pub quantity: Option<Quantity>,
    pub bodysite: Option<Vec<CodeableConcept>>,
    #[serde(rename = "factorOverride")]
    pub factor_override: Option<Decimal>,
    #[serde(rename = "priceOverride")]
    pub price_override: Option<Money>,
    #[serde(rename = "overrideReason")]
    pub override_reason: Option<String>,
    pub enterer: Option<Reference>,
    #[serde(rename = "enteredDate")]
    pub entered_date: Option<DateTime>,
    pub reason: Option<Vec<CodeableConcept>>,
    pub service: Option<Vec<Reference>>,
    #[serde(rename = "productReference")]
    pub product_reference: Option<Reference>,
    #[serde(rename = "productCodeableConcept")]
    pub product_codeable_concept: Option<CodeableConcept>,
    pub account: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "supportingInformation")]
    pub supporting_information: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChargeItemPerformer {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct ChargeItemDefinitionApplicability {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    pub language: Option<String>,
    pub expression: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChargeItemDefinitionPropertyGroup {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub applicability: Option<Vec<ChargeItemDefinitionApplicability>>,
    #[serde(rename = "priceComponent")]
    pub price_component: Option<Vec<ChargeItemDefinitionPropertyGroupPriceComponent>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChargeItemDefinitionPropertyGroupPriceComponent {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub code: Option<CodeableConcept>,
    pub factor: Option<Decimal>,
    pub amount: Option<Money>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChargeItemDefinition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Uri,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub title: Option<String>,
    #[serde(rename = "derivedFromUri")]
    pub derived_from_uri: Option<Vec<Uri>>,
    #[serde(rename = "partOf")]
    pub part_of: Option<Vec<Canonical>>,
    pub replaces: Option<Vec<Canonical>>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod")]
    pub effective_period: Option<Period>,
    pub code: Option<CodeableConcept>,
    pub instance: Option<Vec<Reference>>,
    pub applicability: Option<Vec<ChargeItemDefinitionApplicability>>,
    #[serde(rename = "propertyGroup")]
    pub property_group: Option<Vec<ChargeItemDefinitionPropertyGroup>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct CitationStatusDate {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub activity: CodeableConcept,
    pub actual: Option<Boolean>,
    pub period: Period,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CitationCitedArtifactRelatesToTarget {
    Uri(Uri),
    Identifier(Identifier),
    Reference(Reference),
    Attachment(Attachment),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CitationCitedArtifactRelatesTo {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "relationshipType")]
    pub relationship_type: CodeableConcept,
    #[serde(rename = "targetClassifier")]
    pub target_classifier: Option<Vec<CodeableConcept>>,
    #[serde(rename = "targetUri")]
    pub target_uri: Uri,
    #[serde(rename = "targetIdentifier")]
    pub target_identifier: Identifier,
    #[serde(rename = "targetReference")]
    pub target_reference: Reference,
    #[serde(rename = "targetAttachment")]
    pub target_attachment: Attachment,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CitationCitedArtifactContributorshipEntry {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Option<HumanName>,
    pub initials: Option<String>,
    #[serde(rename = "collectiveName")]
    pub collective_name: Option<String>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "affiliationInfo")]
    pub affiliation_info: Option<Vec<CitationCitedArtifactContributorshipEntryAffiliationInfo>>,
    pub address: Option<Vec<Address>>,
    pub telecom: Option<Vec<ContactPoint>>,
    #[serde(rename = "contributionType")]
    pub contribution_type: Option<Vec<CodeableConcept>>,
    pub role: Option<CodeableConcept>,
    #[serde(rename = "contributionInstance")]
    pub contribution_instance: Option<Vec<CitationCitedArtifactContributorshipEntryContributionInstance>>,
    #[serde(rename = "correspondingContact")]
    pub corresponding_contact: Option<Boolean>,
    #[serde(rename = "listOrder")]
    pub list_order: Option<PositiveInt>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CitationCitedArtifactVersion {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub value: String,
    #[serde(rename = "baseCitation")]
    pub base_citation: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CitationCitedArtifactWebLocation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub url: Option<Uri>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CitationCitedArtifactClassification {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub classifier: Option<Vec<CodeableConcept>>,
    #[serde(rename = "whoClassified")]
    pub who_classified: Option<CitationCitedArtifactClassificationWhoClassified>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CitationCitedArtifactPublicationFormPeriodicReleaseDateOfPublication {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub date: Option<Date>,
    pub year: Option<String>,
    pub month: Option<String>,
    pub day: Option<String>,
    pub season: Option<String>,
    pub text: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CitationCitedArtifact {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "relatedIdentifier")]
    pub related_identifier: Option<Vec<Identifier>>,
    #[serde(rename = "dateAccessed")]
    pub date_accessed: Option<DateTime>,
    pub version: Option<CitationCitedArtifactVersion>,
    #[serde(rename = "currentState")]
    pub current_state: Option<Vec<CodeableConcept>>,
    #[serde(rename = "statusDate")]
    pub status_date: Option<Vec<CitationCitedArtifactStatusDate>>,
    pub title: Option<Vec<CitationCitedArtifactTitle>>,
    #[serde(rename = "abstract")]
    pub r#abstract: Option<Vec<CitationCitedArtifactAbstract>>,
    pub part: Option<CitationCitedArtifactPart>,
    #[serde(rename = "relatesTo")]
    pub relates_to: Option<Vec<CitationCitedArtifactRelatesTo>>,
    #[serde(rename = "publicationForm")]
    pub publication_form: Option<Vec<CitationCitedArtifactPublicationForm>>,
    #[serde(rename = "webLocation")]
    pub web_location: Option<Vec<CitationCitedArtifactWebLocation>>,
    pub classification: Option<Vec<CitationCitedArtifactClassification>>,
    pub contributorship: Option<CitationCitedArtifactContributorship>,
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CitationCitedArtifactPublicationFormPublishedIn {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub identifier: Option<Vec<Identifier>>,
    pub title: Option<String>,
    pub publisher: Option<Reference>,
    #[serde(rename = "publisherLocation")]
    pub publisher_location: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CitationClassification {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub classifier: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CitationCitedArtifactPart {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub value: Option<String>,
    #[serde(rename = "baseCitation")]
    pub base_citation: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CitationCitedArtifactContributorshipEntryAffiliationInfo {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub affiliation: Option<String>,
    pub role: Option<String>,
    pub identifier: Option<Vec<Identifier>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CitationSummary {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub style: Option<CodeableConcept>,
    pub text: Markdown,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CitationRelatesToTarget {
    Uri(Uri),
    Identifier(Identifier),
    Reference(Reference),
    Attachment(Attachment),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CitationRelatesTo {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "relationshipType")]
    pub relationship_type: CodeableConcept,
    #[serde(rename = "targetClassifier")]
    pub target_classifier: Option<Vec<CodeableConcept>>,
    #[serde(rename = "targetUri")]
    pub target_uri: Uri,
    #[serde(rename = "targetIdentifier")]
    pub target_identifier: Identifier,
    #[serde(rename = "targetReference")]
    pub target_reference: Reference,
    #[serde(rename = "targetAttachment")]
    pub target_attachment: Attachment,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Citation {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod")]
    pub effective_period: Option<Period>,
    pub author: Option<Vec<ContactDetail>>,
    pub editor: Option<Vec<ContactDetail>>,
    pub reviewer: Option<Vec<ContactDetail>>,
    pub endorser: Option<Vec<ContactDetail>>,
    pub summary: Option<Vec<CitationSummary>>,
    pub classification: Option<Vec<CitationClassification>>,
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "currentState")]
    pub current_state: Option<Vec<CodeableConcept>>,
    #[serde(rename = "statusDate")]
    pub status_date: Option<Vec<CitationStatusDate>>,
    #[serde(rename = "relatesTo")]
    pub relates_to: Option<Vec<CitationRelatesTo>>,
    #[serde(rename = "citedArtifact")]
    pub cited_artifact: Option<CitationCitedArtifact>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CitationCitedArtifactPublicationFormPeriodicRelease {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "citedMedium")]
    pub cited_medium: Option<CodeableConcept>,
    pub volume: Option<String>,
    pub issue: Option<String>,
    #[serde(rename = "dateOfPublication")]
    pub date_of_publication: Option<CitationCitedArtifactPublicationFormPeriodicReleaseDateOfPublication>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CitationCitedArtifactContributorship {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub complete: Option<Boolean>,
    pub entry: Option<Vec<CitationCitedArtifactContributorshipEntry>>,
    pub summary: Option<Vec<CitationCitedArtifactContributorshipSummary>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CitationCitedArtifactTitle {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    pub language: Option<CodeableConcept>,
    pub text: Markdown,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CitationCitedArtifactContributorshipSummary {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub style: Option<CodeableConcept>,
    pub source: Option<CodeableConcept>,
    pub value: Markdown,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CitationCitedArtifactClassificationWhoClassified {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub person: Option<Reference>,
    pub organization: Option<Reference>,
    pub publisher: Option<Reference>,
    #[serde(rename = "classifierCopyright")]
    pub classifier_copyright: Option<String>,
    #[serde(rename = "freeToShare")]
    pub free_to_share: Option<Boolean>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CitationCitedArtifactPublicationForm {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "publishedIn")]
    pub published_in: Option<CitationCitedArtifactPublicationFormPublishedIn>,
    #[serde(rename = "periodicRelease")]
    pub periodic_release: Option<CitationCitedArtifactPublicationFormPeriodicRelease>,
    #[serde(rename = "articleDate")]
    pub article_date: Option<DateTime>,
    #[serde(rename = "lastRevisionDate")]
    pub last_revision_date: Option<DateTime>,
    pub language: Option<Vec<CodeableConcept>>,
    #[serde(rename = "accessionNumber")]
    pub accession_number: Option<String>,
    #[serde(rename = "pageString")]
    pub page_string: Option<String>,
    #[serde(rename = "firstPage")]
    pub first_page: Option<String>,
    #[serde(rename = "lastPage")]
    pub last_page: Option<String>,
    #[serde(rename = "pageCount")]
    pub page_count: Option<String>,
    pub copyright: Option<Markdown>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CitationCitedArtifactAbstract {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub language: Option<CodeableConcept>,
    pub text: Markdown,
    pub copyright: Option<Markdown>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CitationCitedArtifactContributorshipEntryContributionInstance {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub time: Option<DateTime>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CitationCitedArtifactStatusDate {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub activity: CodeableConcept,
    pub actual: Option<Boolean>,
    pub period: Period,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct ClaimCareTeam {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub provider: Reference,
    pub responsible: Option<Boolean>,
    pub role: Option<CodeableConcept>,
    pub qualification: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Claim {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(rename = "subType")]
    pub sub_type: Option<CodeableConcept>,
    #[serde(rename = "use")]
    pub r#use: Code,
    pub patient: Reference,
    #[serde(rename = "billablePeriod")]
    pub billable_period: Option<Period>,
    pub created: DateTime,
    pub enterer: Option<Reference>,
    pub insurer: Option<Reference>,
    pub provider: Reference,
    pub priority: CodeableConcept,
    #[serde(rename = "fundsReserve")]
    pub funds_reserve: Option<CodeableConcept>,
    pub related: Option<Vec<ClaimRelated>>,
    pub prescription: Option<Reference>,
    #[serde(rename = "originalPrescription")]
    pub original_prescription: Option<Reference>,
    pub payee: Option<ClaimPayee>,
    pub referral: Option<Reference>,
    pub facility: Option<Reference>,
    #[serde(rename = "careTeam")]
    pub care_team: Option<Vec<ClaimCareTeam>>,
    #[serde(rename = "supportingInfo")]
    pub supporting_info: Option<Vec<ClaimSupportingInfo>>,
    pub diagnosis: Option<Vec<ClaimDiagnosis>>,
    pub procedure: Option<Vec<ClaimProcedure>>,
    pub insurance: Option<Vec<ClaimInsurance>>,
    pub accident: Option<ClaimAccident>,
    pub item: Option<Vec<ClaimItem>>,
    pub total: Option<Money>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClaimRelated {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub claim: Option<Reference>,
    pub relationship: Option<CodeableConcept>,
    pub reference: Option<Identifier>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClaimAccidentLocation {
    Address(Address),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClaimAccident {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub date: Date,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "locationAddress")]
    pub location_address: Option<Address>,
    #[serde(rename = "locationReference")]
    pub location_reference: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClaimItemDetailSubDetail {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub revenue: Option<CodeableConcept>,
    pub category: Option<CodeableConcept>,
    #[serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    pub modifier: Option<Vec<CodeableConcept>>,
    #[serde(rename = "programCode")]
    pub program_code: Option<Vec<CodeableConcept>>,
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub net: Option<Money>,
    pub udi: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClaimInsurance {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub focal: Boolean,
    pub identifier: Option<Identifier>,
    pub coverage: Reference,
    #[serde(rename = "businessArrangement")]
    pub business_arrangement: Option<String>,
    #[serde(rename = "preAuthRef")]
    pub pre_auth_ref: Option<Vec<String>>,
    #[serde(rename = "claimResponse")]
    pub claim_response: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClaimItemServiced {
    Date(Date),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClaimItemLocation {
    CodeableConcept(CodeableConcept),
    Address(Address),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClaimItem {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    #[serde(rename = "careTeamSequence")]
    pub care_team_sequence: Option<Vec<PositiveInt>>,
    #[serde(rename = "diagnosisSequence")]
    pub diagnosis_sequence: Option<Vec<PositiveInt>>,
    #[serde(rename = "procedureSequence")]
    pub procedure_sequence: Option<Vec<PositiveInt>>,
    #[serde(rename = "informationSequence")]
    pub information_sequence: Option<Vec<PositiveInt>>,
    pub revenue: Option<CodeableConcept>,
    pub category: Option<CodeableConcept>,
    #[serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    pub modifier: Option<Vec<CodeableConcept>>,
    #[serde(rename = "programCode")]
    pub program_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "servicedDate")]
    pub serviced_date: Option<Date>,
    #[serde(rename = "servicedPeriod")]
    pub serviced_period: Option<Period>,
    #[serde(rename = "locationCodeableConcept")]
    pub location_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "locationAddress")]
    pub location_address: Option<Address>,
    #[serde(rename = "locationReference")]
    pub location_reference: Option<Reference>,
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub net: Option<Money>,
    pub udi: Option<Vec<Reference>>,
    #[serde(rename = "bodySite")]
    pub body_site: Option<CodeableConcept>,
    #[serde(rename = "subSite")]
    pub sub_site: Option<Vec<CodeableConcept>>,
    pub encounter: Option<Vec<Reference>>,
    pub detail: Option<Vec<ClaimItemDetail>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClaimSupportingInfoTiming {
    Date(Date),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClaimSupportingInfoValue {
    Boolean(Boolean),
    String(String),
    Quantity(Quantity),
    Attachment(Attachment),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClaimSupportingInfo {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub category: CodeableConcept,
    pub code: Option<CodeableConcept>,
    #[serde(rename = "timingDate")]
    pub timing_date: Option<Date>,
    #[serde(rename = "timingPeriod")]
    pub timing_period: Option<Period>,
    #[serde(rename = "valueBoolean")]
    pub value_boolean: Option<Boolean>,
    #[serde(rename = "valueString")]
    pub value_string: Option<String>,
    #[serde(rename = "valueQuantity")]
    pub value_quantity: Option<Quantity>,
    #[serde(rename = "valueAttachment")]
    pub value_attachment: Option<Attachment>,
    #[serde(rename = "valueReference")]
    pub value_reference: Option<Reference>,
    pub reason: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClaimItemDetail {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub revenue: Option<CodeableConcept>,
    pub category: Option<CodeableConcept>,
    #[serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    pub modifier: Option<Vec<CodeableConcept>>,
    #[serde(rename = "programCode")]
    pub program_code: Option<Vec<CodeableConcept>>,
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub net: Option<Money>,
    pub udi: Option<Vec<Reference>>,
    #[serde(rename = "subDetail")]
    pub sub_detail: Option<Vec<ClaimItemDetailSubDetail>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClaimPayee {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub party: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClaimDiagnosisDiagnosis {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClaimDiagnosis {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    #[serde(rename = "diagnosisCodeableConcept")]
    pub diagnosis_codeable_concept: CodeableConcept,
    #[serde(rename = "diagnosisReference")]
    pub diagnosis_reference: Reference,
    #[serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(rename = "onAdmission")]
    pub on_admission: Option<CodeableConcept>,
    #[serde(rename = "packageCode")]
    pub package_code: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClaimProcedureProcedure {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClaimProcedure {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    #[serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    pub date: Option<DateTime>,
    #[serde(rename = "procedureCodeableConcept")]
    pub procedure_codeable_concept: CodeableConcept,
    #[serde(rename = "procedureReference")]
    pub procedure_reference: Reference,
    pub udi: Option<Vec<Reference>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct ClaimResponsePayment {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub adjustment: Option<Money>,
    #[serde(rename = "adjustmentReason")]
    pub adjustment_reason: Option<CodeableConcept>,
    pub date: Option<Date>,
    pub amount: Money,
    pub identifier: Option<Identifier>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClaimResponseTotal {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: CodeableConcept,
    pub amount: Money,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClaimResponseItem {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "itemSequence")]
    pub item_sequence: PositiveInt,
    #[serde(rename = "noteNumber")]
    pub note_number: Option<Vec<PositiveInt>>,
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
    pub detail: Option<Vec<ClaimResponseItemDetail>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClaimResponseProcessNote {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub number: Option<PositiveInt>,
    #[serde(rename = "type")]
    pub r#type: Option<Code>,
    pub text: String,
    pub language: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClaimResponseError {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "itemSequence")]
    pub item_sequence: Option<PositiveInt>,
    #[serde(rename = "detailSequence")]
    pub detail_sequence: Option<PositiveInt>,
    #[serde(rename = "subDetailSequence")]
    pub sub_detail_sequence: Option<PositiveInt>,
    pub code: CodeableConcept,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClaimResponseAddItemServiced {
    Date(Date),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClaimResponseAddItemLocation {
    CodeableConcept(CodeableConcept),
    Address(Address),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClaimResponseAddItem {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "itemSequence")]
    pub item_sequence: Option<Vec<PositiveInt>>,
    #[serde(rename = "detailSequence")]
    pub detail_sequence: Option<Vec<PositiveInt>>,
    #[serde(rename = "subdetailSequence")]
    pub subdetail_sequence: Option<Vec<PositiveInt>>,
    pub provider: Option<Vec<Reference>>,
    #[serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    pub modifier: Option<Vec<CodeableConcept>>,
    #[serde(rename = "programCode")]
    pub program_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "servicedDate")]
    pub serviced_date: Option<Date>,
    #[serde(rename = "servicedPeriod")]
    pub serviced_period: Option<Period>,
    #[serde(rename = "locationCodeableConcept")]
    pub location_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "locationAddress")]
    pub location_address: Option<Address>,
    #[serde(rename = "locationReference")]
    pub location_reference: Option<Reference>,
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub net: Option<Money>,
    #[serde(rename = "bodySite")]
    pub body_site: Option<CodeableConcept>,
    #[serde(rename = "subSite")]
    pub sub_site: Option<Vec<CodeableConcept>>,
    #[serde(rename = "noteNumber")]
    pub note_number: Option<Vec<PositiveInt>>,
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
    pub detail: Option<Vec<ClaimResponseAddItemDetail>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClaimResponse {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(rename = "subType")]
    pub sub_type: Option<CodeableConcept>,
    #[serde(rename = "use")]
    pub r#use: Code,
    pub patient: Reference,
    pub created: DateTime,
    pub insurer: Reference,
    pub requestor: Option<Reference>,
    pub request: Option<Reference>,
    pub outcome: Code,
    pub disposition: Option<String>,
    #[serde(rename = "preAuthRef")]
    pub pre_auth_ref: Option<String>,
    #[serde(rename = "preAuthPeriod")]
    pub pre_auth_period: Option<Period>,
    #[serde(rename = "payeeType")]
    pub payee_type: Option<CodeableConcept>,
    pub item: Option<Vec<ClaimResponseItem>>,
    #[serde(rename = "addItem")]
    pub add_item: Option<Vec<ClaimResponseAddItem>>,
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
    pub total: Option<Vec<ClaimResponseTotal>>,
    pub payment: Option<ClaimResponsePayment>,
    #[serde(rename = "fundsReserve")]
    pub funds_reserve: Option<CodeableConcept>,
    #[serde(rename = "formCode")]
    pub form_code: Option<CodeableConcept>,
    pub form: Option<Attachment>,
    #[serde(rename = "processNote")]
    pub process_note: Option<Vec<ClaimResponseProcessNote>>,
    #[serde(rename = "communicationRequest")]
    pub communication_request: Option<Vec<Reference>>,
    pub insurance: Option<Vec<ClaimResponseInsurance>>,
    pub error: Option<Vec<ClaimResponseError>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClaimResponseAddItemDetailSubDetail {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    pub modifier: Option<Vec<CodeableConcept>>,
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub net: Option<Money>,
    #[serde(rename = "noteNumber")]
    pub note_number: Option<Vec<PositiveInt>>,
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClaimResponseInsurance {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub focal: Boolean,
    pub coverage: Reference,
    #[serde(rename = "businessArrangement")]
    pub business_arrangement: Option<String>,
    #[serde(rename = "claimResponse")]
    pub claim_response: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClaimResponseItemAdjudication {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: CodeableConcept,
    pub reason: Option<CodeableConcept>,
    pub amount: Option<Money>,
    pub value: Option<Decimal>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClaimResponseAddItemDetail {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    pub modifier: Option<Vec<CodeableConcept>>,
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub net: Option<Money>,
    #[serde(rename = "noteNumber")]
    pub note_number: Option<Vec<PositiveInt>>,
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
    #[serde(rename = "subDetail")]
    pub sub_detail: Option<Vec<ClaimResponseAddItemDetailSubDetail>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClaimResponseItemDetailSubDetail {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "subDetailSequence")]
    pub sub_detail_sequence: PositiveInt,
    #[serde(rename = "noteNumber")]
    pub note_number: Option<Vec<PositiveInt>>,
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClaimResponseItemDetail {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "detailSequence")]
    pub detail_sequence: PositiveInt,
    #[serde(rename = "noteNumber")]
    pub note_number: Option<Vec<PositiveInt>>,
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
    #[serde(rename = "subDetail")]
    pub sub_detail: Option<Vec<ClaimResponseItemDetailSubDetail>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClinicalImpressionEffective {
    DateTime(DateTime),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClinicalImpression {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "statusReason")]
    pub status_reason: Option<CodeableConcept>,
    pub code: Option<CodeableConcept>,
    pub description: Option<String>,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    #[serde(rename = "effectiveDateTime")]
    pub effective_date_time: Option<DateTime>,
    #[serde(rename = "effectivePeriod")]
    pub effective_period: Option<Period>,
    pub date: Option<DateTime>,
    pub assessor: Option<Reference>,
    pub previous: Option<Reference>,
    pub problem: Option<Vec<Reference>>,
    pub investigation: Option<Vec<ClinicalImpressionInvestigation>>,
    pub protocol: Option<Vec<Uri>>,
    pub summary: Option<String>,
    pub finding: Option<Vec<ClinicalImpressionFinding>>,
    #[serde(rename = "prognosisCodeableConcept")]
    pub prognosis_codeable_concept: Option<Vec<CodeableConcept>>,
    #[serde(rename = "prognosisReference")]
    pub prognosis_reference: Option<Vec<Reference>>,
    #[serde(rename = "supportingInfo")]
    pub supporting_info: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClinicalImpressionInvestigation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    pub item: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClinicalImpressionFinding {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "itemCodeableConcept")]
    pub item_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "itemReference")]
    pub item_reference: Option<Reference>,
    pub basis: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct ClinicalUseDefinitionContraindication {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "diseaseSymptomProcedure")]
    pub disease_symptom_procedure: Option<CodeableReference>,
    #[serde(rename = "diseaseStatus")]
    pub disease_status: Option<CodeableReference>,
    pub comorbidity: Option<Vec<CodeableReference>>,
    pub indication: Option<Vec<Reference>>,
    #[serde(rename = "otherTherapy")]
    pub other_therapy: Option<Vec<ClinicalUseDefinitionContraindicationOtherTherapy>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClinicalUseDefinitionInteraction {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub interactant: Option<Vec<ClinicalUseDefinitionInteractionInteractant>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub effect: Option<CodeableReference>,
    pub incidence: Option<CodeableConcept>,
    pub management: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClinicalUseDefinitionWarning {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<Markdown>,
    pub code: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClinicalUseDefinitionUndesirableEffect {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "symptomConditionEffect")]
    pub symptom_condition_effect: Option<CodeableReference>,
    pub classification: Option<CodeableConcept>,
    #[serde(rename = "frequencyOfOccurrence")]
    pub frequency_of_occurrence: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClinicalUseDefinitionContraindicationOtherTherapy {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "relationshipType")]
    pub relationship_type: CodeableConcept,
    pub therapy: CodeableReference,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClinicalUseDefinitionIndicationDuration {
    Range(Range),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClinicalUseDefinitionIndication {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "diseaseSymptomProcedure")]
    pub disease_symptom_procedure: Option<CodeableReference>,
    #[serde(rename = "diseaseStatus")]
    pub disease_status: Option<CodeableReference>,
    pub comorbidity: Option<Vec<CodeableReference>>,
    #[serde(rename = "intendedEffect")]
    pub intended_effect: Option<CodeableReference>,
    #[serde(rename = "durationRange")]
    pub duration_range: Option<Range>,
    #[serde(rename = "durationString")]
    pub duration_string: Option<String>,
    #[serde(rename = "undesirableEffect")]
    pub undesirable_effect: Option<Vec<Reference>>,
    #[serde(rename = "otherTherapy")]
    pub other_therapy: Option<Vec<ClinicalUseDefinitionContraindicationOtherTherapy>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClinicalUseDefinition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub category: Option<Vec<CodeableConcept>>,
    pub subject: Option<Vec<Reference>>,
    pub status: Option<CodeableConcept>,
    pub contraindication: Option<ClinicalUseDefinitionContraindication>,
    pub indication: Option<ClinicalUseDefinitionIndication>,
    pub interaction: Option<ClinicalUseDefinitionInteraction>,
    pub population: Option<Vec<Reference>>,
    #[serde(rename = "undesirableEffect")]
    pub undesirable_effect: Option<ClinicalUseDefinitionUndesirableEffect>,
    pub warning: Option<ClinicalUseDefinitionWarning>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClinicalUseDefinitionInteractionInteractantItem {
    Reference(Reference),
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClinicalUseDefinitionInteractionInteractant {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "itemReference")]
    pub item_reference: Reference,
    #[serde(rename = "itemCodeableConcept")]
    pub item_codeable_concept: CodeableConcept,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct CodeSystem {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    #[serde(rename = "caseSensitive")]
    pub case_sensitive: Option<Boolean>,
    #[serde(rename = "valueSet")]
    pub value_set: Option<Canonical>,
    #[serde(rename = "hierarchyMeaning")]
    pub hierarchy_meaning: Option<Code>,
    pub compositional: Option<Boolean>,
    #[serde(rename = "versionNeeded")]
    pub version_needed: Option<Boolean>,
    pub content: Code,
    pub supplements: Option<Canonical>,
    pub count: Option<UnsignedInt>,
    pub filter: Option<Vec<CodeSystemFilter>>,
    pub property: Option<Vec<CodeSystemProperty>>,
    pub concept: Option<Vec<CodeSystemConcept>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CodeSystemProperty {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub uri: Option<Uri>,
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub r#type: Code,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CodeSystemConceptPropertyValue {
    Code(Code),
    Coding(Coding),
    String(String),
    Integer(Integer),
    Boolean(Boolean),
    DateTime(DateTime),
    Decimal(Decimal),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CodeSystemConceptProperty {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    #[serde(rename = "valueCode")]
    pub value_code: Code,
    #[serde(rename = "valueCoding")]
    pub value_coding: Coding,
    #[serde(rename = "valueString")]
    pub value_string: String,
    #[serde(rename = "valueInteger")]
    pub value_integer: Integer,
    #[serde(rename = "valueBoolean")]
    pub value_boolean: Boolean,
    #[serde(rename = "valueDateTime")]
    pub value_date_time: DateTime,
    #[serde(rename = "valueDecimal")]
    pub value_decimal: Decimal,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CodeSystemConcept {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub display: Option<String>,
    pub definition: Option<String>,
    pub designation: Option<Vec<CodeSystemConceptDesignation>>,
    pub property: Option<Vec<CodeSystemConceptProperty>>,
    pub concept: Option<Vec<CodeSystemConcept>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CodeSystemFilter {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub description: Option<String>,
    pub operator: Option<Vec<Code>>,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CodeSystemConceptDesignation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub language: Option<Code>,
    #[serde(rename = "use")]
    pub r#use: Option<Coding>,
    pub value: String,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct Communication {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "instantiatesCanonical")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[serde(rename = "instantiatesUri")]
    pub instantiates_uri: Option<Vec<Uri>>,
    #[serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(rename = "partOf")]
    pub part_of: Option<Vec<Reference>>,
    #[serde(rename = "inResponseTo")]
    pub in_response_to: Option<Vec<Reference>>,
    pub status: Code,
    #[serde(rename = "statusReason")]
    pub status_reason: Option<CodeableConcept>,
    pub category: Option<Vec<CodeableConcept>>,
    pub priority: Option<Code>,
    pub medium: Option<Vec<CodeableConcept>>,
    pub subject: Option<Reference>,
    pub topic: Option<CodeableConcept>,
    pub about: Option<Vec<Reference>>,
    pub encounter: Option<Reference>,
    pub sent: Option<DateTime>,
    pub received: Option<DateTime>,
    pub recipient: Option<Vec<Reference>>,
    pub sender: Option<Reference>,
    #[serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    pub payload: Option<Vec<CommunicationPayload>>,
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CommunicationPayloadContent {
    String(String),
    Attachment(Attachment),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CommunicationPayload {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "contentString")]
    pub content_string: String,
    #[serde(rename = "contentAttachment")]
    pub content_attachment: Attachment,
    #[serde(rename = "contentReference")]
    pub content_reference: Reference,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CommunicationRequestPayloadContent {
    String(String),
    Attachment(Attachment),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CommunicationRequestPayload {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "contentString")]
    pub content_string: String,
    #[serde(rename = "contentAttachment")]
    pub content_attachment: Attachment,
    #[serde(rename = "contentReference")]
    pub content_reference: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CommunicationRequestOccurrence {
    DateTime(DateTime),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CommunicationRequest {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    pub replaces: Option<Vec<Reference>>,
    #[serde(rename = "groupIdentifier")]
    pub group_identifier: Option<Identifier>,
    pub status: Code,
    #[serde(rename = "statusReason")]
    pub status_reason: Option<CodeableConcept>,
    pub category: Option<Vec<CodeableConcept>>,
    pub priority: Option<Code>,
    #[serde(rename = "doNotPerform")]
    pub do_not_perform: Option<Boolean>,
    pub medium: Option<Vec<CodeableConcept>>,
    pub subject: Option<Reference>,
    pub about: Option<Vec<Reference>>,
    pub encounter: Option<Reference>,
    pub payload: Option<Vec<CommunicationRequestPayload>>,
    #[serde(rename = "occurrenceDateTime")]
    pub occurrence_date_time: Option<DateTime>,
    #[serde(rename = "occurrencePeriod")]
    pub occurrence_period: Option<Period>,
    #[serde(rename = "authoredOn")]
    pub authored_on: Option<DateTime>,
    pub requester: Option<Reference>,
    pub recipient: Option<Vec<Reference>>,
    pub sender: Option<Reference>,
    #[serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct CompartmentDefinitionResource {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub param: Option<Vec<String>>,
    pub documentation: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CompartmentDefinition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Uri,
    pub version: Option<String>,
    pub name: String,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub purpose: Option<Markdown>,
    pub code: Code,
    pub search: Boolean,
    pub resource: Option<Vec<CompartmentDefinitionResource>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CompositionRelatesToTarget {
    Identifier(Identifier),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CompositionRelatesTo {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    #[serde(rename = "targetIdentifier")]
    pub target_identifier: Identifier,
    #[serde(rename = "targetReference")]
    pub target_reference: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CompositionAttester {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub mode: Code,
    pub time: Option<DateTime>,
    pub party: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CompositionEvent {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<Vec<CodeableConcept>>,
    pub period: Option<Period>,
    pub detail: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CompositionSection {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub title: Option<String>,
    pub code: Option<CodeableConcept>,
    pub author: Option<Vec<Reference>>,
    pub focus: Option<Reference>,
    pub text: Option<Narrative>,
    pub mode: Option<Code>,
    #[serde(rename = "orderedBy")]
    pub ordered_by: Option<CodeableConcept>,
    pub entry: Option<Vec<Reference>>,
    #[serde(rename = "emptyReason")]
    pub empty_reason: Option<CodeableConcept>,
    pub section: Option<Vec<CompositionSection>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Composition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    pub status: Code,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub category: Option<Vec<CodeableConcept>>,
    pub subject: Option<Reference>,
    pub encounter: Option<Reference>,
    pub date: DateTime,
    pub author: Option<Vec<Reference>>,
    pub title: String,
    pub confidentiality: Option<Code>,
    pub attester: Option<Vec<CompositionAttester>>,
    pub custodian: Option<Reference>,
    #[serde(rename = "relatesTo")]
    pub relates_to: Option<Vec<CompositionRelatesTo>>,
    pub event: Option<Vec<CompositionEvent>>,
    pub section: Option<Vec<CompositionSection>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct ConceptMapGroupElementTarget {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<Code>,
    pub display: Option<String>,
    pub equivalence: Code,
    pub comment: Option<String>,
    #[serde(rename = "dependsOn")]
    pub depends_on: Option<Vec<ConceptMapGroupElementTargetDependsOn>>,
    pub product: Option<Vec<ConceptMapGroupElementTargetDependsOn>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConceptMapGroupUnmapped {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub mode: Code,
    pub code: Option<Code>,
    pub display: Option<String>,
    pub url: Option<Canonical>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConceptMapGroup {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub source: Option<Uri>,
    #[serde(rename = "sourceVersion")]
    pub source_version: Option<String>,
    pub target: Option<Uri>,
    #[serde(rename = "targetVersion")]
    pub target_version: Option<String>,
    pub element: Option<Vec<ConceptMapGroupElement>>,
    pub unmapped: Option<ConceptMapGroupUnmapped>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConceptMapGroupElementTargetDependsOn {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub property: Uri,
    pub system: Option<Canonical>,
    pub value: String,
    pub display: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConceptMapSource {
    Uri(Uri),
    Canonical(Canonical),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConceptMapTarget {
    Uri(Uri),
    Canonical(Canonical),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConceptMap {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Identifier>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    #[serde(rename = "sourceUri")]
    pub source_uri: Option<Uri>,
    #[serde(rename = "sourceCanonical")]
    pub source_canonical: Option<Canonical>,
    #[serde(rename = "targetUri")]
    pub target_uri: Option<Uri>,
    #[serde(rename = "targetCanonical")]
    pub target_canonical: Option<Canonical>,
    pub group: Option<Vec<ConceptMapGroup>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConceptMapGroupElement {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<Code>,
    pub display: Option<String>,
    pub target: Option<Vec<ConceptMapGroupElementTarget>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConditionOnset {
    DateTime(DateTime),
    Age(Age),
    Period(Period),
    Range(Range),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConditionAbatement {
    DateTime(DateTime),
    Age(Age),
    Period(Period),
    Range(Range),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Condition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "clinicalStatus")]
    pub clinical_status: Option<CodeableConcept>,
    #[serde(rename = "verificationStatus")]
    pub verification_status: Option<CodeableConcept>,
    pub category: Option<Vec<CodeableConcept>>,
    pub severity: Option<CodeableConcept>,
    pub code: Option<CodeableConcept>,
    #[serde(rename = "bodySite")]
    pub body_site: Option<Vec<CodeableConcept>>,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    #[serde(rename = "onsetDateTime")]
    pub onset_date_time: Option<DateTime>,
    #[serde(rename = "onsetAge")]
    pub onset_age: Option<Age>,
    #[serde(rename = "onsetPeriod")]
    pub onset_period: Option<Period>,
    #[serde(rename = "onsetRange")]
    pub onset_range: Option<Range>,
    #[serde(rename = "onsetString")]
    pub onset_string: Option<String>,
    #[serde(rename = "abatementDateTime")]
    pub abatement_date_time: Option<DateTime>,
    #[serde(rename = "abatementAge")]
    pub abatement_age: Option<Age>,
    #[serde(rename = "abatementPeriod")]
    pub abatement_period: Option<Period>,
    #[serde(rename = "abatementRange")]
    pub abatement_range: Option<Range>,
    #[serde(rename = "abatementString")]
    pub abatement_string: Option<String>,
    #[serde(rename = "recordedDate")]
    pub recorded_date: Option<DateTime>,
    pub recorder: Option<Reference>,
    pub asserter: Option<Reference>,
    pub stage: Option<Vec<ConditionStage>>,
    pub evidence: Option<Vec<ConditionEvidence>>,
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConditionEvidence {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<Vec<CodeableConcept>>,
    pub detail: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConditionStage {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub summary: Option<CodeableConcept>,
    pub assessment: Option<Vec<Reference>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConsentSource {
    Attachment(Attachment),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Consent {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub scope: CodeableConcept,
    pub category: Option<Vec<CodeableConcept>>,
    pub patient: Option<Reference>,
    #[serde(rename = "dateTime")]
    pub date_time: Option<DateTime>,
    pub performer: Option<Vec<Reference>>,
    pub organization: Option<Vec<Reference>>,
    #[serde(rename = "sourceAttachment")]
    pub source_attachment: Option<Attachment>,
    #[serde(rename = "sourceReference")]
    pub source_reference: Option<Reference>,
    pub policy: Option<Vec<ConsentPolicy>>,
    #[serde(rename = "policyRule")]
    pub policy_rule: Option<CodeableConcept>,
    pub verification: Option<Vec<ConsentVerification>>,
    pub provision: Option<ConsentProvision>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConsentProvision {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Option<Code>,
    pub period: Option<Period>,
    pub actor: Option<Vec<ConsentProvisionActor>>,
    pub action: Option<Vec<CodeableConcept>>,
    #[serde(rename = "securityLabel")]
    pub security_label: Option<Vec<Coding>>,
    pub purpose: Option<Vec<Coding>>,
    pub class: Option<Vec<Coding>>,
    pub code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "dataPeriod")]
    pub data_period: Option<Period>,
    pub data: Option<Vec<ConsentProvisionData>>,
    pub provision: Option<Vec<ConsentProvision>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConsentProvisionActor {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub role: CodeableConcept,
    pub reference: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConsentVerification {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub verified: Boolean,
    #[serde(rename = "verifiedWith")]
    pub verified_with: Option<Reference>,
    #[serde(rename = "verificationDate")]
    pub verification_date: Option<DateTime>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConsentProvisionData {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub meaning: Code,
    pub reference: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConsentPolicy {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub authority: Option<Uri>,
    pub uri: Option<Uri>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct ContractSigner {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Coding,
    pub party: Reference,
    pub signature: Option<Vec<Signature>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContractTermOffer {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub party: Option<Vec<ContractTermOfferParty>>,
    pub topic: Option<Reference>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub decision: Option<CodeableConcept>,
    #[serde(rename = "decisionMode")]
    pub decision_mode: Option<Vec<CodeableConcept>>,
    pub answer: Option<Vec<ContractTermOfferAnswer>>,
    pub text: Option<String>,
    #[serde(rename = "linkId")]
    pub link_id: Option<Vec<String>>,
    #[serde(rename = "securityLabelNumber")]
    pub security_label_number: Option<Vec<UnsignedInt>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContractTermAssetValuedItemEntity {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContractTermAssetValuedItem {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "entityCodeableConcept")]
    pub entity_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "entityReference")]
    pub entity_reference: Option<Reference>,
    pub identifier: Option<Identifier>,
    #[serde(rename = "effectiveTime")]
    pub effective_time: Option<DateTime>,
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub points: Option<Decimal>,
    pub net: Option<Money>,
    pub payment: Option<String>,
    #[serde(rename = "paymentDate")]
    pub payment_date: Option<DateTime>,
    pub responsible: Option<Reference>,
    pub recipient: Option<Reference>,
    #[serde(rename = "linkId")]
    pub link_id: Option<Vec<String>>,
    #[serde(rename = "securityLabelNumber")]
    pub security_label_number: Option<Vec<UnsignedInt>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContractTermTopic {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContractTerm {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    pub issued: Option<DateTime>,
    pub applies: Option<Period>,
    #[serde(rename = "topicCodeableConcept")]
    pub topic_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "topicReference")]
    pub topic_reference: Option<Reference>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "subType")]
    pub sub_type: Option<CodeableConcept>,
    pub text: Option<String>,
    #[serde(rename = "securityLabel")]
    pub security_label: Option<Vec<ContractTermSecurityLabel>>,
    pub offer: ContractTermOffer,
    pub asset: Option<Vec<ContractTermAsset>>,
    pub action: Option<Vec<ContractTermAction>>,
    pub group: Option<Vec<ContractTerm>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContractRuleContent {
    Attachment(Attachment),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContractRule {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "contentAttachment")]
    pub content_attachment: Attachment,
    #[serde(rename = "contentReference")]
    pub content_reference: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContractTermActionOccurrence {
    DateTime(DateTime),
    Period(Period),
    Timing(Timing),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContractTermAction {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "doNotPerform")]
    pub do_not_perform: Option<Boolean>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub subject: Option<Vec<ContractTermActionSubject>>,
    pub intent: CodeableConcept,
    #[serde(rename = "linkId")]
    pub link_id: Option<Vec<String>>,
    pub status: CodeableConcept,
    pub context: Option<Reference>,
    #[serde(rename = "contextLinkId")]
    pub context_link_id: Option<Vec<String>>,
    #[serde(rename = "occurrenceDateTime")]
    pub occurrence_date_time: Option<DateTime>,
    #[serde(rename = "occurrencePeriod")]
    pub occurrence_period: Option<Period>,
    #[serde(rename = "occurrenceTiming")]
    pub occurrence_timing: Option<Timing>,
    pub requester: Option<Vec<Reference>>,
    #[serde(rename = "requesterLinkId")]
    pub requester_link_id: Option<Vec<String>>,
    #[serde(rename = "performerType")]
    pub performer_type: Option<Vec<CodeableConcept>>,
    #[serde(rename = "performerRole")]
    pub performer_role: Option<CodeableConcept>,
    pub performer: Option<Reference>,
    #[serde(rename = "performerLinkId")]
    pub performer_link_id: Option<Vec<String>>,
    #[serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    pub reason: Option<Vec<String>>,
    #[serde(rename = "reasonLinkId")]
    pub reason_link_id: Option<Vec<String>>,
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "securityLabelNumber")]
    pub security_label_number: Option<Vec<UnsignedInt>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContractFriendlyContent {
    Attachment(Attachment),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContractFriendly {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "contentAttachment")]
    pub content_attachment: Attachment,
    #[serde(rename = "contentReference")]
    pub content_reference: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContractLegalContent {
    Attachment(Attachment),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContractLegal {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "contentAttachment")]
    pub content_attachment: Attachment,
    #[serde(rename = "contentReference")]
    pub content_reference: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContractTermOfferParty {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub reference: Option<Vec<Reference>>,
    pub role: CodeableConcept,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContractTermAsset {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub scope: Option<CodeableConcept>,
    #[serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(rename = "typeReference")]
    pub type_reference: Option<Vec<Reference>>,
    pub subtype: Option<Vec<CodeableConcept>>,
    pub relationship: Option<Coding>,
    pub context: Option<Vec<ContractTermAssetContext>>,
    pub condition: Option<String>,
    #[serde(rename = "periodType")]
    pub period_type: Option<Vec<CodeableConcept>>,
    pub period: Option<Vec<Period>>,
    #[serde(rename = "usePeriod")]
    pub use_period: Option<Vec<Period>>,
    pub text: Option<String>,
    #[serde(rename = "linkId")]
    pub link_id: Option<Vec<String>>,
    pub answer: Option<Vec<ContractTermOfferAnswer>>,
    #[serde(rename = "securityLabelNumber")]
    pub security_label_number: Option<Vec<UnsignedInt>>,
    #[serde(rename = "valuedItem")]
    pub valued_item: Option<Vec<ContractTermAssetValuedItem>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContractTopic {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContractLegallyBinding {
    Attachment(Attachment),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Contract {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub url: Option<Uri>,
    pub version: Option<String>,
    pub status: Option<Code>,
    #[serde(rename = "legalState")]
    pub legal_state: Option<CodeableConcept>,
    #[serde(rename = "instantiatesCanonical")]
    pub instantiates_canonical: Option<Reference>,
    #[serde(rename = "instantiatesUri")]
    pub instantiates_uri: Option<Uri>,
    #[serde(rename = "contentDerivative")]
    pub content_derivative: Option<CodeableConcept>,
    pub issued: Option<DateTime>,
    pub applies: Option<Period>,
    #[serde(rename = "expirationType")]
    pub expiration_type: Option<CodeableConcept>,
    pub subject: Option<Vec<Reference>>,
    pub authority: Option<Vec<Reference>>,
    pub domain: Option<Vec<Reference>>,
    pub site: Option<Vec<Reference>>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub alias: Option<Vec<String>>,
    pub author: Option<Reference>,
    pub scope: Option<CodeableConcept>,
    #[serde(rename = "topicCodeableConcept")]
    pub topic_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "topicReference")]
    pub topic_reference: Option<Reference>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "subType")]
    pub sub_type: Option<Vec<CodeableConcept>>,
    #[serde(rename = "contentDefinition")]
    pub content_definition: Option<ContractContentDefinition>,
    pub term: Option<Vec<ContractTerm>>,
    #[serde(rename = "supportingInfo")]
    pub supporting_info: Option<Vec<Reference>>,
    #[serde(rename = "relevantHistory")]
    pub relevant_history: Option<Vec<Reference>>,
    pub signer: Option<Vec<ContractSigner>>,
    pub friendly: Option<Vec<ContractFriendly>>,
    pub legal: Option<Vec<ContractLegal>>,
    pub rule: Option<Vec<ContractRule>>,
    #[serde(rename = "legallyBindingAttachment")]
    pub legally_binding_attachment: Option<Attachment>,
    #[serde(rename = "legallyBindingReference")]
    pub legally_binding_reference: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContractTermActionSubject {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub reference: Option<Vec<Reference>>,
    pub role: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContractTermOfferAnswerValue {
    Boolean(Boolean),
    Decimal(Decimal),
    Integer(Integer),
    Date(Date),
    DateTime(DateTime),
    Time(Time),
    String(String),
    Uri(Uri),
    Attachment(Attachment),
    Coding(Coding),
    Quantity(Quantity),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContractTermOfferAnswer {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "valueBoolean")]
    pub value_boolean: Boolean,
    #[serde(rename = "valueDecimal")]
    pub value_decimal: Decimal,
    #[serde(rename = "valueInteger")]
    pub value_integer: Integer,
    #[serde(rename = "valueDate")]
    pub value_date: Date,
    #[serde(rename = "valueDateTime")]
    pub value_date_time: DateTime,
    #[serde(rename = "valueTime")]
    pub value_time: Time,
    #[serde(rename = "valueString")]
    pub value_string: String,
    #[serde(rename = "valueUri")]
    pub value_uri: Uri,
    #[serde(rename = "valueAttachment")]
    pub value_attachment: Attachment,
    #[serde(rename = "valueCoding")]
    pub value_coding: Coding,
    #[serde(rename = "valueQuantity")]
    pub value_quantity: Quantity,
    #[serde(rename = "valueReference")]
    pub value_reference: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContractContentDefinition {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(rename = "subType")]
    pub sub_type: Option<CodeableConcept>,
    pub publisher: Option<Reference>,
    #[serde(rename = "publicationDate")]
    pub publication_date: Option<DateTime>,
    #[serde(rename = "publicationStatus")]
    pub publication_status: Code,
    pub copyright: Option<Markdown>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContractTermSecurityLabel {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub number: Option<Vec<UnsignedInt>>,
    pub classification: Coding,
    pub category: Option<Vec<Coding>>,
    pub control: Option<Vec<Coding>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ContractTermAssetContext {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub reference: Option<Reference>,
    pub code: Option<Vec<CodeableConcept>>,
    pub text: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct CoverageClass {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub value: String,
    pub name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CoverageCostToBeneficiaryException {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub period: Option<Period>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Coverage {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "policyHolder")]
    pub policy_holder: Option<Reference>,
    pub subscriber: Option<Reference>,
    #[serde(rename = "subscriberId")]
    pub subscriber_id: Option<String>,
    pub beneficiary: Reference,
    pub dependent: Option<String>,
    pub relationship: Option<CodeableConcept>,
    pub period: Option<Period>,
    pub payor: Option<Vec<Reference>>,
    pub class: Option<Vec<CoverageClass>>,
    pub order: Option<PositiveInt>,
    pub network: Option<String>,
    #[serde(rename = "costToBeneficiary")]
    pub cost_to_beneficiary: Option<Vec<CoverageCostToBeneficiary>>,
    pub subrogation: Option<Boolean>,
    pub contract: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CoverageCostToBeneficiaryValue {
    Quantity(Quantity),
    Money(Money),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CoverageCostToBeneficiary {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "valueQuantity")]
    pub value_quantity: Quantity,
    #[serde(rename = "valueMoney")]
    pub value_money: Money,
    pub exception: Option<Vec<CoverageCostToBeneficiaryException>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CoverageEligibilityRequestItemDiagnosisDiagnosis {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CoverageEligibilityRequestItemDiagnosis {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "diagnosisCodeableConcept")]
    pub diagnosis_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "diagnosisReference")]
    pub diagnosis_reference: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CoverageEligibilityRequestInsurance {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub focal: Option<Boolean>,
    pub coverage: Reference,
    #[serde(rename = "businessArrangement")]
    pub business_arrangement: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CoverageEligibilityRequestServiced {
    Date(Date),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CoverageEligibilityRequest {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub priority: Option<CodeableConcept>,
    pub purpose: Option<Vec<Code>>,
    pub patient: Reference,
    #[serde(rename = "servicedDate")]
    pub serviced_date: Option<Date>,
    #[serde(rename = "servicedPeriod")]
    pub serviced_period: Option<Period>,
    pub created: DateTime,
    pub enterer: Option<Reference>,
    pub provider: Option<Reference>,
    pub insurer: Reference,
    pub facility: Option<Reference>,
    #[serde(rename = "supportingInfo")]
    pub supporting_info: Option<Vec<CoverageEligibilityRequestSupportingInfo>>,
    pub insurance: Option<Vec<CoverageEligibilityRequestInsurance>>,
    pub item: Option<Vec<CoverageEligibilityRequestItem>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CoverageEligibilityRequestSupportingInfo {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub information: Reference,
    #[serde(rename = "appliesToAll")]
    pub applies_to_all: Option<Boolean>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CoverageEligibilityRequestItem {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "supportingInfoSequence")]
    pub supporting_info_sequence: Option<Vec<PositiveInt>>,
    pub category: Option<CodeableConcept>,
    #[serde(rename = "productOrService")]
    pub product_or_service: Option<CodeableConcept>,
    pub modifier: Option<Vec<CodeableConcept>>,
    pub provider: Option<Reference>,
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub facility: Option<Reference>,
    pub diagnosis: Option<Vec<CoverageEligibilityRequestItemDiagnosis>>,
    pub detail: Option<Vec<Reference>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct CoverageEligibilityResponseInsuranceItem {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: Option<CodeableConcept>,
    #[serde(rename = "productOrService")]
    pub product_or_service: Option<CodeableConcept>,
    pub modifier: Option<Vec<CodeableConcept>>,
    pub provider: Option<Reference>,
    pub excluded: Option<Boolean>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub network: Option<CodeableConcept>,
    pub unit: Option<CodeableConcept>,
    pub term: Option<CodeableConcept>,
    pub benefit: Option<Vec<CoverageEligibilityResponseInsuranceItemBenefit>>,
    #[serde(rename = "authorizationRequired")]
    pub authorization_required: Option<Boolean>,
    #[serde(rename = "authorizationSupporting")]
    pub authorization_supporting: Option<Vec<CodeableConcept>>,
    #[serde(rename = "authorizationUrl")]
    pub authorization_url: Option<Uri>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CoverageEligibilityResponseError {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CoverageEligibilityResponseServiced {
    Date(Date),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CoverageEligibilityResponse {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub purpose: Option<Vec<Code>>,
    pub patient: Reference,
    #[serde(rename = "servicedDate")]
    pub serviced_date: Option<Date>,
    #[serde(rename = "servicedPeriod")]
    pub serviced_period: Option<Period>,
    pub created: DateTime,
    pub requestor: Option<Reference>,
    pub request: Reference,
    pub outcome: Code,
    pub disposition: Option<String>,
    pub insurer: Reference,
    pub insurance: Option<Vec<CoverageEligibilityResponseInsurance>>,
    #[serde(rename = "preAuthRef")]
    pub pre_auth_ref: Option<String>,
    pub form: Option<CodeableConcept>,
    pub error: Option<Vec<CoverageEligibilityResponseError>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CoverageEligibilityResponseInsurance {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub coverage: Reference,
    pub inforce: Option<Boolean>,
    #[serde(rename = "benefitPeriod")]
    pub benefit_period: Option<Period>,
    pub item: Option<Vec<CoverageEligibilityResponseInsuranceItem>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CoverageEligibilityResponseInsuranceItemBenefitAllowed {
    UnsignedInt(UnsignedInt),
    String(String),
    Money(Money),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CoverageEligibilityResponseInsuranceItemBenefitUsed {
    UnsignedInt(UnsignedInt),
    String(String),
    Money(Money),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CoverageEligibilityResponseInsuranceItemBenefit {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(rename = "allowedUnsignedInt")]
    pub allowed_unsigned_int: Option<UnsignedInt>,
    #[serde(rename = "allowedString")]
    pub allowed_string: Option<String>,
    #[serde(rename = "allowedMoney")]
    pub allowed_money: Option<Money>,
    #[serde(rename = "usedUnsignedInt")]
    pub used_unsigned_int: Option<UnsignedInt>,
    #[serde(rename = "usedString")]
    pub used_string: Option<String>,
    #[serde(rename = "usedMoney")]
    pub used_money: Option<Money>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DetectedIssueIdentified {
    DateTime(DateTime),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DetectedIssue {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub code: Option<CodeableConcept>,
    pub severity: Option<Code>,
    pub patient: Option<Reference>,
    #[serde(rename = "identifiedDateTime")]
    pub identified_date_time: Option<DateTime>,
    #[serde(rename = "identifiedPeriod")]
    pub identified_period: Option<Period>,
    pub author: Option<Reference>,
    pub implicated: Option<Vec<Reference>>,
    pub evidence: Option<Vec<DetectedIssueEvidence>>,
    pub detail: Option<String>,
    pub reference: Option<Uri>,
    pub mitigation: Option<Vec<DetectedIssueMitigation>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DetectedIssueEvidence {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<Vec<CodeableConcept>>,
    pub detail: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DetectedIssueMitigation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub action: CodeableConcept,
    pub date: Option<DateTime>,
    pub author: Option<Reference>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct Device {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub definition: Option<Reference>,
    #[serde(rename = "udiCarrier")]
    pub udi_carrier: Option<Vec<DeviceUdiCarrier>>,
    pub status: Option<Code>,
    #[serde(rename = "statusReason")]
    pub status_reason: Option<Vec<CodeableConcept>>,
    #[serde(rename = "distinctIdentifier")]
    pub distinct_identifier: Option<String>,
    pub manufacturer: Option<String>,
    #[serde(rename = "manufactureDate")]
    pub manufacture_date: Option<DateTime>,
    #[serde(rename = "expirationDate")]
    pub expiration_date: Option<DateTime>,
    #[serde(rename = "lotNumber")]
    pub lot_number: Option<String>,
    #[serde(rename = "serialNumber")]
    pub serial_number: Option<String>,
    #[serde(rename = "deviceName")]
    pub device_name: Option<Vec<DeviceDeviceName>>,
    #[serde(rename = "modelNumber")]
    pub model_number: Option<String>,
    #[serde(rename = "partNumber")]
    pub part_number: Option<String>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub specialization: Option<Vec<DeviceSpecialization>>,
    pub version: Option<Vec<DeviceVersion>>,
    pub property: Option<Vec<DeviceProperty>>,
    pub patient: Option<Reference>,
    pub owner: Option<Reference>,
    pub contact: Option<Vec<ContactPoint>>,
    pub location: Option<Reference>,
    pub url: Option<Uri>,
    pub note: Option<Vec<Annotation>>,
    pub safety: Option<Vec<CodeableConcept>>,
    pub parent: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeviceUdiCarrier {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "deviceIdentifier")]
    pub device_identifier: Option<String>,
    pub issuer: Option<Uri>,
    pub jurisdiction: Option<Uri>,
    #[serde(rename = "carrierAIDC")]
    pub carrier_a_i_d_c: Option<Base64Binary>,
    #[serde(rename = "carrierHRF")]
    pub carrier_h_r_f: Option<String>,
    #[serde(rename = "entryType")]
    pub entry_type: Option<Code>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeviceDeviceName {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[serde(rename = "type")]
    pub r#type: Code,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeviceVersion {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub component: Option<Identifier>,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeviceProperty {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(rename = "valueQuantity")]
    pub value_quantity: Option<Vec<Quantity>>,
    #[serde(rename = "valueCode")]
    pub value_code: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeviceSpecialization {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "systemType")]
    pub system_type: CodeableConcept,
    pub version: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct DeviceDefinitionSpecialization {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "systemType")]
    pub system_type: String,
    pub version: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeviceDefinitionDeviceName {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[serde(rename = "type")]
    pub r#type: Code,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeviceDefinitionCapability {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub description: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeviceDefinitionUdiDeviceIdentifier {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "deviceIdentifier")]
    pub device_identifier: String,
    pub issuer: Uri,
    pub jurisdiction: Uri,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeviceDefinitionProperty {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(rename = "valueQuantity")]
    pub value_quantity: Option<Vec<Quantity>>,
    #[serde(rename = "valueCode")]
    pub value_code: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeviceDefinitionMaterial {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub substance: CodeableConcept,
    pub alternate: Option<Boolean>,
    #[serde(rename = "allergenicIndicator")]
    pub allergenic_indicator: Option<Boolean>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceDefinitionManufacturer {
    String(String),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeviceDefinition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "udiDeviceIdentifier")]
    pub udi_device_identifier: Option<Vec<DeviceDefinitionUdiDeviceIdentifier>>,
    #[serde(rename = "manufacturerString")]
    pub manufacturer_string: Option<String>,
    #[serde(rename = "manufacturerReference")]
    pub manufacturer_reference: Option<Reference>,
    #[serde(rename = "deviceName")]
    pub device_name: Option<Vec<DeviceDefinitionDeviceName>>,
    #[serde(rename = "modelNumber")]
    pub model_number: Option<String>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub specialization: Option<Vec<DeviceDefinitionSpecialization>>,
    pub version: Option<Vec<String>>,
    pub safety: Option<Vec<CodeableConcept>>,
    #[serde(rename = "shelfLifeStorage")]
    pub shelf_life_storage: Option<Vec<ProductShelfLife>>,
    #[serde(rename = "physicalCharacteristics")]
    pub physical_characteristics: Option<ProdCharacteristic>,
    #[serde(rename = "languageCode")]
    pub language_code: Option<Vec<CodeableConcept>>,
    pub capability: Option<Vec<DeviceDefinitionCapability>>,
    pub property: Option<Vec<DeviceDefinitionProperty>>,
    pub owner: Option<Reference>,
    pub contact: Option<Vec<ContactPoint>>,
    pub url: Option<Uri>,
    #[serde(rename = "onlineInformation")]
    pub online_information: Option<Uri>,
    pub note: Option<Vec<Annotation>>,
    pub quantity: Option<Quantity>,
    #[serde(rename = "parentDevice")]
    pub parent_device: Option<Reference>,
    pub material: Option<Vec<DeviceDefinitionMaterial>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct DeviceMetric {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub unit: Option<CodeableConcept>,
    pub source: Option<Reference>,
    pub parent: Option<Reference>,
    #[serde(rename = "operationalStatus")]
    pub operational_status: Option<Code>,
    pub color: Option<Code>,
    pub category: Code,
    #[serde(rename = "measurementPeriod")]
    pub measurement_period: Option<Timing>,
    pub calibration: Option<Vec<DeviceMetricCalibration>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeviceMetricCalibration {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Option<Code>,
    pub state: Option<Code>,
    pub time: Option<Instant>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceRequestCode {
    Reference(Reference),
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceRequestOccurrence {
    DateTime(DateTime),
    Period(Period),
    Timing(Timing),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeviceRequest {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "instantiatesCanonical")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[serde(rename = "instantiatesUri")]
    pub instantiates_uri: Option<Vec<Uri>>,
    #[serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(rename = "priorRequest")]
    pub prior_request: Option<Vec<Reference>>,
    #[serde(rename = "groupIdentifier")]
    pub group_identifier: Option<Identifier>,
    pub status: Option<Code>,
    pub intent: Code,
    pub priority: Option<Code>,
    #[serde(rename = "codeReference")]
    pub code_reference: Reference,
    #[serde(rename = "codeCodeableConcept")]
    pub code_codeable_concept: CodeableConcept,
    pub parameter: Option<Vec<DeviceRequestParameter>>,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    #[serde(rename = "occurrenceDateTime")]
    pub occurrence_date_time: Option<DateTime>,
    #[serde(rename = "occurrencePeriod")]
    pub occurrence_period: Option<Period>,
    #[serde(rename = "occurrenceTiming")]
    pub occurrence_timing: Option<Timing>,
    #[serde(rename = "authoredOn")]
    pub authored_on: Option<DateTime>,
    pub requester: Option<Reference>,
    #[serde(rename = "performerType")]
    pub performer_type: Option<CodeableConcept>,
    pub performer: Option<Reference>,
    #[serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    pub insurance: Option<Vec<Reference>>,
    #[serde(rename = "supportingInfo")]
    pub supporting_info: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "relevantHistory")]
    pub relevant_history: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceRequestParameterValue {
    CodeableConcept(CodeableConcept),
    Quantity(Quantity),
    Range(Range),
    Boolean(Boolean),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeviceRequestParameter {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    #[serde(rename = "valueCodeableConcept")]
    pub value_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "valueQuantity")]
    pub value_quantity: Option<Quantity>,
    #[serde(rename = "valueRange")]
    pub value_range: Option<Range>,
    #[serde(rename = "valueBoolean")]
    pub value_boolean: Option<Boolean>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceUseStatementTiming {
    Timing(Timing),
    Period(Period),
    DateTime(DateTime),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeviceUseStatement {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    pub status: Code,
    pub subject: Reference,
    #[serde(rename = "derivedFrom")]
    pub derived_from: Option<Vec<Reference>>,
    #[serde(rename = "timingTiming")]
    pub timing_timing: Option<Timing>,
    #[serde(rename = "timingPeriod")]
    pub timing_period: Option<Period>,
    #[serde(rename = "timingDateTime")]
    pub timing_date_time: Option<DateTime>,
    #[serde(rename = "recordedOn")]
    pub recorded_on: Option<DateTime>,
    pub source: Option<Reference>,
    pub device: Reference,
    #[serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(rename = "bodySite")]
    pub body_site: Option<CodeableConcept>,
    pub note: Option<Vec<Annotation>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticReportEffective {
    DateTime(DateTime),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DiagnosticReport {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    pub status: Code,
    pub category: Option<Vec<CodeableConcept>>,
    pub code: CodeableConcept,
    pub subject: Option<Reference>,
    pub encounter: Option<Reference>,
    #[serde(rename = "effectiveDateTime")]
    pub effective_date_time: Option<DateTime>,
    #[serde(rename = "effectivePeriod")]
    pub effective_period: Option<Period>,
    pub issued: Option<Instant>,
    pub performer: Option<Vec<Reference>>,
    #[serde(rename = "resultsInterpreter")]
    pub results_interpreter: Option<Vec<Reference>>,
    pub specimen: Option<Vec<Reference>>,
    pub result: Option<Vec<Reference>>,
    #[serde(rename = "imagingStudy")]
    pub imaging_study: Option<Vec<Reference>>,
    pub media: Option<Vec<DiagnosticReportMedia>>,
    pub conclusion: Option<String>,
    #[serde(rename = "conclusionCode")]
    pub conclusion_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "presentedForm")]
    pub presented_form: Option<Vec<Attachment>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DiagnosticReportMedia {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub comment: Option<String>,
    pub link: Reference,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct DocumentManifest {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "masterIdentifier")]
    pub master_identifier: Option<Identifier>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub subject: Option<Reference>,
    pub created: Option<DateTime>,
    pub author: Option<Vec<Reference>>,
    pub recipient: Option<Vec<Reference>>,
    pub source: Option<Uri>,
    pub description: Option<String>,
    pub content: Option<Vec<Reference>>,
    pub related: Option<Vec<DocumentManifestRelated>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DocumentManifestRelated {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    #[serde(rename = "ref")]
    pub r#ref: Option<Reference>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct DocumentReferenceContent {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub attachment: Attachment,
    pub format: Option<Coding>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DocumentReference {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "masterIdentifier")]
    pub master_identifier: Option<Identifier>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "docStatus")]
    pub doc_status: Option<Code>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub category: Option<Vec<CodeableConcept>>,
    pub subject: Option<Reference>,
    pub date: Option<Instant>,
    pub author: Option<Vec<Reference>>,
    pub authenticator: Option<Reference>,
    pub custodian: Option<Reference>,
    #[serde(rename = "relatesTo")]
    pub relates_to: Option<Vec<DocumentReferenceRelatesTo>>,
    pub description: Option<String>,
    #[serde(rename = "securityLabel")]
    pub security_label: Option<Vec<CodeableConcept>>,
    pub content: Option<Vec<DocumentReferenceContent>>,
    pub context: Option<DocumentReferenceContext>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DocumentReferenceContext {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub encounter: Option<Vec<Reference>>,
    pub event: Option<Vec<CodeableConcept>>,
    pub period: Option<Period>,
    #[serde(rename = "facilityType")]
    pub facility_type: Option<CodeableConcept>,
    #[serde(rename = "practiceSetting")]
    pub practice_setting: Option<CodeableConcept>,
    #[serde(rename = "sourcePatientInfo")]
    pub source_patient_info: Option<Reference>,
    pub related: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DocumentReferenceRelatesTo {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub target: Reference,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct Encounter {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "statusHistory")]
    pub status_history: Option<Vec<EncounterStatusHistory>>,
    pub class: Coding,
    #[serde(rename = "classHistory")]
    pub class_history: Option<Vec<EncounterClassHistory>>,
    #[serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(rename = "serviceType")]
    pub service_type: Option<CodeableConcept>,
    pub priority: Option<CodeableConcept>,
    pub subject: Option<Reference>,
    #[serde(rename = "episodeOfCare")]
    pub episode_of_care: Option<Vec<Reference>>,
    #[serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    pub participant: Option<Vec<EncounterParticipant>>,
    pub appointment: Option<Vec<Reference>>,
    pub period: Option<Period>,
    pub length: Option<Duration>,
    #[serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    pub diagnosis: Option<Vec<EncounterDiagnosis>>,
    pub account: Option<Vec<Reference>>,
    pub hospitalization: Option<EncounterHospitalization>,
    pub location: Option<Vec<EncounterLocation>>,
    #[serde(rename = "serviceProvider")]
    pub service_provider: Option<Reference>,
    #[serde(rename = "partOf")]
    pub part_of: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EncounterStatusHistory {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub status: Code,
    pub period: Period,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EncounterDiagnosis {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub condition: Reference,
    #[serde(rename = "use")]
    pub r#use: Option<CodeableConcept>,
    pub rank: Option<PositiveInt>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EncounterHospitalization {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "preAdmissionIdentifier")]
    pub pre_admission_identifier: Option<Identifier>,
    pub origin: Option<Reference>,
    #[serde(rename = "admitSource")]
    pub admit_source: Option<CodeableConcept>,
    #[serde(rename = "reAdmission")]
    pub re_admission: Option<CodeableConcept>,
    #[serde(rename = "dietPreference")]
    pub diet_preference: Option<Vec<CodeableConcept>>,
    #[serde(rename = "specialCourtesy")]
    pub special_courtesy: Option<Vec<CodeableConcept>>,
    #[serde(rename = "specialArrangement")]
    pub special_arrangement: Option<Vec<CodeableConcept>>,
    pub destination: Option<Reference>,
    #[serde(rename = "dischargeDisposition")]
    pub discharge_disposition: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EncounterLocation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub location: Reference,
    pub status: Option<Code>,
    #[serde(rename = "physicalType")]
    pub physical_type: Option<CodeableConcept>,
    pub period: Option<Period>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EncounterParticipant {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    pub period: Option<Period>,
    pub individual: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EncounterClassHistory {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub class: Coding,
    pub period: Period,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct Endpoint {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "connectionType")]
    pub connection_type: Coding,
    pub name: Option<String>,
    #[serde(rename = "managingOrganization")]
    pub managing_organization: Option<Reference>,
    pub contact: Option<Vec<ContactPoint>>,
    pub period: Option<Period>,
    #[serde(rename = "payloadType")]
    pub payload_type: Option<Vec<CodeableConcept>>,
    #[serde(rename = "payloadMimeType")]
    pub payload_mime_type: Option<Vec<Code>>,
    pub address: Url,
    pub header: Option<Vec<String>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct EnrollmentRequest {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Option<Code>,
    pub created: Option<DateTime>,
    pub insurer: Option<Reference>,
    pub provider: Option<Reference>,
    pub candidate: Option<Reference>,
    pub coverage: Option<Reference>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct EnrollmentResponse {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Option<Code>,
    pub request: Option<Reference>,
    pub outcome: Option<Code>,
    pub disposition: Option<String>,
    pub created: Option<DateTime>,
    pub organization: Option<Reference>,
    #[serde(rename = "requestProvider")]
    pub request_provider: Option<Reference>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct EpisodeOfCare {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "statusHistory")]
    pub status_history: Option<Vec<EpisodeOfCareStatusHistory>>,
    #[serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    pub diagnosis: Option<Vec<EpisodeOfCareDiagnosis>>,
    pub patient: Reference,
    #[serde(rename = "managingOrganization")]
    pub managing_organization: Option<Reference>,
    pub period: Option<Period>,
    #[serde(rename = "referralRequest")]
    pub referral_request: Option<Vec<Reference>>,
    #[serde(rename = "careManager")]
    pub care_manager: Option<Reference>,
    pub team: Option<Vec<Reference>>,
    pub account: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EpisodeOfCareStatusHistory {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub status: Code,
    pub period: Period,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EpisodeOfCareDiagnosis {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub condition: Reference,
    pub role: Option<CodeableConcept>,
    pub rank: Option<PositiveInt>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EventDefinitionSubject {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EventDefinition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    #[serde(rename = "subjectCodeableConcept")]
    pub subject_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "subjectReference")]
    pub subject_reference: Option<Reference>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub usage: Option<String>,
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod")]
    pub effective_period: Option<Period>,
    pub topic: Option<Vec<CodeableConcept>>,
    pub author: Option<Vec<ContactDetail>>,
    pub editor: Option<Vec<ContactDetail>>,
    pub reviewer: Option<Vec<ContactDetail>>,
    pub endorser: Option<Vec<ContactDetail>>,
    #[serde(rename = "relatedArtifact")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    pub trigger: Option<Vec<TriggerDefinition>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct EvidenceStatistic {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "statisticType")]
    pub statistic_type: Option<CodeableConcept>,
    pub category: Option<CodeableConcept>,
    pub quantity: Option<Quantity>,
    #[serde(rename = "numberOfEvents")]
    pub number_of_events: Option<UnsignedInt>,
    #[serde(rename = "numberAffected")]
    pub number_affected: Option<UnsignedInt>,
    #[serde(rename = "sampleSize")]
    pub sample_size: Option<EvidenceStatisticSampleSize>,
    #[serde(rename = "attributeEstimate")]
    pub attribute_estimate: Option<Vec<EvidenceStatisticAttributeEstimate>>,
    #[serde(rename = "modelCharacteristic")]
    pub model_characteristic: Option<Vec<EvidenceStatisticModelCharacteristic>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EvidenceStatisticSampleSize {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "numberOfStudies")]
    pub number_of_studies: Option<UnsignedInt>,
    #[serde(rename = "numberOfParticipants")]
    pub number_of_participants: Option<UnsignedInt>,
    #[serde(rename = "knownDataCount")]
    pub known_data_count: Option<UnsignedInt>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EvidenceStatisticModelCharacteristic {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    pub value: Option<Quantity>,
    pub variable: Option<Vec<EvidenceStatisticModelCharacteristicVariable>>,
    #[serde(rename = "attributeEstimate")]
    pub attribute_estimate: Option<Vec<EvidenceStatisticAttributeEstimate>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EvidenceStatisticModelCharacteristicVariable {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "variableDefinition")]
    pub variable_definition: Reference,
    pub handling: Option<Code>,
    #[serde(rename = "valueCategory")]
    pub value_category: Option<Vec<CodeableConcept>>,
    #[serde(rename = "valueQuantity")]
    pub value_quantity: Option<Vec<Quantity>>,
    #[serde(rename = "valueRange")]
    pub value_range: Option<Vec<Range>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EvidenceCiteAs {
    Reference(Reference),
    Markdown(Markdown),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Evidence {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub title: Option<String>,
    #[serde(rename = "citeAsReference")]
    pub cite_as_reference: Option<Reference>,
    #[serde(rename = "citeAsMarkdown")]
    pub cite_as_markdown: Option<Markdown>,
    pub status: Code,
    pub date: Option<DateTime>,
    #[serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    #[serde(rename = "approvalDate")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate")]
    pub last_review_date: Option<Date>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub author: Option<Vec<ContactDetail>>,
    pub editor: Option<Vec<ContactDetail>>,
    pub reviewer: Option<Vec<ContactDetail>>,
    pub endorser: Option<Vec<ContactDetail>>,
    #[serde(rename = "relatedArtifact")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    pub description: Option<Markdown>,
    pub assertion: Option<Markdown>,
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "variableDefinition")]
    pub variable_definition: Option<Vec<EvidenceVariableDefinition>>,
    #[serde(rename = "synthesisType")]
    pub synthesis_type: Option<CodeableConcept>,
    #[serde(rename = "studyType")]
    pub study_type: Option<CodeableConcept>,
    pub statistic: Option<Vec<EvidenceStatistic>>,
    pub certainty: Option<Vec<EvidenceCertainty>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EvidenceVariableDefinition {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<Markdown>,
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "variableRole")]
    pub variable_role: CodeableConcept,
    pub observed: Option<Reference>,
    pub intended: Option<Reference>,
    #[serde(rename = "directnessMatch")]
    pub directness_match: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EvidenceStatisticAttributeEstimate {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub quantity: Option<Quantity>,
    pub level: Option<Decimal>,
    pub range: Option<Range>,
    #[serde(rename = "attributeEstimate")]
    pub attribute_estimate: Option<Vec<EvidenceStatisticAttributeEstimate>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EvidenceCertainty {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub rating: Option<CodeableConcept>,
    pub rater: Option<String>,
    pub subcomponent: Option<Vec<EvidenceCertainty>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EvidenceReportCiteAs {
    Reference(Reference),
    Markdown(Markdown),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EvidenceReport {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub status: Code,
    #[serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "relatedIdentifier")]
    pub related_identifier: Option<Vec<Identifier>>,
    #[serde(rename = "citeAsReference")]
    pub cite_as_reference: Option<Reference>,
    #[serde(rename = "citeAsMarkdown")]
    pub cite_as_markdown: Option<Markdown>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "relatedArtifact")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    pub subject: EvidenceReportSubject,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub author: Option<Vec<ContactDetail>>,
    pub editor: Option<Vec<ContactDetail>>,
    pub reviewer: Option<Vec<ContactDetail>>,
    pub endorser: Option<Vec<ContactDetail>>,
    #[serde(rename = "relatesTo")]
    pub relates_to: Option<Vec<EvidenceReportRelatesTo>>,
    pub section: Option<Vec<EvidenceReportSection>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EvidenceReportRelatesToTarget {
    Identifier(Identifier),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EvidenceReportRelatesTo {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    #[serde(rename = "targetIdentifier")]
    pub target_identifier: Identifier,
    #[serde(rename = "targetReference")]
    pub target_reference: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EvidenceReportSubject {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub characteristic: Option<Vec<EvidenceReportSubjectCharacteristic>>,
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EvidenceReportSection {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub title: Option<String>,
    pub focus: Option<CodeableConcept>,
    #[serde(rename = "focusReference")]
    pub focus_reference: Option<Reference>,
    pub author: Option<Vec<Reference>>,
    pub text: Option<Narrative>,
    pub mode: Option<Code>,
    #[serde(rename = "orderedBy")]
    pub ordered_by: Option<CodeableConcept>,
    #[serde(rename = "entryClassifier")]
    pub entry_classifier: Option<Vec<CodeableConcept>>,
    #[serde(rename = "entryReference")]
    pub entry_reference: Option<Vec<Reference>>,
    #[serde(rename = "entryQuantity")]
    pub entry_quantity: Option<Vec<Quantity>>,
    #[serde(rename = "emptyReason")]
    pub empty_reason: Option<CodeableConcept>,
    pub section: Option<Vec<EvidenceReportSection>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EvidenceReportSubjectCharacteristicValue {
    Reference(Reference),
    CodeableConcept(CodeableConcept),
    Boolean(Boolean),
    Quantity(Quantity),
    Range(Range),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EvidenceReportSubjectCharacteristic {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    #[serde(rename = "valueReference")]
    pub value_reference: Reference,
    #[serde(rename = "valueCodeableConcept")]
    pub value_codeable_concept: CodeableConcept,
    #[serde(rename = "valueBoolean")]
    pub value_boolean: Boolean,
    #[serde(rename = "valueQuantity")]
    pub value_quantity: Quantity,
    #[serde(rename = "valueRange")]
    pub value_range: Range,
    pub exclude: Option<Boolean>,
    pub period: Option<Period>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct EvidenceVariable {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    #[serde(rename = "shortTitle")]
    pub short_title: Option<String>,
    pub subtitle: Option<String>,
    pub status: Code,
    pub date: Option<DateTime>,
    pub description: Option<Markdown>,
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub author: Option<Vec<ContactDetail>>,
    pub editor: Option<Vec<ContactDetail>>,
    pub reviewer: Option<Vec<ContactDetail>>,
    pub endorser: Option<Vec<ContactDetail>>,
    #[serde(rename = "relatedArtifact")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    pub actual: Option<Boolean>,
    #[serde(rename = "characteristicCombination")]
    pub characteristic_combination: Option<Code>,
    pub characteristic: Option<Vec<EvidenceVariableCharacteristic>>,
    pub handling: Option<Code>,
    pub category: Option<Vec<EvidenceVariableCategory>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EvidenceVariableCharacteristicDefinition {
    Reference(Reference),
    Canonical(Canonical),
    CodeableConcept(CodeableConcept),
    Expression(Expression),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EvidenceVariableCharacteristic {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    #[serde(rename = "definitionReference")]
    pub definition_reference: Reference,
    #[serde(rename = "definitionCanonical")]
    pub definition_canonical: Canonical,
    #[serde(rename = "definitionCodeableConcept")]
    pub definition_codeable_concept: CodeableConcept,
    #[serde(rename = "definitionExpression")]
    pub definition_expression: Expression,
    pub method: Option<CodeableConcept>,
    pub device: Option<Reference>,
    pub exclude: Option<Boolean>,
    #[serde(rename = "timeFromStart")]
    pub time_from_start: Option<EvidenceVariableCharacteristicTimeFromStart>,
    #[serde(rename = "groupMeasure")]
    pub group_measure: Option<Code>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EvidenceVariableCategoryValue {
    CodeableConcept(CodeableConcept),
    Quantity(Quantity),
    Range(Range),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EvidenceVariableCategory {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Option<String>,
    #[serde(rename = "valueCodeableConcept")]
    pub value_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "valueQuantity")]
    pub value_quantity: Option<Quantity>,
    #[serde(rename = "valueRange")]
    pub value_range: Option<Range>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EvidenceVariableCharacteristicTimeFromStart {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    pub quantity: Option<Quantity>,
    pub range: Option<Range>,
    pub note: Option<Vec<Annotation>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct ExampleScenarioProcessStepAlternative {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub title: String,
    pub description: Option<Markdown>,
    pub step: Option<Vec<ExampleScenarioProcessStep>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExampleScenarioProcess {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub title: String,
    pub description: Option<Markdown>,
    #[serde(rename = "preConditions")]
    pub pre_conditions: Option<Markdown>,
    #[serde(rename = "postConditions")]
    pub post_conditions: Option<Markdown>,
    pub step: Option<Vec<ExampleScenarioProcessStep>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExampleScenarioInstance {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "resourceId")]
    pub resource_id: String,
    #[serde(rename = "resourceType")]
    pub resource_type: Code,
    pub name: Option<String>,
    pub description: Option<Markdown>,
    pub version: Option<Vec<ExampleScenarioInstanceVersion>>,
    #[serde(rename = "containedInstance")]
    pub contained_instance: Option<Vec<ExampleScenarioInstanceContainedInstance>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExampleScenarioProcessStepOperation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub number: String,
    #[serde(rename = "type")]
    pub r#type: Option<String>,
    pub name: Option<String>,
    pub initiator: Option<String>,
    pub receiver: Option<String>,
    pub description: Option<Markdown>,
    #[serde(rename = "initiatorActive")]
    pub initiator_active: Option<Boolean>,
    #[serde(rename = "receiverActive")]
    pub receiver_active: Option<Boolean>,
    pub request: Option<ExampleScenarioInstanceContainedInstance>,
    pub response: Option<ExampleScenarioInstanceContainedInstance>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExampleScenarioInstanceContainedInstance {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "resourceId")]
    pub resource_id: String,
    #[serde(rename = "versionId")]
    pub version_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExampleScenario {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub copyright: Option<Markdown>,
    pub purpose: Option<Markdown>,
    pub actor: Option<Vec<ExampleScenarioActor>>,
    pub instance: Option<Vec<ExampleScenarioInstance>>,
    pub process: Option<Vec<ExampleScenarioProcess>>,
    pub workflow: Option<Vec<Canonical>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExampleScenarioProcessStep {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub process: Option<Vec<ExampleScenarioProcess>>,
    pub pause: Option<Boolean>,
    pub operation: Option<ExampleScenarioProcessStepOperation>,
    pub alternative: Option<Vec<ExampleScenarioProcessStepAlternative>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExampleScenarioInstanceVersion {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "versionId")]
    pub version_id: String,
    pub description: Markdown,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExampleScenarioActor {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "actorId")]
    pub actor_id: String,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub name: Option<String>,
    pub description: Option<Markdown>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct ExplanationOfBenefitAddItemDetail {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    pub modifier: Option<Vec<CodeableConcept>>,
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub net: Option<Money>,
    #[serde(rename = "noteNumber")]
    pub note_number: Option<Vec<PositiveInt>>,
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
    #[serde(rename = "subDetail")]
    pub sub_detail: Option<Vec<ExplanationOfBenefitAddItemDetailSubDetail>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExplanationOfBenefitItemDetailSubDetail {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub revenue: Option<CodeableConcept>,
    pub category: Option<CodeableConcept>,
    #[serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    pub modifier: Option<Vec<CodeableConcept>>,
    #[serde(rename = "programCode")]
    pub program_code: Option<Vec<CodeableConcept>>,
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub net: Option<Money>,
    pub udi: Option<Vec<Reference>>,
    #[serde(rename = "noteNumber")]
    pub note_number: Option<Vec<PositiveInt>>,
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitItemServiced {
    Date(Date),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitItemLocation {
    CodeableConcept(CodeableConcept),
    Address(Address),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExplanationOfBenefitItem {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    #[serde(rename = "careTeamSequence")]
    pub care_team_sequence: Option<Vec<PositiveInt>>,
    #[serde(rename = "diagnosisSequence")]
    pub diagnosis_sequence: Option<Vec<PositiveInt>>,
    #[serde(rename = "procedureSequence")]
    pub procedure_sequence: Option<Vec<PositiveInt>>,
    #[serde(rename = "informationSequence")]
    pub information_sequence: Option<Vec<PositiveInt>>,
    pub revenue: Option<CodeableConcept>,
    pub category: Option<CodeableConcept>,
    #[serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    pub modifier: Option<Vec<CodeableConcept>>,
    #[serde(rename = "programCode")]
    pub program_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "servicedDate")]
    pub serviced_date: Option<Date>,
    #[serde(rename = "servicedPeriod")]
    pub serviced_period: Option<Period>,
    #[serde(rename = "locationCodeableConcept")]
    pub location_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "locationAddress")]
    pub location_address: Option<Address>,
    #[serde(rename = "locationReference")]
    pub location_reference: Option<Reference>,
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub net: Option<Money>,
    pub udi: Option<Vec<Reference>>,
    #[serde(rename = "bodySite")]
    pub body_site: Option<CodeableConcept>,
    #[serde(rename = "subSite")]
    pub sub_site: Option<Vec<CodeableConcept>>,
    pub encounter: Option<Vec<Reference>>,
    #[serde(rename = "noteNumber")]
    pub note_number: Option<Vec<PositiveInt>>,
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
    pub detail: Option<Vec<ExplanationOfBenefitItemDetail>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExplanationOfBenefitTotal {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: CodeableConcept,
    pub amount: Money,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitDiagnosisDiagnosis {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExplanationOfBenefitDiagnosis {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    #[serde(rename = "diagnosisCodeableConcept")]
    pub diagnosis_codeable_concept: CodeableConcept,
    #[serde(rename = "diagnosisReference")]
    pub diagnosis_reference: Reference,
    #[serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(rename = "onAdmission")]
    pub on_admission: Option<CodeableConcept>,
    #[serde(rename = "packageCode")]
    pub package_code: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExplanationOfBenefitItemDetail {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub revenue: Option<CodeableConcept>,
    pub category: Option<CodeableConcept>,
    #[serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    pub modifier: Option<Vec<CodeableConcept>>,
    #[serde(rename = "programCode")]
    pub program_code: Option<Vec<CodeableConcept>>,
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub net: Option<Money>,
    pub udi: Option<Vec<Reference>>,
    #[serde(rename = "noteNumber")]
    pub note_number: Option<Vec<PositiveInt>>,
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
    #[serde(rename = "subDetail")]
    pub sub_detail: Option<Vec<ExplanationOfBenefitItemDetailSubDetail>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExplanationOfBenefitProcessNote {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub number: Option<PositiveInt>,
    #[serde(rename = "type")]
    pub r#type: Option<Code>,
    pub text: Option<String>,
    pub language: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExplanationOfBenefitInsurance {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub focal: Boolean,
    pub coverage: Reference,
    #[serde(rename = "preAuthRef")]
    pub pre_auth_ref: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExplanationOfBenefitItemAdjudication {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: CodeableConcept,
    pub reason: Option<CodeableConcept>,
    pub amount: Option<Money>,
    pub value: Option<Decimal>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitBenefitBalanceFinancialAllowed {
    UnsignedInt(UnsignedInt),
    String(String),
    Money(Money),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitBenefitBalanceFinancialUsed {
    UnsignedInt(UnsignedInt),
    Money(Money),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExplanationOfBenefitBenefitBalanceFinancial {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(rename = "allowedUnsignedInt")]
    pub allowed_unsigned_int: Option<UnsignedInt>,
    #[serde(rename = "allowedString")]
    pub allowed_string: Option<String>,
    #[serde(rename = "allowedMoney")]
    pub allowed_money: Option<Money>,
    #[serde(rename = "usedUnsignedInt")]
    pub used_unsigned_int: Option<UnsignedInt>,
    #[serde(rename = "usedMoney")]
    pub used_money: Option<Money>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitProcedureProcedure {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExplanationOfBenefitProcedure {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    #[serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    pub date: Option<DateTime>,
    #[serde(rename = "procedureCodeableConcept")]
    pub procedure_codeable_concept: CodeableConcept,
    #[serde(rename = "procedureReference")]
    pub procedure_reference: Reference,
    pub udi: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExplanationOfBenefitRelated {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub claim: Option<Reference>,
    pub relationship: Option<CodeableConcept>,
    pub reference: Option<Identifier>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitAccidentLocation {
    Address(Address),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExplanationOfBenefitAccident {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub date: Option<Date>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "locationAddress")]
    pub location_address: Option<Address>,
    #[serde(rename = "locationReference")]
    pub location_reference: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitSupportingInfoTiming {
    Date(Date),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitSupportingInfoValue {
    Boolean(Boolean),
    String(String),
    Quantity(Quantity),
    Attachment(Attachment),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExplanationOfBenefitSupportingInfo {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub category: CodeableConcept,
    pub code: Option<CodeableConcept>,
    #[serde(rename = "timingDate")]
    pub timing_date: Option<Date>,
    #[serde(rename = "timingPeriod")]
    pub timing_period: Option<Period>,
    #[serde(rename = "valueBoolean")]
    pub value_boolean: Option<Boolean>,
    #[serde(rename = "valueString")]
    pub value_string: Option<String>,
    #[serde(rename = "valueQuantity")]
    pub value_quantity: Option<Quantity>,
    #[serde(rename = "valueAttachment")]
    pub value_attachment: Option<Attachment>,
    #[serde(rename = "valueReference")]
    pub value_reference: Option<Reference>,
    pub reason: Option<Coding>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitAddItemServiced {
    Date(Date),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitAddItemLocation {
    CodeableConcept(CodeableConcept),
    Address(Address),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExplanationOfBenefitAddItem {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "itemSequence")]
    pub item_sequence: Option<Vec<PositiveInt>>,
    #[serde(rename = "detailSequence")]
    pub detail_sequence: Option<Vec<PositiveInt>>,
    #[serde(rename = "subDetailSequence")]
    pub sub_detail_sequence: Option<Vec<PositiveInt>>,
    pub provider: Option<Vec<Reference>>,
    #[serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    pub modifier: Option<Vec<CodeableConcept>>,
    #[serde(rename = "programCode")]
    pub program_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "servicedDate")]
    pub serviced_date: Option<Date>,
    #[serde(rename = "servicedPeriod")]
    pub serviced_period: Option<Period>,
    #[serde(rename = "locationCodeableConcept")]
    pub location_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "locationAddress")]
    pub location_address: Option<Address>,
    #[serde(rename = "locationReference")]
    pub location_reference: Option<Reference>,
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub net: Option<Money>,
    #[serde(rename = "bodySite")]
    pub body_site: Option<CodeableConcept>,
    #[serde(rename = "subSite")]
    pub sub_site: Option<Vec<CodeableConcept>>,
    #[serde(rename = "noteNumber")]
    pub note_number: Option<Vec<PositiveInt>>,
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
    pub detail: Option<Vec<ExplanationOfBenefitAddItemDetail>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExplanationOfBenefit {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(rename = "subType")]
    pub sub_type: Option<CodeableConcept>,
    #[serde(rename = "use")]
    pub r#use: Code,
    pub patient: Reference,
    #[serde(rename = "billablePeriod")]
    pub billable_period: Option<Period>,
    pub created: DateTime,
    pub enterer: Option<Reference>,
    pub insurer: Reference,
    pub provider: Reference,
    pub priority: Option<CodeableConcept>,
    #[serde(rename = "fundsReserveRequested")]
    pub funds_reserve_requested: Option<CodeableConcept>,
    #[serde(rename = "fundsReserve")]
    pub funds_reserve: Option<CodeableConcept>,
    pub related: Option<Vec<ExplanationOfBenefitRelated>>,
    pub prescription: Option<Reference>,
    #[serde(rename = "originalPrescription")]
    pub original_prescription: Option<Reference>,
    pub payee: Option<ExplanationOfBenefitPayee>,
    pub referral: Option<Reference>,
    pub facility: Option<Reference>,
    pub claim: Option<Reference>,
    #[serde(rename = "claimResponse")]
    pub claim_response: Option<Reference>,
    pub outcome: Code,
    pub disposition: Option<String>,
    #[serde(rename = "preAuthRef")]
    pub pre_auth_ref: Option<Vec<String>>,
    #[serde(rename = "preAuthRefPeriod")]
    pub pre_auth_ref_period: Option<Vec<Period>>,
    #[serde(rename = "careTeam")]
    pub care_team: Option<Vec<ExplanationOfBenefitCareTeam>>,
    #[serde(rename = "supportingInfo")]
    pub supporting_info: Option<Vec<ExplanationOfBenefitSupportingInfo>>,
    pub diagnosis: Option<Vec<ExplanationOfBenefitDiagnosis>>,
    pub procedure: Option<Vec<ExplanationOfBenefitProcedure>>,
    pub precedence: Option<PositiveInt>,
    pub insurance: Option<Vec<ExplanationOfBenefitInsurance>>,
    pub accident: Option<ExplanationOfBenefitAccident>,
    pub item: Option<Vec<ExplanationOfBenefitItem>>,
    #[serde(rename = "addItem")]
    pub add_item: Option<Vec<ExplanationOfBenefitAddItem>>,
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
    pub total: Option<Vec<ExplanationOfBenefitTotal>>,
    pub payment: Option<ExplanationOfBenefitPayment>,
    #[serde(rename = "formCode")]
    pub form_code: Option<CodeableConcept>,
    pub form: Option<Attachment>,
    #[serde(rename = "processNote")]
    pub process_note: Option<Vec<ExplanationOfBenefitProcessNote>>,
    #[serde(rename = "benefitPeriod")]
    pub benefit_period: Option<Period>,
    #[serde(rename = "benefitBalance")]
    pub benefit_balance: Option<Vec<ExplanationOfBenefitBenefitBalance>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExplanationOfBenefitPayee {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub party: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExplanationOfBenefitCareTeam {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub provider: Reference,
    pub responsible: Option<Boolean>,
    pub role: Option<CodeableConcept>,
    pub qualification: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExplanationOfBenefitAddItemDetailSubDetail {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "productOrService")]
    pub product_or_service: CodeableConcept,
    pub modifier: Option<Vec<CodeableConcept>>,
    pub quantity: Option<Quantity>,
    #[serde(rename = "unitPrice")]
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub net: Option<Money>,
    #[serde(rename = "noteNumber")]
    pub note_number: Option<Vec<PositiveInt>>,
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExplanationOfBenefitPayment {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub adjustment: Option<Money>,
    #[serde(rename = "adjustmentReason")]
    pub adjustment_reason: Option<CodeableConcept>,
    pub date: Option<Date>,
    pub amount: Option<Money>,
    pub identifier: Option<Identifier>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExplanationOfBenefitBenefitBalance {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: CodeableConcept,
    pub excluded: Option<Boolean>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub network: Option<CodeableConcept>,
    pub unit: Option<CodeableConcept>,
    pub term: Option<CodeableConcept>,
    pub financial: Option<Vec<ExplanationOfBenefitBenefitBalanceFinancial>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FamilyMemberHistoryBorn {
    Period(Period),
    Date(Date),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FamilyMemberHistoryAge {
    Age(Age),
    Range(Range),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FamilyMemberHistoryDeceased {
    Boolean(Boolean),
    Age(Age),
    Range(Range),
    Date(Date),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FamilyMemberHistory {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "instantiatesCanonical")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[serde(rename = "instantiatesUri")]
    pub instantiates_uri: Option<Vec<Uri>>,
    pub status: Code,
    #[serde(rename = "dataAbsentReason")]
    pub data_absent_reason: Option<CodeableConcept>,
    pub patient: Reference,
    pub date: Option<DateTime>,
    pub name: Option<String>,
    pub relationship: CodeableConcept,
    pub sex: Option<CodeableConcept>,
    #[serde(rename = "bornPeriod")]
    pub born_period: Option<Period>,
    #[serde(rename = "bornDate")]
    pub born_date: Option<Date>,
    #[serde(rename = "bornString")]
    pub born_string: Option<String>,
    #[serde(rename = "ageAge")]
    pub age_age: Option<Age>,
    #[serde(rename = "ageRange")]
    pub age_range: Option<Range>,
    #[serde(rename = "ageString")]
    pub age_string: Option<String>,
    #[serde(rename = "estimatedAge")]
    pub estimated_age: Option<Boolean>,
    #[serde(rename = "deceasedBoolean")]
    pub deceased_boolean: Option<Boolean>,
    #[serde(rename = "deceasedAge")]
    pub deceased_age: Option<Age>,
    #[serde(rename = "deceasedRange")]
    pub deceased_range: Option<Range>,
    #[serde(rename = "deceasedDate")]
    pub deceased_date: Option<Date>,
    #[serde(rename = "deceasedString")]
    pub deceased_string: Option<String>,
    #[serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    pub condition: Option<Vec<FamilyMemberHistoryCondition>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FamilyMemberHistoryConditionOnset {
    Age(Age),
    Range(Range),
    Period(Period),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FamilyMemberHistoryCondition {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    pub outcome: Option<CodeableConcept>,
    #[serde(rename = "contributedToDeath")]
    pub contributed_to_death: Option<Boolean>,
    #[serde(rename = "onsetAge")]
    pub onset_age: Option<Age>,
    #[serde(rename = "onsetRange")]
    pub onset_range: Option<Range>,
    #[serde(rename = "onsetPeriod")]
    pub onset_period: Option<Period>,
    #[serde(rename = "onsetString")]
    pub onset_string: Option<String>,
    pub note: Option<Vec<Annotation>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct Flag {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub category: Option<Vec<CodeableConcept>>,
    pub code: CodeableConcept,
    pub subject: Reference,
    pub period: Option<Period>,
    pub encounter: Option<Reference>,
    pub author: Option<Reference>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GoalStart {
    Date(Date),
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Goal {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "lifecycleStatus")]
    pub lifecycle_status: Code,
    #[serde(rename = "achievementStatus")]
    pub achievement_status: Option<CodeableConcept>,
    pub category: Option<Vec<CodeableConcept>>,
    pub priority: Option<CodeableConcept>,
    pub description: CodeableConcept,
    pub subject: Reference,
    #[serde(rename = "startDate")]
    pub start_date: Option<Date>,
    #[serde(rename = "startCodeableConcept")]
    pub start_codeable_concept: Option<CodeableConcept>,
    pub target: Option<Vec<GoalTarget>>,
    #[serde(rename = "statusDate")]
    pub status_date: Option<Date>,
    #[serde(rename = "statusReason")]
    pub status_reason: Option<String>,
    #[serde(rename = "expressedBy")]
    pub expressed_by: Option<Reference>,
    pub addresses: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "outcomeCode")]
    pub outcome_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "outcomeReference")]
    pub outcome_reference: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GoalTargetDetail {
    Quantity(Quantity),
    Range(Range),
    CodeableConcept(CodeableConcept),
    String(String),
    Boolean(Boolean),
    Integer(Integer),
    Ratio(Ratio),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GoalTargetDue {
    Date(Date),
    Duration(Duration),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GoalTarget {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub measure: Option<CodeableConcept>,
    #[serde(rename = "detailQuantity")]
    pub detail_quantity: Option<Quantity>,
    #[serde(rename = "detailRange")]
    pub detail_range: Option<Range>,
    #[serde(rename = "detailCodeableConcept")]
    pub detail_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "detailString")]
    pub detail_string: Option<String>,
    #[serde(rename = "detailBoolean")]
    pub detail_boolean: Option<Boolean>,
    #[serde(rename = "detailInteger")]
    pub detail_integer: Option<Integer>,
    #[serde(rename = "detailRatio")]
    pub detail_ratio: Option<Ratio>,
    #[serde(rename = "dueDate")]
    pub due_date: Option<Date>,
    #[serde(rename = "dueDuration")]
    pub due_duration: Option<Duration>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct GraphDefinitionLink {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub path: Option<String>,
    #[serde(rename = "sliceName")]
    pub slice_name: Option<String>,
    pub min: Option<Integer>,
    pub max: Option<String>,
    pub description: Option<String>,
    pub target: Option<Vec<GraphDefinitionLinkTarget>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GraphDefinition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub version: Option<String>,
    pub name: String,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub start: Code,
    pub profile: Option<Canonical>,
    pub link: Option<Vec<GraphDefinitionLink>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GraphDefinitionLinkTargetCompartment {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "use")]
    pub r#use: Code,
    pub code: Code,
    pub rule: Code,
    pub expression: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GraphDefinitionLinkTarget {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub params: Option<String>,
    pub profile: Option<Canonical>,
    pub compartment: Option<Vec<GraphDefinitionLinkTargetCompartment>>,
    pub link: Option<Vec<GraphDefinitionLink>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GroupCharacteristicValue {
    CodeableConcept(CodeableConcept),
    Boolean(Boolean),
    Quantity(Quantity),
    Range(Range),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GroupCharacteristic {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    #[serde(rename = "valueCodeableConcept")]
    pub value_codeable_concept: CodeableConcept,
    #[serde(rename = "valueBoolean")]
    pub value_boolean: Boolean,
    #[serde(rename = "valueQuantity")]
    pub value_quantity: Quantity,
    #[serde(rename = "valueRange")]
    pub value_range: Range,
    #[serde(rename = "valueReference")]
    pub value_reference: Reference,
    pub exclude: Boolean,
    pub period: Option<Period>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GroupMember {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub entity: Reference,
    pub period: Option<Period>,
    pub inactive: Option<Boolean>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Group {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub active: Option<Boolean>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub actual: Boolean,
    pub code: Option<CodeableConcept>,
    pub name: Option<String>,
    pub quantity: Option<UnsignedInt>,
    #[serde(rename = "managingEntity")]
    pub managing_entity: Option<Reference>,
    pub characteristic: Option<Vec<GroupCharacteristic>>,
    pub member: Option<Vec<GroupMember>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GuidanceResponseModule {
    Uri(Uri),
    Canonical(Canonical),
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GuidanceResponse {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "requestIdentifier")]
    pub request_identifier: Option<Identifier>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "moduleUri")]
    pub module_uri: Uri,
    #[serde(rename = "moduleCanonical")]
    pub module_canonical: Canonical,
    #[serde(rename = "moduleCodeableConcept")]
    pub module_codeable_concept: CodeableConcept,
    pub status: Code,
    pub subject: Option<Reference>,
    pub encounter: Option<Reference>,
    #[serde(rename = "occurrenceDateTime")]
    pub occurrence_date_time: Option<DateTime>,
    pub performer: Option<Reference>,
    #[serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "evaluationMessage")]
    pub evaluation_message: Option<Vec<Reference>>,
    #[serde(rename = "outputParameters")]
    pub output_parameters: Option<Reference>,
    pub result: Option<Reference>,
    #[serde(rename = "dataRequirement")]
    pub data_requirement: Option<Vec<DataRequirement>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct HealthcareServiceEligibility {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub comment: Option<Markdown>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HealthcareServiceAvailableTime {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "daysOfWeek")]
    pub days_of_week: Option<Vec<Code>>,
    #[serde(rename = "allDay")]
    pub all_day: Option<Boolean>,
    #[serde(rename = "availableStartTime")]
    pub available_start_time: Option<Time>,
    #[serde(rename = "availableEndTime")]
    pub available_end_time: Option<Time>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HealthcareService {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub active: Option<Boolean>,
    #[serde(rename = "providedBy")]
    pub provided_by: Option<Reference>,
    pub category: Option<Vec<CodeableConcept>>,
    #[serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    pub specialty: Option<Vec<CodeableConcept>>,
    pub location: Option<Vec<Reference>>,
    pub name: Option<String>,
    pub comment: Option<String>,
    #[serde(rename = "extraDetails")]
    pub extra_details: Option<Markdown>,
    pub photo: Option<Attachment>,
    pub telecom: Option<Vec<ContactPoint>>,
    #[serde(rename = "coverageArea")]
    pub coverage_area: Option<Vec<Reference>>,
    #[serde(rename = "serviceProvisionCode")]
    pub service_provision_code: Option<Vec<CodeableConcept>>,
    pub eligibility: Option<Vec<HealthcareServiceEligibility>>,
    pub program: Option<Vec<CodeableConcept>>,
    pub characteristic: Option<Vec<CodeableConcept>>,
    pub communication: Option<Vec<CodeableConcept>>,
    #[serde(rename = "referralMethod")]
    pub referral_method: Option<Vec<CodeableConcept>>,
    #[serde(rename = "appointmentRequired")]
    pub appointment_required: Option<Boolean>,
    #[serde(rename = "availableTime")]
    pub available_time: Option<Vec<HealthcareServiceAvailableTime>>,
    #[serde(rename = "notAvailable")]
    pub not_available: Option<Vec<HealthcareServiceNotAvailable>>,
    #[serde(rename = "availabilityExceptions")]
    pub availability_exceptions: Option<String>,
    pub endpoint: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HealthcareServiceNotAvailable {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: String,
    pub during: Option<Period>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct ImagingStudySeriesPerformer {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImagingStudySeries {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub uid: Id,
    pub number: Option<UnsignedInt>,
    pub modality: Coding,
    pub description: Option<String>,
    #[serde(rename = "numberOfInstances")]
    pub number_of_instances: Option<UnsignedInt>,
    pub endpoint: Option<Vec<Reference>>,
    #[serde(rename = "bodySite")]
    pub body_site: Option<Coding>,
    pub laterality: Option<Coding>,
    pub specimen: Option<Vec<Reference>>,
    pub started: Option<DateTime>,
    pub performer: Option<Vec<ImagingStudySeriesPerformer>>,
    pub instance: Option<Vec<ImagingStudySeriesInstance>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImagingStudy {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub modality: Option<Vec<Coding>>,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    pub started: Option<DateTime>,
    #[serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    pub referrer: Option<Reference>,
    pub interpreter: Option<Vec<Reference>>,
    pub endpoint: Option<Vec<Reference>>,
    #[serde(rename = "numberOfSeries")]
    pub number_of_series: Option<UnsignedInt>,
    #[serde(rename = "numberOfInstances")]
    pub number_of_instances: Option<UnsignedInt>,
    #[serde(rename = "procedureReference")]
    pub procedure_reference: Option<Reference>,
    #[serde(rename = "procedureCode")]
    pub procedure_code: Option<Vec<CodeableConcept>>,
    pub location: Option<Reference>,
    #[serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    pub description: Option<String>,
    pub series: Option<Vec<ImagingStudySeries>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImagingStudySeriesInstance {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub uid: Id,
    #[serde(rename = "sopClass")]
    pub sop_class: Coding,
    pub number: Option<UnsignedInt>,
    pub title: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImmunizationProtocolAppliedDoseNumber {
    PositiveInt(PositiveInt),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImmunizationProtocolAppliedSeriesDoses {
    PositiveInt(PositiveInt),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImmunizationProtocolApplied {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub series: Option<String>,
    pub authority: Option<Reference>,
    #[serde(rename = "targetDisease")]
    pub target_disease: Option<Vec<CodeableConcept>>,
    #[serde(rename = "doseNumberPositiveInt")]
    pub dose_number_positive_int: PositiveInt,
    #[serde(rename = "doseNumberString")]
    pub dose_number_string: String,
    #[serde(rename = "seriesDosesPositiveInt")]
    pub series_doses_positive_int: Option<PositiveInt>,
    #[serde(rename = "seriesDosesString")]
    pub series_doses_string: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImmunizationOccurrence {
    DateTime(DateTime),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Immunization {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "statusReason")]
    pub status_reason: Option<CodeableConcept>,
    #[serde(rename = "vaccineCode")]
    pub vaccine_code: CodeableConcept,
    pub patient: Reference,
    pub encounter: Option<Reference>,
    #[serde(rename = "occurrenceDateTime")]
    pub occurrence_date_time: DateTime,
    #[serde(rename = "occurrenceString")]
    pub occurrence_string: String,
    pub recorded: Option<DateTime>,
    #[serde(rename = "primarySource")]
    pub primary_source: Option<Boolean>,
    #[serde(rename = "reportOrigin")]
    pub report_origin: Option<CodeableConcept>,
    pub location: Option<Reference>,
    pub manufacturer: Option<Reference>,
    #[serde(rename = "lotNumber")]
    pub lot_number: Option<String>,
    #[serde(rename = "expirationDate")]
    pub expiration_date: Option<Date>,
    pub site: Option<CodeableConcept>,
    pub route: Option<CodeableConcept>,
    #[serde(rename = "doseQuantity")]
    pub dose_quantity: Option<Quantity>,
    pub performer: Option<Vec<ImmunizationPerformer>>,
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(rename = "isSubpotent")]
    pub is_subpotent: Option<Boolean>,
    #[serde(rename = "subpotentReason")]
    pub subpotent_reason: Option<Vec<CodeableConcept>>,
    pub education: Option<Vec<ImmunizationEducation>>,
    #[serde(rename = "programEligibility")]
    pub program_eligibility: Option<Vec<CodeableConcept>>,
    #[serde(rename = "fundingSource")]
    pub funding_source: Option<CodeableConcept>,
    pub reaction: Option<Vec<ImmunizationReaction>>,
    #[serde(rename = "protocolApplied")]
    pub protocol_applied: Option<Vec<ImmunizationProtocolApplied>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImmunizationPerformer {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImmunizationReaction {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub date: Option<DateTime>,
    pub detail: Option<Reference>,
    pub reported: Option<Boolean>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImmunizationEducation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "documentType")]
    pub document_type: Option<String>,
    pub reference: Option<Uri>,
    #[serde(rename = "publicationDate")]
    pub publication_date: Option<DateTime>,
    #[serde(rename = "presentationDate")]
    pub presentation_date: Option<DateTime>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImmunizationEvaluationDoseNumber {
    PositiveInt(PositiveInt),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImmunizationEvaluationSeriesDoses {
    PositiveInt(PositiveInt),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImmunizationEvaluation {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub patient: Reference,
    pub date: Option<DateTime>,
    pub authority: Option<Reference>,
    #[serde(rename = "targetDisease")]
    pub target_disease: CodeableConcept,
    #[serde(rename = "immunizationEvent")]
    pub immunization_event: Reference,
    #[serde(rename = "doseStatus")]
    pub dose_status: CodeableConcept,
    #[serde(rename = "doseStatusReason")]
    pub dose_status_reason: Option<Vec<CodeableConcept>>,
    pub description: Option<String>,
    pub series: Option<String>,
    #[serde(rename = "doseNumberPositiveInt")]
    pub dose_number_positive_int: Option<PositiveInt>,
    #[serde(rename = "doseNumberString")]
    pub dose_number_string: Option<String>,
    #[serde(rename = "seriesDosesPositiveInt")]
    pub series_doses_positive_int: Option<PositiveInt>,
    #[serde(rename = "seriesDosesString")]
    pub series_doses_string: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct ImmunizationRecommendation {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub patient: Reference,
    pub date: DateTime,
    pub authority: Option<Reference>,
    pub recommendation: Option<Vec<ImmunizationRecommendationRecommendation>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImmunizationRecommendationRecommendationDateCriterion {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    pub value: DateTime,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImmunizationRecommendationRecommendationDoseNumber {
    PositiveInt(PositiveInt),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImmunizationRecommendationRecommendationSeriesDoses {
    PositiveInt(PositiveInt),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImmunizationRecommendationRecommendation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "vaccineCode")]
    pub vaccine_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "targetDisease")]
    pub target_disease: Option<CodeableConcept>,
    #[serde(rename = "contraindicatedVaccineCode")]
    pub contraindicated_vaccine_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "forecastStatus")]
    pub forecast_status: CodeableConcept,
    #[serde(rename = "forecastReason")]
    pub forecast_reason: Option<Vec<CodeableConcept>>,
    #[serde(rename = "dateCriterion")]
    pub date_criterion: Option<Vec<ImmunizationRecommendationRecommendationDateCriterion>>,
    pub description: Option<String>,
    pub series: Option<String>,
    #[serde(rename = "doseNumberPositiveInt")]
    pub dose_number_positive_int: Option<PositiveInt>,
    #[serde(rename = "doseNumberString")]
    pub dose_number_string: Option<String>,
    #[serde(rename = "seriesDosesPositiveInt")]
    pub series_doses_positive_int: Option<PositiveInt>,
    #[serde(rename = "seriesDosesString")]
    pub series_doses_string: Option<String>,
    #[serde(rename = "supportingImmunization")]
    pub supporting_immunization: Option<Vec<Reference>>,
    #[serde(rename = "supportingPatientInformation")]
    pub supporting_patient_information: Option<Vec<Reference>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImplementationGuideDefinitionPageName {
    Url(Url),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImplementationGuideDefinitionPage {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "nameUrl")]
    pub name_url: Url,
    #[serde(rename = "nameReference")]
    pub name_reference: Reference,
    pub title: String,
    pub generation: Code,
    pub page: Option<Vec<ImplementationGuideDefinitionPage>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImplementationGuideDefinitionParameter {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImplementationGuideDefinition {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub grouping: Option<Vec<ImplementationGuideDefinitionGrouping>>,
    pub resource: Option<Vec<ImplementationGuideDefinitionResource>>,
    pub page: Option<ImplementationGuideDefinitionPage>,
    pub parameter: Option<Vec<ImplementationGuideDefinitionParameter>>,
    pub template: Option<Vec<ImplementationGuideDefinitionTemplate>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImplementationGuideManifest {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub rendering: Option<Url>,
    pub resource: Option<Vec<ImplementationGuideManifestResource>>,
    pub page: Option<Vec<ImplementationGuideManifestPage>>,
    pub image: Option<Vec<String>>,
    pub other: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImplementationGuideManifestPage {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub title: Option<String>,
    pub anchor: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImplementationGuideDefinitionTemplate {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub source: String,
    pub scope: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImplementationGuideManifestResourceExample {
    Boolean(Boolean),
    Canonical(Canonical),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImplementationGuideManifestResource {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub reference: Reference,
    #[serde(rename = "exampleBoolean")]
    pub example_boolean: Option<Boolean>,
    #[serde(rename = "exampleCanonical")]
    pub example_canonical: Option<Canonical>,
    #[serde(rename = "relativePath")]
    pub relative_path: Option<Url>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImplementationGuideDependsOn {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub uri: Canonical,
    #[serde(rename = "packageId")]
    pub package_id: Option<Id>,
    pub version: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImplementationGuide {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Uri,
    pub version: Option<String>,
    pub name: String,
    pub title: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub copyright: Option<Markdown>,
    #[serde(rename = "packageId")]
    pub package_id: Id,
    pub license: Option<Code>,
    #[serde(rename = "fhirVersion")]
    pub fhir_version: Option<Vec<Code>>,
    #[serde(rename = "dependsOn")]
    pub depends_on: Option<Vec<ImplementationGuideDependsOn>>,
    pub global: Option<Vec<ImplementationGuideGlobal>>,
    pub definition: Option<ImplementationGuideDefinition>,
    pub manifest: Option<ImplementationGuideManifest>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImplementationGuideDefinitionResourceExample {
    Boolean(Boolean),
    Canonical(Canonical),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImplementationGuideDefinitionResource {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub reference: Reference,
    #[serde(rename = "fhirVersion")]
    pub fhir_version: Option<Vec<Code>>,
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "exampleBoolean")]
    pub example_boolean: Option<Boolean>,
    #[serde(rename = "exampleCanonical")]
    pub example_canonical: Option<Canonical>,
    #[serde(rename = "groupingId")]
    pub grouping_id: Option<Id>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImplementationGuideGlobal {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub profile: Canonical,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImplementationGuideDefinitionGrouping {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub description: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct IngredientManufacturer {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub role: Option<Code>,
    pub manufacturer: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IngredientSubstanceStrengthReferenceStrengthStrength {
    Ratio(Ratio),
    RatioRange(RatioRange),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IngredientSubstanceStrengthReferenceStrength {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub substance: Option<CodeableReference>,
    #[serde(rename = "strengthRatio")]
    pub strength_ratio: Ratio,
    #[serde(rename = "strengthRatioRange")]
    pub strength_ratio_range: RatioRange,
    #[serde(rename = "measurementPoint")]
    pub measurement_point: Option<String>,
    pub country: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IngredientSubstance {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableReference,
    pub strength: Option<Vec<IngredientSubstanceStrength>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Ingredient {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    pub status: Code,
    #[serde(rename = "for")]
    pub r#for: Option<Vec<Reference>>,
    pub role: CodeableConcept,
    pub function: Option<Vec<CodeableConcept>>,
    #[serde(rename = "allergenicIndicator")]
    pub allergenic_indicator: Option<Boolean>,
    pub manufacturer: Option<Vec<IngredientManufacturer>>,
    pub substance: IngredientSubstance,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IngredientSubstanceStrengthPresentation {
    Ratio(Ratio),
    RatioRange(RatioRange),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IngredientSubstanceStrengthConcentration {
    Ratio(Ratio),
    RatioRange(RatioRange),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IngredientSubstanceStrength {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "presentationRatio")]
    pub presentation_ratio: Option<Ratio>,
    #[serde(rename = "presentationRatioRange")]
    pub presentation_ratio_range: Option<RatioRange>,
    #[serde(rename = "textPresentation")]
    pub text_presentation: Option<String>,
    #[serde(rename = "concentrationRatio")]
    pub concentration_ratio: Option<Ratio>,
    #[serde(rename = "concentrationRatioRange")]
    pub concentration_ratio_range: Option<RatioRange>,
    #[serde(rename = "textConcentration")]
    pub text_concentration: Option<String>,
    #[serde(rename = "measurementPoint")]
    pub measurement_point: Option<String>,
    pub country: Option<Vec<CodeableConcept>>,
    #[serde(rename = "referenceStrength")]
    pub reference_strength: Option<Vec<IngredientSubstanceStrengthReferenceStrength>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct InsurancePlanCoverageBenefit {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub requirement: Option<String>,
    pub limit: Option<Vec<InsurancePlanCoverageBenefitLimit>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InsurancePlanPlanGeneralCost {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "groupSize")]
    pub group_size: Option<PositiveInt>,
    pub cost: Option<Money>,
    pub comment: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InsurancePlanPlan {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "coverageArea")]
    pub coverage_area: Option<Vec<Reference>>,
    pub network: Option<Vec<Reference>>,
    #[serde(rename = "generalCost")]
    pub general_cost: Option<Vec<InsurancePlanPlanGeneralCost>>,
    #[serde(rename = "specificCost")]
    pub specific_cost: Option<Vec<InsurancePlanPlanSpecificCost>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InsurancePlanPlanSpecificCostBenefit {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub cost: Option<Vec<InsurancePlanPlanSpecificCostBenefitCost>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InsurancePlanPlanSpecificCostBenefitCost {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub applicability: Option<CodeableConcept>,
    pub qualifiers: Option<Vec<CodeableConcept>>,
    pub value: Option<Quantity>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InsurancePlan {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Option<Code>,
    #[serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    pub name: Option<String>,
    pub alias: Option<Vec<String>>,
    pub period: Option<Period>,
    #[serde(rename = "ownedBy")]
    pub owned_by: Option<Reference>,
    #[serde(rename = "administeredBy")]
    pub administered_by: Option<Reference>,
    #[serde(rename = "coverageArea")]
    pub coverage_area: Option<Vec<Reference>>,
    pub contact: Option<Vec<InsurancePlanContact>>,
    pub endpoint: Option<Vec<Reference>>,
    pub network: Option<Vec<Reference>>,
    pub coverage: Option<Vec<InsurancePlanCoverage>>,
    pub plan: Option<Vec<InsurancePlanPlan>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InsurancePlanCoverageBenefitLimit {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub value: Option<Quantity>,
    pub code: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InsurancePlanPlanSpecificCost {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: CodeableConcept,
    pub benefit: Option<Vec<InsurancePlanPlanSpecificCostBenefit>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InsurancePlanContact {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub purpose: Option<CodeableConcept>,
    pub name: Option<HumanName>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub address: Option<Address>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InsurancePlanCoverage {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub network: Option<Vec<Reference>>,
    pub benefit: Option<Vec<InsurancePlanCoverageBenefit>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InvoiceLineItemChargeItem {
    Reference(Reference),
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InvoiceLineItem {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: Option<PositiveInt>,
    #[serde(rename = "chargeItemReference")]
    pub charge_item_reference: Reference,
    #[serde(rename = "chargeItemCodeableConcept")]
    pub charge_item_codeable_concept: CodeableConcept,
    #[serde(rename = "priceComponent")]
    pub price_component: Option<Vec<InvoiceLineItemPriceComponent>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Invoice {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "cancelledReason")]
    pub cancelled_reason: Option<String>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub subject: Option<Reference>,
    pub recipient: Option<Reference>,
    pub date: Option<DateTime>,
    pub participant: Option<Vec<InvoiceParticipant>>,
    pub issuer: Option<Reference>,
    pub account: Option<Reference>,
    #[serde(rename = "lineItem")]
    pub line_item: Option<Vec<InvoiceLineItem>>,
    #[serde(rename = "totalPriceComponent")]
    pub total_price_component: Option<Vec<InvoiceLineItemPriceComponent>>,
    #[serde(rename = "totalNet")]
    pub total_net: Option<Money>,
    #[serde(rename = "totalGross")]
    pub total_gross: Option<Money>,
    #[serde(rename = "paymentTerms")]
    pub payment_terms: Option<Markdown>,
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InvoiceParticipant {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub role: Option<CodeableConcept>,
    pub actor: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InvoiceLineItemPriceComponent {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub code: Option<CodeableConcept>,
    pub factor: Option<Decimal>,
    pub amount: Option<Money>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LibrarySubject {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Library {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(rename = "subjectCodeableConcept")]
    pub subject_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "subjectReference")]
    pub subject_reference: Option<Reference>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub usage: Option<String>,
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod")]
    pub effective_period: Option<Period>,
    pub topic: Option<Vec<CodeableConcept>>,
    pub author: Option<Vec<ContactDetail>>,
    pub editor: Option<Vec<ContactDetail>>,
    pub reviewer: Option<Vec<ContactDetail>>,
    pub endorser: Option<Vec<ContactDetail>>,
    #[serde(rename = "relatedArtifact")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    pub parameter: Option<Vec<ParameterDefinition>>,
    #[serde(rename = "dataRequirement")]
    pub data_requirement: Option<Vec<DataRequirement>>,
    pub content: Option<Vec<Attachment>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct Linkage {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub active: Option<Boolean>,
    pub author: Option<Reference>,
    pub item: Option<Vec<LinkageItem>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LinkageItem {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub resource: Reference,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct List {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub mode: Code,
    pub title: Option<String>,
    pub code: Option<CodeableConcept>,
    pub subject: Option<Reference>,
    pub encounter: Option<Reference>,
    pub date: Option<DateTime>,
    pub source: Option<Reference>,
    #[serde(rename = "orderedBy")]
    pub ordered_by: Option<CodeableConcept>,
    pub note: Option<Vec<Annotation>>,
    pub entry: Option<Vec<ListEntry>>,
    #[serde(rename = "emptyReason")]
    pub empty_reason: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListEntry {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub flag: Option<CodeableConcept>,
    pub deleted: Option<Boolean>,
    pub date: Option<DateTime>,
    pub item: Reference,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct LocationHoursOfOperation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "daysOfWeek")]
    pub days_of_week: Option<Vec<Code>>,
    #[serde(rename = "allDay")]
    pub all_day: Option<Boolean>,
    #[serde(rename = "openingTime")]
    pub opening_time: Option<Time>,
    #[serde(rename = "closingTime")]
    pub closing_time: Option<Time>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Location {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Option<Code>,
    #[serde(rename = "operationalStatus")]
    pub operational_status: Option<Coding>,
    pub name: Option<String>,
    pub alias: Option<Vec<String>>,
    pub description: Option<String>,
    pub mode: Option<Code>,
    #[serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub address: Option<Address>,
    #[serde(rename = "physicalType")]
    pub physical_type: Option<CodeableConcept>,
    pub position: Option<LocationPosition>,
    #[serde(rename = "managingOrganization")]
    pub managing_organization: Option<Reference>,
    #[serde(rename = "partOf")]
    pub part_of: Option<Reference>,
    #[serde(rename = "hoursOfOperation")]
    pub hours_of_operation: Option<Vec<LocationHoursOfOperation>>,
    #[serde(rename = "availabilityExceptions")]
    pub availability_exceptions: Option<String>,
    pub endpoint: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LocationPosition {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub longitude: Decimal,
    pub latitude: Decimal,
    pub altitude: Option<Decimal>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct ManufacturedItemDefinition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "manufacturedDoseForm")]
    pub manufactured_dose_form: CodeableConcept,
    #[serde(rename = "unitOfPresentation")]
    pub unit_of_presentation: Option<CodeableConcept>,
    pub manufacturer: Option<Vec<Reference>>,
    pub ingredient: Option<Vec<CodeableConcept>>,
    pub property: Option<Vec<ManufacturedItemDefinitionProperty>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ManufacturedItemDefinitionPropertyValue {
    CodeableConcept(CodeableConcept),
    Quantity(Quantity),
    Date(Date),
    Boolean(Boolean),
    Attachment(Attachment),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ManufacturedItemDefinitionProperty {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(rename = "valueCodeableConcept")]
    pub value_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "valueQuantity")]
    pub value_quantity: Option<Quantity>,
    #[serde(rename = "valueDate")]
    pub value_date: Option<Date>,
    #[serde(rename = "valueBoolean")]
    pub value_boolean: Option<Boolean>,
    #[serde(rename = "valueAttachment")]
    pub value_attachment: Option<Attachment>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct MeasureGroupStratifierComponent {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub description: Option<String>,
    pub criteria: Expression,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MeasureGroupStratifier {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub description: Option<String>,
    pub criteria: Option<Expression>,
    pub component: Option<Vec<MeasureGroupStratifierComponent>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MeasureSupplementalData {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub usage: Option<Vec<CodeableConcept>>,
    pub description: Option<String>,
    pub criteria: Expression,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MeasureGroup {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub description: Option<String>,
    pub population: Option<Vec<MeasureGroupPopulation>>,
    pub stratifier: Option<Vec<MeasureGroupStratifier>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MeasureGroupPopulation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub description: Option<String>,
    pub criteria: Expression,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MeasureSubject {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Measure {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    #[serde(rename = "subjectCodeableConcept")]
    pub subject_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "subjectReference")]
    pub subject_reference: Option<Reference>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub usage: Option<String>,
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod")]
    pub effective_period: Option<Period>,
    pub topic: Option<Vec<CodeableConcept>>,
    pub author: Option<Vec<ContactDetail>>,
    pub editor: Option<Vec<ContactDetail>>,
    pub reviewer: Option<Vec<ContactDetail>>,
    pub endorser: Option<Vec<ContactDetail>>,
    #[serde(rename = "relatedArtifact")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    pub library: Option<Vec<Canonical>>,
    pub disclaimer: Option<Markdown>,
    pub scoring: Option<CodeableConcept>,
    #[serde(rename = "compositeScoring")]
    pub composite_scoring: Option<CodeableConcept>,
    #[serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(rename = "riskAdjustment")]
    pub risk_adjustment: Option<String>,
    #[serde(rename = "rateAggregation")]
    pub rate_aggregation: Option<String>,
    pub rationale: Option<Markdown>,
    #[serde(rename = "clinicalRecommendationStatement")]
    pub clinical_recommendation_statement: Option<Markdown>,
    #[serde(rename = "improvementNotation")]
    pub improvement_notation: Option<CodeableConcept>,
    pub definition: Option<Vec<Markdown>>,
    pub guidance: Option<Markdown>,
    pub group: Option<Vec<MeasureGroup>>,
    #[serde(rename = "supplementalData")]
    pub supplemental_data: Option<Vec<MeasureSupplementalData>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct MeasureReportGroupPopulation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub count: Option<Integer>,
    #[serde(rename = "subjectResults")]
    pub subject_results: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MeasureReportGroupStratifierStratumComponent {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    pub value: CodeableConcept,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MeasureReportGroupStratifierStratum {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub value: Option<CodeableConcept>,
    pub component: Option<Vec<MeasureReportGroupStratifierStratumComponent>>,
    pub population: Option<Vec<MeasureReportGroupStratifierStratumPopulation>>,
    #[serde(rename = "measureScore")]
    pub measure_score: Option<Quantity>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MeasureReport {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub measure: Canonical,
    pub subject: Option<Reference>,
    pub date: Option<DateTime>,
    pub reporter: Option<Reference>,
    pub period: Period,
    #[serde(rename = "improvementNotation")]
    pub improvement_notation: Option<CodeableConcept>,
    pub group: Option<Vec<MeasureReportGroup>>,
    #[serde(rename = "evaluatedResource")]
    pub evaluated_resource: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MeasureReportGroup {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub population: Option<Vec<MeasureReportGroupPopulation>>,
    #[serde(rename = "measureScore")]
    pub measure_score: Option<Quantity>,
    pub stratifier: Option<Vec<MeasureReportGroupStratifier>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MeasureReportGroupStratifier {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<Vec<CodeableConcept>>,
    pub stratum: Option<Vec<MeasureReportGroupStratifierStratum>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MeasureReportGroupStratifierStratumPopulation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub count: Option<Integer>,
    #[serde(rename = "subjectResults")]
    pub subject_results: Option<Reference>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MediaCreated {
    DateTime(DateTime),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Media {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(rename = "partOf")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub modality: Option<CodeableConcept>,
    pub view: Option<CodeableConcept>,
    pub subject: Option<Reference>,
    pub encounter: Option<Reference>,
    #[serde(rename = "createdDateTime")]
    pub created_date_time: Option<DateTime>,
    #[serde(rename = "createdPeriod")]
    pub created_period: Option<Period>,
    pub issued: Option<Instant>,
    pub operator: Option<Reference>,
    #[serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "bodySite")]
    pub body_site: Option<CodeableConcept>,
    #[serde(rename = "deviceName")]
    pub device_name: Option<String>,
    pub device: Option<Reference>,
    pub height: Option<PositiveInt>,
    pub width: Option<PositiveInt>,
    pub frames: Option<PositiveInt>,
    pub duration: Option<Decimal>,
    pub content: Attachment,
    pub note: Option<Vec<Annotation>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationIngredientItem {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MedicationIngredient {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "itemCodeableConcept")]
    pub item_codeable_concept: CodeableConcept,
    #[serde(rename = "itemReference")]
    pub item_reference: Reference,
    #[serde(rename = "isActive")]
    pub is_active: Option<Boolean>,
    pub strength: Option<Ratio>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Medication {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub code: Option<CodeableConcept>,
    pub status: Option<Code>,
    pub manufacturer: Option<Reference>,
    pub form: Option<CodeableConcept>,
    pub amount: Option<Ratio>,
    pub ingredient: Option<Vec<MedicationIngredient>>,
    pub batch: Option<MedicationBatch>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MedicationBatch {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "lotNumber")]
    pub lot_number: Option<String>,
    #[serde(rename = "expirationDate")]
    pub expiration_date: Option<DateTime>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationAdministrationDosageRate {
    Ratio(Ratio),
    Quantity(Quantity),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MedicationAdministrationDosage {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub text: Option<String>,
    pub site: Option<CodeableConcept>,
    pub route: Option<CodeableConcept>,
    pub method: Option<CodeableConcept>,
    pub dose: Option<Quantity>,
    #[serde(rename = "rateRatio")]
    pub rate_ratio: Option<Ratio>,
    #[serde(rename = "rateQuantity")]
    pub rate_quantity: Option<Quantity>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationAdministrationMedication {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationAdministrationEffective {
    DateTime(DateTime),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MedicationAdministration {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub instantiates: Option<Vec<Uri>>,
    #[serde(rename = "partOf")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    #[serde(rename = "statusReason")]
    pub status_reason: Option<Vec<CodeableConcept>>,
    pub category: Option<CodeableConcept>,
    #[serde(rename = "medicationCodeableConcept")]
    pub medication_codeable_concept: CodeableConcept,
    #[serde(rename = "medicationReference")]
    pub medication_reference: Reference,
    pub subject: Reference,
    pub context: Option<Reference>,
    #[serde(rename = "supportingInformation")]
    pub supporting_information: Option<Vec<Reference>>,
    #[serde(rename = "effectiveDateTime")]
    pub effective_date_time: DateTime,
    #[serde(rename = "effectivePeriod")]
    pub effective_period: Period,
    pub performer: Option<Vec<MedicationAdministrationPerformer>>,
    #[serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    pub request: Option<Reference>,
    pub device: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    pub dosage: Option<MedicationAdministrationDosage>,
    #[serde(rename = "eventHistory")]
    pub event_history: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MedicationAdministrationPerformer {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct MedicationDispensePerformer {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationDispenseStatusReason {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationDispenseMedication {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MedicationDispense {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "partOf")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    #[serde(rename = "statusReasonCodeableConcept")]
    pub status_reason_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "statusReasonReference")]
    pub status_reason_reference: Option<Reference>,
    pub category: Option<CodeableConcept>,
    #[serde(rename = "medicationCodeableConcept")]
    pub medication_codeable_concept: CodeableConcept,
    #[serde(rename = "medicationReference")]
    pub medication_reference: Reference,
    pub subject: Option<Reference>,
    pub context: Option<Reference>,
    #[serde(rename = "supportingInformation")]
    pub supporting_information: Option<Vec<Reference>>,
    pub performer: Option<Vec<MedicationDispensePerformer>>,
    pub location: Option<Reference>,
    #[serde(rename = "authorizingPrescription")]
    pub authorizing_prescription: Option<Vec<Reference>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub quantity: Option<Quantity>,
    #[serde(rename = "daysSupply")]
    pub days_supply: Option<Quantity>,
    #[serde(rename = "whenPrepared")]
    pub when_prepared: Option<DateTime>,
    #[serde(rename = "whenHandedOver")]
    pub when_handed_over: Option<DateTime>,
    pub destination: Option<Reference>,
    pub receiver: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "dosageInstruction")]
    pub dosage_instruction: Option<Vec<Dosage>>,
    pub substitution: Option<MedicationDispenseSubstitution>,
    #[serde(rename = "detectedIssue")]
    pub detected_issue: Option<Vec<Reference>>,
    #[serde(rename = "eventHistory")]
    pub event_history: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MedicationDispenseSubstitution {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "wasSubstituted")]
    pub was_substituted: Boolean,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub reason: Option<Vec<CodeableConcept>>,
    #[serde(rename = "responsibleParty")]
    pub responsible_party: Option<Vec<Reference>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct MedicationKnowledgeMedicineClassification {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub classification: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MedicationKnowledgeMonograph {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub source: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MedicationKnowledgeRelatedMedicationKnowledge {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub reference: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MedicationKnowledgeKinetics {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "areaUnderCurve")]
    pub area_under_curve: Option<Vec<Quantity>>,
    #[serde(rename = "lethalDose50")]
    pub lethal_dose50: Option<Vec<Quantity>>,
    #[serde(rename = "halfLifePeriod")]
    pub half_life_period: Option<Duration>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MedicationKnowledge {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub status: Option<Code>,
    pub manufacturer: Option<Reference>,
    #[serde(rename = "doseForm")]
    pub dose_form: Option<CodeableConcept>,
    pub amount: Option<Quantity>,
    pub synonym: Option<Vec<String>>,
    #[serde(rename = "relatedMedicationKnowledge")]
    pub related_medication_knowledge: Option<Vec<MedicationKnowledgeRelatedMedicationKnowledge>>,
    #[serde(rename = "associatedMedication")]
    pub associated_medication: Option<Vec<Reference>>,
    #[serde(rename = "productType")]
    pub product_type: Option<Vec<CodeableConcept>>,
    pub monograph: Option<Vec<MedicationKnowledgeMonograph>>,
    pub ingredient: Option<Vec<MedicationKnowledgeIngredient>>,
    #[serde(rename = "preparationInstruction")]
    pub preparation_instruction: Option<Markdown>,
    #[serde(rename = "intendedRoute")]
    pub intended_route: Option<Vec<CodeableConcept>>,
    pub cost: Option<Vec<MedicationKnowledgeCost>>,
    #[serde(rename = "monitoringProgram")]
    pub monitoring_program: Option<Vec<MedicationKnowledgeMonitoringProgram>>,
    #[serde(rename = "administrationGuidelines")]
    pub administration_guidelines: Option<Vec<MedicationKnowledgeAdministrationGuidelines>>,
    #[serde(rename = "medicineClassification")]
    pub medicine_classification: Option<Vec<MedicationKnowledgeMedicineClassification>>,
    pub packaging: Option<MedicationKnowledgePackaging>,
    #[serde(rename = "drugCharacteristic")]
    pub drug_characteristic: Option<Vec<MedicationKnowledgeDrugCharacteristic>>,
    pub contraindication: Option<Vec<Reference>>,
    pub regulatory: Option<Vec<MedicationKnowledgeRegulatory>>,
    pub kinetics: Option<Vec<MedicationKnowledgeKinetics>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MedicationKnowledgeRegulatory {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "regulatoryAuthority")]
    pub regulatory_authority: Reference,
    pub substitution: Option<Vec<MedicationKnowledgeRegulatorySubstitution>>,
    pub schedule: Option<Vec<MedicationKnowledgeRegulatorySchedule>>,
    #[serde(rename = "maxDispense")]
    pub max_dispense: Option<MedicationKnowledgeRegulatoryMaxDispense>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MedicationKnowledgeMonitoringProgram {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MedicationKnowledgeAdministrationGuidelinesDosage {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub dosage: Option<Vec<Dosage>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MedicationKnowledgeCost {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub source: Option<String>,
    pub cost: Money,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MedicationKnowledgePackaging {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub quantity: Option<Quantity>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationKnowledgeIngredientItem {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MedicationKnowledgeIngredient {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "itemCodeableConcept")]
    pub item_codeable_concept: CodeableConcept,
    #[serde(rename = "itemReference")]
    pub item_reference: Reference,
    #[serde(rename = "isActive")]
    pub is_active: Option<Boolean>,
    pub strength: Option<Ratio>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationKnowledgeAdministrationGuidelinesIndication {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MedicationKnowledgeAdministrationGuidelines {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub dosage: Option<Vec<MedicationKnowledgeAdministrationGuidelinesDosage>>,
    #[serde(rename = "indicationCodeableConcept")]
    pub indication_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "indicationReference")]
    pub indication_reference: Option<Reference>,
    #[serde(rename = "patientCharacteristics")]
    pub patient_characteristics: Option<Vec<MedicationKnowledgeAdministrationGuidelinesPatientCharacteristics>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationKnowledgeDrugCharacteristicValue {
    CodeableConcept(CodeableConcept),
    String(String),
    Quantity(Quantity),
    Base64Binary(Base64Binary),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MedicationKnowledgeDrugCharacteristic {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "valueCodeableConcept")]
    pub value_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "valueString")]
    pub value_string: Option<String>,
    #[serde(rename = "valueQuantity")]
    pub value_quantity: Option<Quantity>,
    #[serde(rename = "valueBase64Binary")]
    pub value_base64_binary: Option<Base64Binary>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MedicationKnowledgeRegulatorySchedule {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub schedule: CodeableConcept,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationKnowledgeAdministrationGuidelinesPatientCharacteristicsCharacteristic {
    CodeableConcept(CodeableConcept),
    Quantity(Quantity),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MedicationKnowledgeAdministrationGuidelinesPatientCharacteristics {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "characteristicCodeableConcept")]
    pub characteristic_codeable_concept: CodeableConcept,
    #[serde(rename = "characteristicQuantity")]
    pub characteristic_quantity: Quantity,
    pub value: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MedicationKnowledgeRegulatoryMaxDispense {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub quantity: Quantity,
    pub period: Option<Duration>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MedicationKnowledgeRegulatorySubstitution {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub allowed: Boolean,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct MedicationRequestDispenseRequestInitialFill {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub quantity: Option<Quantity>,
    pub duration: Option<Duration>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MedicationRequestDispenseRequest {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "initialFill")]
    pub initial_fill: Option<MedicationRequestDispenseRequestInitialFill>,
    #[serde(rename = "dispenseInterval")]
    pub dispense_interval: Option<Duration>,
    #[serde(rename = "validityPeriod")]
    pub validity_period: Option<Period>,
    #[serde(rename = "numberOfRepeatsAllowed")]
    pub number_of_repeats_allowed: Option<UnsignedInt>,
    pub quantity: Option<Quantity>,
    #[serde(rename = "expectedSupplyDuration")]
    pub expected_supply_duration: Option<Duration>,
    pub performer: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationRequestReported {
    Boolean(Boolean),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationRequestMedication {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MedicationRequest {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    #[serde(rename = "statusReason")]
    pub status_reason: Option<CodeableConcept>,
    pub intent: Code,
    pub category: Option<Vec<CodeableConcept>>,
    pub priority: Option<Code>,
    #[serde(rename = "doNotPerform")]
    pub do_not_perform: Option<Boolean>,
    #[serde(rename = "reportedBoolean")]
    pub reported_boolean: Option<Boolean>,
    #[serde(rename = "reportedReference")]
    pub reported_reference: Option<Reference>,
    #[serde(rename = "medicationCodeableConcept")]
    pub medication_codeable_concept: CodeableConcept,
    #[serde(rename = "medicationReference")]
    pub medication_reference: Reference,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    #[serde(rename = "supportingInformation")]
    pub supporting_information: Option<Vec<Reference>>,
    #[serde(rename = "authoredOn")]
    pub authored_on: Option<DateTime>,
    pub requester: Option<Reference>,
    pub performer: Option<Reference>,
    #[serde(rename = "performerType")]
    pub performer_type: Option<CodeableConcept>,
    pub recorder: Option<Reference>,
    #[serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(rename = "instantiatesCanonical")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[serde(rename = "instantiatesUri")]
    pub instantiates_uri: Option<Vec<Uri>>,
    #[serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(rename = "groupIdentifier")]
    pub group_identifier: Option<Identifier>,
    #[serde(rename = "courseOfTherapyType")]
    pub course_of_therapy_type: Option<CodeableConcept>,
    pub insurance: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "dosageInstruction")]
    pub dosage_instruction: Option<Vec<Dosage>>,
    #[serde(rename = "dispenseRequest")]
    pub dispense_request: Option<MedicationRequestDispenseRequest>,
    pub substitution: Option<MedicationRequestSubstitution>,
    #[serde(rename = "priorPrescription")]
    pub prior_prescription: Option<Reference>,
    #[serde(rename = "detectedIssue")]
    pub detected_issue: Option<Vec<Reference>>,
    #[serde(rename = "eventHistory")]
    pub event_history: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationRequestSubstitutionAllowed {
    Boolean(Boolean),
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MedicationRequestSubstitution {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "allowedBoolean")]
    pub allowed_boolean: Boolean,
    #[serde(rename = "allowedCodeableConcept")]
    pub allowed_codeable_concept: CodeableConcept,
    pub reason: Option<CodeableConcept>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationStatementMedication {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicationStatementEffective {
    DateTime(DateTime),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MedicationStatement {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(rename = "partOf")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    #[serde(rename = "statusReason")]
    pub status_reason: Option<Vec<CodeableConcept>>,
    pub category: Option<CodeableConcept>,
    #[serde(rename = "medicationCodeableConcept")]
    pub medication_codeable_concept: CodeableConcept,
    #[serde(rename = "medicationReference")]
    pub medication_reference: Reference,
    pub subject: Reference,
    pub context: Option<Reference>,
    #[serde(rename = "effectiveDateTime")]
    pub effective_date_time: Option<DateTime>,
    #[serde(rename = "effectivePeriod")]
    pub effective_period: Option<Period>,
    #[serde(rename = "dateAsserted")]
    pub date_asserted: Option<DateTime>,
    #[serde(rename = "informationSource")]
    pub information_source: Option<Reference>,
    #[serde(rename = "derivedFrom")]
    pub derived_from: Option<Vec<Reference>>,
    #[serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    pub dosage: Option<Vec<Dosage>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct MedicinalProductDefinitionOperation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableReference>,
    #[serde(rename = "effectiveDate")]
    pub effective_date: Option<Period>,
    pub organization: Option<Vec<Reference>>,
    #[serde(rename = "confidentialityIndicator")]
    pub confidentiality_indicator: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MedicinalProductDefinition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub domain: Option<CodeableConcept>,
    pub version: Option<String>,
    pub status: Option<CodeableConcept>,
    #[serde(rename = "statusDate")]
    pub status_date: Option<DateTime>,
    pub description: Option<Markdown>,
    #[serde(rename = "combinedPharmaceuticalDoseForm")]
    pub combined_pharmaceutical_dose_form: Option<CodeableConcept>,
    pub route: Option<Vec<CodeableConcept>>,
    pub indication: Option<Markdown>,
    #[serde(rename = "legalStatusOfSupply")]
    pub legal_status_of_supply: Option<CodeableConcept>,
    #[serde(rename = "additionalMonitoringIndicator")]
    pub additional_monitoring_indicator: Option<CodeableConcept>,
    #[serde(rename = "specialMeasures")]
    pub special_measures: Option<Vec<CodeableConcept>>,
    #[serde(rename = "pediatricUseIndicator")]
    pub pediatric_use_indicator: Option<CodeableConcept>,
    pub classification: Option<Vec<CodeableConcept>>,
    #[serde(rename = "marketingStatus")]
    pub marketing_status: Option<Vec<MarketingStatus>>,
    #[serde(rename = "packagedMedicinalProduct")]
    pub packaged_medicinal_product: Option<Vec<CodeableConcept>>,
    pub ingredient: Option<Vec<CodeableConcept>>,
    pub impurity: Option<Vec<CodeableReference>>,
    #[serde(rename = "attachedDocument")]
    pub attached_document: Option<Vec<Reference>>,
    #[serde(rename = "masterFile")]
    pub master_file: Option<Vec<Reference>>,
    pub contact: Option<Vec<MedicinalProductDefinitionContact>>,
    #[serde(rename = "clinicalTrial")]
    pub clinical_trial: Option<Vec<Reference>>,
    pub code: Option<Vec<Coding>>,
    pub name: Option<Vec<MedicinalProductDefinitionName>>,
    #[serde(rename = "crossReference")]
    pub cross_reference: Option<Vec<MedicinalProductDefinitionCrossReference>>,
    pub operation: Option<Vec<MedicinalProductDefinitionOperation>>,
    pub characteristic: Option<Vec<MedicinalProductDefinitionCharacteristic>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MedicinalProductDefinitionCrossReference {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub product: CodeableReference,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MedicinalProductDefinitionNameCountryLanguage {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub country: CodeableConcept,
    pub jurisdiction: Option<CodeableConcept>,
    pub language: CodeableConcept,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MedicinalProductDefinitionContact {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub contact: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MedicinalProductDefinitionName {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "productName")]
    pub product_name: String,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "namePart")]
    pub name_part: Option<Vec<MedicinalProductDefinitionNameNamePart>>,
    #[serde(rename = "countryLanguage")]
    pub country_language: Option<Vec<MedicinalProductDefinitionNameCountryLanguage>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MedicinalProductDefinitionCharacteristicValue {
    CodeableConcept(CodeableConcept),
    Quantity(Quantity),
    Date(Date),
    Boolean(Boolean),
    Attachment(Attachment),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MedicinalProductDefinitionCharacteristic {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(rename = "valueCodeableConcept")]
    pub value_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "valueQuantity")]
    pub value_quantity: Option<Quantity>,
    #[serde(rename = "valueDate")]
    pub value_date: Option<Date>,
    #[serde(rename = "valueBoolean")]
    pub value_boolean: Option<Boolean>,
    #[serde(rename = "valueAttachment")]
    pub value_attachment: Option<Attachment>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MedicinalProductDefinitionNameNamePart {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub part: String,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MessageDefinitionEvent {
    Coding(Coding),
    Uri(Uri),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MessageDefinition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub replaces: Option<Vec<Canonical>>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: DateTime,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    pub base: Option<Canonical>,
    pub parent: Option<Vec<Canonical>>,
    #[serde(rename = "eventCoding")]
    pub event_coding: Coding,
    #[serde(rename = "eventUri")]
    pub event_uri: Uri,
    pub category: Option<Code>,
    pub focus: Option<Vec<MessageDefinitionFocus>>,
    #[serde(rename = "responseRequired")]
    pub response_required: Option<Code>,
    #[serde(rename = "allowedResponse")]
    pub allowed_response: Option<Vec<MessageDefinitionAllowedResponse>>,
    pub graph: Option<Vec<Canonical>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MessageDefinitionAllowedResponse {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub message: Canonical,
    pub situation: Option<Markdown>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MessageDefinitionFocus {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub profile: Option<Canonical>,
    pub min: UnsignedInt,
    pub max: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct MessageHeaderDestination {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Option<String>,
    pub target: Option<Reference>,
    pub endpoint: Url,
    pub receiver: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MessageHeaderEvent {
    Coding(Coding),
    Uri(Uri),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MessageHeader {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "eventCoding")]
    pub event_coding: Coding,
    #[serde(rename = "eventUri")]
    pub event_uri: Uri,
    pub destination: Option<Vec<MessageHeaderDestination>>,
    pub sender: Option<Reference>,
    pub enterer: Option<Reference>,
    pub author: Option<Reference>,
    pub source: MessageHeaderSource,
    pub responsible: Option<Reference>,
    pub reason: Option<CodeableConcept>,
    pub response: Option<MessageHeaderResponse>,
    pub focus: Option<Vec<Reference>>,
    pub definition: Option<Canonical>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MessageHeaderSource {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Option<String>,
    pub software: Option<String>,
    pub version: Option<String>,
    pub contact: Option<ContactPoint>,
    pub endpoint: Url,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MessageHeaderResponse {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Id,
    pub code: Code,
    pub details: Option<Reference>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct MolecularSequenceQuality {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(rename = "standardSequence")]
    pub standard_sequence: Option<CodeableConcept>,
    pub start: Option<Integer>,
    pub end: Option<Integer>,
    pub score: Option<Quantity>,
    pub method: Option<CodeableConcept>,
    #[serde(rename = "truthTP")]
    pub truth_t_p: Option<Decimal>,
    #[serde(rename = "queryTP")]
    pub query_t_p: Option<Decimal>,
    #[serde(rename = "truthFN")]
    pub truth_f_n: Option<Decimal>,
    #[serde(rename = "queryFP")]
    pub query_f_p: Option<Decimal>,
    #[serde(rename = "gtFP")]
    pub gt_f_p: Option<Decimal>,
    pub precision: Option<Decimal>,
    pub recall: Option<Decimal>,
    #[serde(rename = "fScore")]
    pub f_score: Option<Decimal>,
    pub roc: Option<MolecularSequenceQualityRoc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MolecularSequenceReferenceSeq {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub chromosome: Option<CodeableConcept>,
    #[serde(rename = "genomeBuild")]
    pub genome_build: Option<String>,
    pub orientation: Option<Code>,
    #[serde(rename = "referenceSeqId")]
    pub reference_seq_id: Option<CodeableConcept>,
    #[serde(rename = "referenceSeqPointer")]
    pub reference_seq_pointer: Option<Reference>,
    #[serde(rename = "referenceSeqString")]
    pub reference_seq_string: Option<String>,
    pub strand: Option<Code>,
    #[serde(rename = "windowStart")]
    pub window_start: Option<Integer>,
    #[serde(rename = "windowEnd")]
    pub window_end: Option<Integer>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MolecularSequenceRepository {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub url: Option<Uri>,
    pub name: Option<String>,
    #[serde(rename = "datasetId")]
    pub dataset_id: Option<String>,
    #[serde(rename = "variantsetId")]
    pub variantset_id: Option<String>,
    #[serde(rename = "readsetId")]
    pub readset_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MolecularSequenceStructureVariantOuter {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub start: Option<Integer>,
    pub end: Option<Integer>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MolecularSequenceVariant {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub start: Option<Integer>,
    pub end: Option<Integer>,
    #[serde(rename = "observedAllele")]
    pub observed_allele: Option<String>,
    #[serde(rename = "referenceAllele")]
    pub reference_allele: Option<String>,
    pub cigar: Option<String>,
    #[serde(rename = "variantPointer")]
    pub variant_pointer: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MolecularSequence {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "type")]
    pub r#type: Option<Code>,
    #[serde(rename = "coordinateSystem")]
    pub coordinate_system: Integer,
    pub patient: Option<Reference>,
    pub specimen: Option<Reference>,
    pub device: Option<Reference>,
    pub performer: Option<Reference>,
    pub quantity: Option<Quantity>,
    #[serde(rename = "referenceSeq")]
    pub reference_seq: Option<MolecularSequenceReferenceSeq>,
    pub variant: Option<Vec<MolecularSequenceVariant>>,
    #[serde(rename = "observedSeq")]
    pub observed_seq: Option<String>,
    pub quality: Option<Vec<MolecularSequenceQuality>>,
    #[serde(rename = "readCoverage")]
    pub read_coverage: Option<Integer>,
    pub repository: Option<Vec<MolecularSequenceRepository>>,
    pub pointer: Option<Vec<Reference>>,
    #[serde(rename = "structureVariant")]
    pub structure_variant: Option<Vec<MolecularSequenceStructureVariant>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MolecularSequenceQualityRoc {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub score: Option<Vec<Integer>>,
    #[serde(rename = "numTP")]
    pub num_t_p: Option<Vec<Integer>>,
    #[serde(rename = "numFP")]
    pub num_f_p: Option<Vec<Integer>>,
    #[serde(rename = "numFN")]
    pub num_f_n: Option<Vec<Integer>>,
    pub precision: Option<Vec<Decimal>>,
    pub sensitivity: Option<Vec<Decimal>>,
    #[serde(rename = "fMeasure")]
    pub f_measure: Option<Vec<Decimal>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MolecularSequenceStructureVariant {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "variantType")]
    pub variant_type: Option<CodeableConcept>,
    pub exact: Option<Boolean>,
    pub length: Option<Integer>,
    pub outer: Option<MolecularSequenceStructureVariantOuter>,
    pub inner: Option<MolecularSequenceStructureVariantInner>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MolecularSequenceStructureVariantInner {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub start: Option<Integer>,
    pub end: Option<Integer>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct NamingSystem {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub status: Code,
    pub kind: Code,
    pub date: DateTime,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub responsible: Option<String>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub description: Option<Markdown>,
    #[serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub usage: Option<String>,
    #[serde(rename = "uniqueId")]
    pub unique_id: Option<Vec<NamingSystemUniqueId>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NamingSystemUniqueId {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub value: String,
    pub preferred: Option<Boolean>,
    pub comment: Option<String>,
    pub period: Option<Period>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct NutritionOrderOralDietTexture {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub modifier: Option<CodeableConcept>,
    #[serde(rename = "foodType")]
    pub food_type: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NutritionOrderEnteralFormulaAdministrationRate {
    Quantity(Quantity),
    Ratio(Ratio),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NutritionOrderEnteralFormulaAdministration {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub schedule: Option<Timing>,
    pub quantity: Option<Quantity>,
    #[serde(rename = "rateQuantity")]
    pub rate_quantity: Option<Quantity>,
    #[serde(rename = "rateRatio")]
    pub rate_ratio: Option<Ratio>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NutritionOrderSupplement {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "productName")]
    pub product_name: Option<String>,
    pub schedule: Option<Vec<Timing>>,
    pub quantity: Option<Quantity>,
    pub instruction: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NutritionOrder {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "instantiatesCanonical")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[serde(rename = "instantiatesUri")]
    pub instantiates_uri: Option<Vec<Uri>>,
    pub instantiates: Option<Vec<Uri>>,
    pub status: Code,
    pub intent: Code,
    pub patient: Reference,
    pub encounter: Option<Reference>,
    #[serde(rename = "dateTime")]
    pub date_time: DateTime,
    pub orderer: Option<Reference>,
    #[serde(rename = "allergyIntolerance")]
    pub allergy_intolerance: Option<Vec<Reference>>,
    #[serde(rename = "foodPreferenceModifier")]
    pub food_preference_modifier: Option<Vec<CodeableConcept>>,
    #[serde(rename = "excludeFoodModifier")]
    pub exclude_food_modifier: Option<Vec<CodeableConcept>>,
    #[serde(rename = "oralDiet")]
    pub oral_diet: Option<NutritionOrderOralDiet>,
    pub supplement: Option<Vec<NutritionOrderSupplement>>,
    #[serde(rename = "enteralFormula")]
    pub enteral_formula: Option<NutritionOrderEnteralFormula>,
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NutritionOrderOralDietNutrient {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub modifier: Option<CodeableConcept>,
    pub amount: Option<Quantity>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NutritionOrderOralDiet {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    pub schedule: Option<Vec<Timing>>,
    pub nutrient: Option<Vec<NutritionOrderOralDietNutrient>>,
    pub texture: Option<Vec<NutritionOrderOralDietTexture>>,
    #[serde(rename = "fluidConsistencyType")]
    pub fluid_consistency_type: Option<Vec<CodeableConcept>>,
    pub instruction: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NutritionOrderEnteralFormula {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "baseFormulaType")]
    pub base_formula_type: Option<CodeableConcept>,
    #[serde(rename = "baseFormulaProductName")]
    pub base_formula_product_name: Option<String>,
    #[serde(rename = "additiveType")]
    pub additive_type: Option<CodeableConcept>,
    #[serde(rename = "additiveProductName")]
    pub additive_product_name: Option<String>,
    #[serde(rename = "caloricDensity")]
    pub caloric_density: Option<Quantity>,
    #[serde(rename = "routeofAdministration")]
    pub routeof_administration: Option<CodeableConcept>,
    pub administration: Option<Vec<NutritionOrderEnteralFormulaAdministration>>,
    #[serde(rename = "maxVolumeToDeliver")]
    pub max_volume_to_deliver: Option<Quantity>,
    #[serde(rename = "administrationInstruction")]
    pub administration_instruction: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct NutritionProductInstance {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub quantity: Option<Quantity>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "lotNumber")]
    pub lot_number: Option<String>,
    pub expiry: Option<DateTime>,
    #[serde(rename = "useBy")]
    pub use_by: Option<DateTime>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NutritionProductProductCharacteristicValue {
    CodeableConcept(CodeableConcept),
    String(String),
    Quantity(Quantity),
    Base64Binary(Base64Binary),
    Attachment(Attachment),
    Boolean(Boolean),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NutritionProductProductCharacteristic {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(rename = "valueCodeableConcept")]
    pub value_codeable_concept: CodeableConcept,
    #[serde(rename = "valueString")]
    pub value_string: String,
    #[serde(rename = "valueQuantity")]
    pub value_quantity: Quantity,
    #[serde(rename = "valueBase64Binary")]
    pub value_base64_binary: Base64Binary,
    #[serde(rename = "valueAttachment")]
    pub value_attachment: Attachment,
    #[serde(rename = "valueBoolean")]
    pub value_boolean: Boolean,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NutritionProduct {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub status: Code,
    pub category: Option<Vec<CodeableConcept>>,
    pub code: Option<CodeableConcept>,
    pub manufacturer: Option<Vec<Reference>>,
    pub nutrient: Option<Vec<NutritionProductNutrient>>,
    pub ingredient: Option<Vec<NutritionProductIngredient>>,
    #[serde(rename = "knownAllergen")]
    pub known_allergen: Option<Vec<CodeableReference>>,
    #[serde(rename = "productCharacteristic")]
    pub product_characteristic: Option<Vec<NutritionProductProductCharacteristic>>,
    pub instance: Option<NutritionProductInstance>,
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NutritionProductNutrient {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub item: Option<CodeableReference>,
    pub amount: Option<Vec<Ratio>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NutritionProductIngredient {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub item: CodeableReference,
    pub amount: Option<Vec<Ratio>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct ObservationReferenceRange {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub low: Option<Quantity>,
    pub high: Option<Quantity>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "appliesTo")]
    pub applies_to: Option<Vec<CodeableConcept>>,
    pub age: Option<Range>,
    pub text: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ObservationEffective {
    DateTime(DateTime),
    Period(Period),
    Timing(Timing),
    Instant(Instant),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ObservationValue {
    Quantity(Quantity),
    CodeableConcept(CodeableConcept),
    String(String),
    Boolean(Boolean),
    Integer(Integer),
    Range(Range),
    Ratio(Ratio),
    SampledData(SampledData),
    Time(Time),
    DateTime(DateTime),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Observation {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(rename = "partOf")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    pub category: Option<Vec<CodeableConcept>>,
    pub code: CodeableConcept,
    pub subject: Option<Reference>,
    pub focus: Option<Vec<Reference>>,
    pub encounter: Option<Reference>,
    #[serde(rename = "effectiveDateTime")]
    pub effective_date_time: Option<DateTime>,
    #[serde(rename = "effectivePeriod")]
    pub effective_period: Option<Period>,
    #[serde(rename = "effectiveTiming")]
    pub effective_timing: Option<Timing>,
    #[serde(rename = "effectiveInstant")]
    pub effective_instant: Option<Instant>,
    pub issued: Option<Instant>,
    pub performer: Option<Vec<Reference>>,
    #[serde(rename = "valueQuantity")]
    pub value_quantity: Option<Quantity>,
    #[serde(rename = "valueCodeableConcept")]
    pub value_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "valueString")]
    pub value_string: Option<String>,
    #[serde(rename = "valueBoolean")]
    pub value_boolean: Option<Boolean>,
    #[serde(rename = "valueInteger")]
    pub value_integer: Option<Integer>,
    #[serde(rename = "valueRange")]
    pub value_range: Option<Range>,
    #[serde(rename = "valueRatio")]
    pub value_ratio: Option<Ratio>,
    #[serde(rename = "valueSampledData")]
    pub value_sampled_data: Option<SampledData>,
    #[serde(rename = "valueTime")]
    pub value_time: Option<Time>,
    #[serde(rename = "valueDateTime")]
    pub value_date_time: Option<DateTime>,
    #[serde(rename = "valuePeriod")]
    pub value_period: Option<Period>,
    #[serde(rename = "dataAbsentReason")]
    pub data_absent_reason: Option<CodeableConcept>,
    pub interpretation: Option<Vec<CodeableConcept>>,
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "bodySite")]
    pub body_site: Option<CodeableConcept>,
    pub method: Option<CodeableConcept>,
    pub specimen: Option<Reference>,
    pub device: Option<Reference>,
    #[serde(rename = "referenceRange")]
    pub reference_range: Option<Vec<ObservationReferenceRange>>,
    #[serde(rename = "hasMember")]
    pub has_member: Option<Vec<Reference>>,
    #[serde(rename = "derivedFrom")]
    pub derived_from: Option<Vec<Reference>>,
    pub component: Option<Vec<ObservationComponent>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ObservationComponentValue {
    Quantity(Quantity),
    CodeableConcept(CodeableConcept),
    String(String),
    Boolean(Boolean),
    Integer(Integer),
    Range(Range),
    Ratio(Ratio),
    SampledData(SampledData),
    Time(Time),
    DateTime(DateTime),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ObservationComponent {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    #[serde(rename = "valueQuantity")]
    pub value_quantity: Option<Quantity>,
    #[serde(rename = "valueCodeableConcept")]
    pub value_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "valueString")]
    pub value_string: Option<String>,
    #[serde(rename = "valueBoolean")]
    pub value_boolean: Option<Boolean>,
    #[serde(rename = "valueInteger")]
    pub value_integer: Option<Integer>,
    #[serde(rename = "valueRange")]
    pub value_range: Option<Range>,
    #[serde(rename = "valueRatio")]
    pub value_ratio: Option<Ratio>,
    #[serde(rename = "valueSampledData")]
    pub value_sampled_data: Option<SampledData>,
    #[serde(rename = "valueTime")]
    pub value_time: Option<Time>,
    #[serde(rename = "valueDateTime")]
    pub value_date_time: Option<DateTime>,
    #[serde(rename = "valuePeriod")]
    pub value_period: Option<Period>,
    #[serde(rename = "dataAbsentReason")]
    pub data_absent_reason: Option<CodeableConcept>,
    pub interpretation: Option<Vec<CodeableConcept>>,
    #[serde(rename = "referenceRange")]
    pub reference_range: Option<Vec<ObservationReferenceRange>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct ObservationDefinitionQuantitativeDetails {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "customaryUnit")]
    pub customary_unit: Option<CodeableConcept>,
    pub unit: Option<CodeableConcept>,
    #[serde(rename = "conversionFactor")]
    pub conversion_factor: Option<Decimal>,
    #[serde(rename = "decimalPrecision")]
    pub decimal_precision: Option<Integer>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ObservationDefinition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: Option<Vec<CodeableConcept>>,
    pub code: CodeableConcept,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "permittedDataType")]
    pub permitted_data_type: Option<Vec<Code>>,
    #[serde(rename = "multipleResultsAllowed")]
    pub multiple_results_allowed: Option<Boolean>,
    pub method: Option<CodeableConcept>,
    #[serde(rename = "preferredReportName")]
    pub preferred_report_name: Option<String>,
    #[serde(rename = "quantitativeDetails")]
    pub quantitative_details: Option<ObservationDefinitionQuantitativeDetails>,
    #[serde(rename = "qualifiedInterval")]
    pub qualified_interval: Option<Vec<ObservationDefinitionQualifiedInterval>>,
    #[serde(rename = "validCodedValueSet")]
    pub valid_coded_value_set: Option<Reference>,
    #[serde(rename = "normalCodedValueSet")]
    pub normal_coded_value_set: Option<Reference>,
    #[serde(rename = "abnormalCodedValueSet")]
    pub abnormal_coded_value_set: Option<Reference>,
    #[serde(rename = "criticalCodedValueSet")]
    pub critical_coded_value_set: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ObservationDefinitionQualifiedInterval {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: Option<Code>,
    pub range: Option<Range>,
    pub context: Option<CodeableConcept>,
    #[serde(rename = "appliesTo")]
    pub applies_to: Option<Vec<CodeableConcept>>,
    pub gender: Option<Code>,
    pub age: Option<Range>,
    #[serde(rename = "gestationalAge")]
    pub gestational_age: Option<Range>,
    pub condition: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct OperationDefinition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub version: Option<String>,
    pub name: String,
    pub title: Option<String>,
    pub status: Code,
    pub kind: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    #[serde(rename = "affectsState")]
    pub affects_state: Option<Boolean>,
    pub code: Code,
    pub comment: Option<Markdown>,
    pub base: Option<Canonical>,
    pub resource: Option<Vec<Code>>,
    pub system: Boolean,
    #[serde(rename = "type")]
    pub r#type: Boolean,
    pub instance: Boolean,
    #[serde(rename = "inputProfile")]
    pub input_profile: Option<Canonical>,
    #[serde(rename = "outputProfile")]
    pub output_profile: Option<Canonical>,
    pub parameter: Option<Vec<OperationDefinitionParameter>>,
    pub overload: Option<Vec<OperationDefinitionOverload>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OperationDefinitionOverload {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "parameterName")]
    pub parameter_name: Option<Vec<String>>,
    pub comment: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OperationDefinitionParameterReferencedFrom {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub source: String,
    #[serde(rename = "sourceId")]
    pub source_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OperationDefinitionParameter {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Code,
    #[serde(rename = "use")]
    pub r#use: Code,
    pub min: Integer,
    pub max: String,
    pub documentation: Option<String>,
    #[serde(rename = "type")]
    pub r#type: Option<Code>,
    #[serde(rename = "targetProfile")]
    pub target_profile: Option<Vec<Canonical>>,
    #[serde(rename = "searchType")]
    pub search_type: Option<Code>,
    pub binding: Option<OperationDefinitionParameterBinding>,
    #[serde(rename = "referencedFrom")]
    pub referenced_from: Option<Vec<OperationDefinitionParameterReferencedFrom>>,
    pub part: Option<Vec<OperationDefinitionParameter>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OperationDefinitionParameterBinding {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub strength: Code,
    #[serde(rename = "valueSet")]
    pub value_set: Canonical,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct OperationOutcomeIssue {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub severity: Code,
    pub code: Code,
    pub details: Option<CodeableConcept>,
    pub diagnostics: Option<String>,
    pub location: Option<Vec<String>>,
    pub expression: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OperationOutcome {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub issue: Option<Vec<OperationOutcomeIssue>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct Organization {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub active: Option<Boolean>,
    #[serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    pub name: Option<String>,
    pub alias: Option<Vec<String>>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub address: Option<Vec<Address>>,
    #[serde(rename = "partOf")]
    pub part_of: Option<Reference>,
    pub contact: Option<Vec<OrganizationContact>>,
    pub endpoint: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OrganizationContact {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub purpose: Option<CodeableConcept>,
    pub name: Option<HumanName>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub address: Option<Address>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct OrganizationAffiliation {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub active: Option<Boolean>,
    pub period: Option<Period>,
    pub organization: Option<Reference>,
    #[serde(rename = "participatingOrganization")]
    pub participating_organization: Option<Reference>,
    pub network: Option<Vec<Reference>>,
    pub code: Option<Vec<CodeableConcept>>,
    pub specialty: Option<Vec<CodeableConcept>>,
    pub location: Option<Vec<Reference>>,
    #[serde(rename = "healthcareService")]
    pub healthcare_service: Option<Vec<Reference>>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub endpoint: Option<Vec<Reference>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct PackagedProductDefinitionLegalStatusOfSupply {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub jurisdiction: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PackagedProductDefinitionPackage {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub quantity: Option<Integer>,
    pub material: Option<Vec<CodeableConcept>>,
    #[serde(rename = "alternateMaterial")]
    pub alternate_material: Option<Vec<CodeableConcept>>,
    #[serde(rename = "shelfLifeStorage")]
    pub shelf_life_storage: Option<Vec<PackagedProductDefinitionPackageShelfLifeStorage>>,
    pub manufacturer: Option<Vec<Reference>>,
    pub property: Option<Vec<PackagedProductDefinitionPackageProperty>>,
    #[serde(rename = "containedItem")]
    pub contained_item: Option<Vec<PackagedProductDefinitionPackageContainedItem>>,
    pub package: Option<Vec<PackagedProductDefinitionPackage>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PackagedProductDefinitionPackageShelfLifeStoragePeriod {
    Duration(Duration),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PackagedProductDefinitionPackageShelfLifeStorage {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "periodDuration")]
    pub period_duration: Option<Duration>,
    #[serde(rename = "periodString")]
    pub period_string: Option<String>,
    #[serde(rename = "specialPrecautionsForStorage")]
    pub special_precautions_for_storage: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PackagedProductDefinition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "packageFor")]
    pub package_for: Option<Vec<Reference>>,
    pub status: Option<CodeableConcept>,
    #[serde(rename = "statusDate")]
    pub status_date: Option<DateTime>,
    #[serde(rename = "containedItemQuantity")]
    pub contained_item_quantity: Option<Vec<Quantity>>,
    pub description: Option<Markdown>,
    #[serde(rename = "legalStatusOfSupply")]
    pub legal_status_of_supply: Option<Vec<PackagedProductDefinitionLegalStatusOfSupply>>,
    #[serde(rename = "marketingStatus")]
    pub marketing_status: Option<Vec<MarketingStatus>>,
    pub characteristic: Option<Vec<CodeableConcept>>,
    #[serde(rename = "copackagedIndicator")]
    pub copackaged_indicator: Option<Boolean>,
    pub manufacturer: Option<Vec<Reference>>,
    pub package: Option<PackagedProductDefinitionPackage>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PackagedProductDefinitionPackagePropertyValue {
    CodeableConcept(CodeableConcept),
    Quantity(Quantity),
    Date(Date),
    Boolean(Boolean),
    Attachment(Attachment),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PackagedProductDefinitionPackageProperty {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(rename = "valueCodeableConcept")]
    pub value_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "valueQuantity")]
    pub value_quantity: Option<Quantity>,
    #[serde(rename = "valueDate")]
    pub value_date: Option<Date>,
    #[serde(rename = "valueBoolean")]
    pub value_boolean: Option<Boolean>,
    #[serde(rename = "valueAttachment")]
    pub value_attachment: Option<Attachment>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PackagedProductDefinitionPackageContainedItem {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub item: CodeableReference,
    pub amount: Option<Quantity>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct Parameters {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub parameter: Option<Vec<ParametersParameter>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ParametersParameterValue {
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
pub struct ParametersParameter {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
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
    pub resource: Option<Resource>,
    pub part: Option<Vec<ParametersParameter>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct PatientCommunication {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub language: CodeableConcept,
    pub preferred: Option<Boolean>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PatientLink {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub other: Reference,
    #[serde(rename = "type")]
    pub r#type: Code,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PatientContact {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub relationship: Option<Vec<CodeableConcept>>,
    pub name: Option<HumanName>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub address: Option<Address>,
    pub gender: Option<Code>,
    pub organization: Option<Reference>,
    pub period: Option<Period>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PatientDeceased {
    Boolean(Boolean),
    DateTime(DateTime),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PatientMultipleBirth {
    Boolean(Boolean),
    Integer(Integer),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Patient {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub active: Option<Boolean>,
    pub name: Option<Vec<HumanName>>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub gender: Option<Code>,
    #[serde(rename = "birthDate")]
    pub birth_date: Option<Date>,
    #[serde(rename = "deceasedBoolean")]
    pub deceased_boolean: Option<Boolean>,
    #[serde(rename = "deceasedDateTime")]
    pub deceased_date_time: Option<DateTime>,
    pub address: Option<Vec<Address>>,
    #[serde(rename = "maritalStatus")]
    pub marital_status: Option<CodeableConcept>,
    #[serde(rename = "multipleBirthBoolean")]
    pub multiple_birth_boolean: Option<Boolean>,
    #[serde(rename = "multipleBirthInteger")]
    pub multiple_birth_integer: Option<Integer>,
    pub photo: Option<Vec<Attachment>>,
    pub contact: Option<Vec<PatientContact>>,
    pub communication: Option<Vec<PatientCommunication>>,
    #[serde(rename = "generalPractitioner")]
    pub general_practitioner: Option<Vec<Reference>>,
    #[serde(rename = "managingOrganization")]
    pub managing_organization: Option<Reference>,
    pub link: Option<Vec<PatientLink>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct PaymentNotice {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub request: Option<Reference>,
    pub response: Option<Reference>,
    pub created: DateTime,
    pub provider: Option<Reference>,
    pub payment: Reference,
    #[serde(rename = "paymentDate")]
    pub payment_date: Option<Date>,
    pub payee: Option<Reference>,
    pub recipient: Reference,
    pub amount: Money,
    #[serde(rename = "paymentStatus")]
    pub payment_status: Option<CodeableConcept>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct PaymentReconciliationDetail {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    pub predecessor: Option<Identifier>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    pub request: Option<Reference>,
    pub submitter: Option<Reference>,
    pub response: Option<Reference>,
    pub date: Option<Date>,
    pub responsible: Option<Reference>,
    pub payee: Option<Reference>,
    pub amount: Option<Money>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PaymentReconciliation {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub period: Option<Period>,
    pub created: DateTime,
    #[serde(rename = "paymentIssuer")]
    pub payment_issuer: Option<Reference>,
    pub request: Option<Reference>,
    pub requestor: Option<Reference>,
    pub outcome: Option<Code>,
    pub disposition: Option<String>,
    #[serde(rename = "paymentDate")]
    pub payment_date: Date,
    #[serde(rename = "paymentAmount")]
    pub payment_amount: Money,
    #[serde(rename = "paymentIdentifier")]
    pub payment_identifier: Option<Identifier>,
    pub detail: Option<Vec<PaymentReconciliationDetail>>,
    #[serde(rename = "formCode")]
    pub form_code: Option<CodeableConcept>,
    #[serde(rename = "processNote")]
    pub process_note: Option<Vec<PaymentReconciliationProcessNote>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PaymentReconciliationProcessNote {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Option<Code>,
    pub text: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct PersonLink {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub target: Reference,
    pub assurance: Option<Code>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Person {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub name: Option<Vec<HumanName>>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub gender: Option<Code>,
    #[serde(rename = "birthDate")]
    pub birth_date: Option<Date>,
    pub address: Option<Vec<Address>>,
    pub photo: Option<Attachment>,
    #[serde(rename = "managingOrganization")]
    pub managing_organization: Option<Reference>,
    pub active: Option<Boolean>,
    pub link: Option<Vec<PersonLink>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct PlanDefinitionActionDynamicValue {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub path: Option<String>,
    pub expression: Option<Expression>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlanDefinitionGoalTargetDetail {
    Quantity(Quantity),
    Range(Range),
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PlanDefinitionGoalTarget {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub measure: Option<CodeableConcept>,
    #[serde(rename = "detailQuantity")]
    pub detail_quantity: Option<Quantity>,
    #[serde(rename = "detailRange")]
    pub detail_range: Option<Range>,
    #[serde(rename = "detailCodeableConcept")]
    pub detail_codeable_concept: Option<CodeableConcept>,
    pub due: Option<Duration>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlanDefinitionActionSubject {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
    Canonical(Canonical),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlanDefinitionActionTiming {
    DateTime(DateTime),
    Age(Age),
    Period(Period),
    Duration(Duration),
    Range(Range),
    Timing(Timing),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlanDefinitionActionDefinition {
    Canonical(Canonical),
    Uri(Uri),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PlanDefinitionAction {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub prefix: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "textEquivalent")]
    pub text_equivalent: Option<String>,
    pub priority: Option<Code>,
    pub code: Option<Vec<CodeableConcept>>,
    pub reason: Option<Vec<CodeableConcept>>,
    pub documentation: Option<Vec<RelatedArtifact>>,
    #[serde(rename = "goalId")]
    pub goal_id: Option<Vec<Id>>,
    #[serde(rename = "subjectCodeableConcept")]
    pub subject_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "subjectReference")]
    pub subject_reference: Option<Reference>,
    #[serde(rename = "subjectCanonical")]
    pub subject_canonical: Option<Canonical>,
    pub trigger: Option<Vec<TriggerDefinition>>,
    pub condition: Option<Vec<PlanDefinitionActionCondition>>,
    pub input: Option<Vec<DataRequirement>>,
    pub output: Option<Vec<DataRequirement>>,
    #[serde(rename = "relatedAction")]
    pub related_action: Option<Vec<PlanDefinitionActionRelatedAction>>,
    #[serde(rename = "timingDateTime")]
    pub timing_date_time: Option<DateTime>,
    #[serde(rename = "timingAge")]
    pub timing_age: Option<Age>,
    #[serde(rename = "timingPeriod")]
    pub timing_period: Option<Period>,
    #[serde(rename = "timingDuration")]
    pub timing_duration: Option<Duration>,
    #[serde(rename = "timingRange")]
    pub timing_range: Option<Range>,
    #[serde(rename = "timingTiming")]
    pub timing_timing: Option<Timing>,
    pub participant: Option<Vec<PlanDefinitionActionParticipant>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "groupingBehavior")]
    pub grouping_behavior: Option<Code>,
    #[serde(rename = "selectionBehavior")]
    pub selection_behavior: Option<Code>,
    #[serde(rename = "requiredBehavior")]
    pub required_behavior: Option<Code>,
    #[serde(rename = "precheckBehavior")]
    pub precheck_behavior: Option<Code>,
    #[serde(rename = "cardinalityBehavior")]
    pub cardinality_behavior: Option<Code>,
    #[serde(rename = "definitionCanonical")]
    pub definition_canonical: Option<Canonical>,
    #[serde(rename = "definitionUri")]
    pub definition_uri: Option<Uri>,
    pub transform: Option<Canonical>,
    #[serde(rename = "dynamicValue")]
    pub dynamic_value: Option<Vec<PlanDefinitionActionDynamicValue>>,
    pub action: Option<Vec<PlanDefinitionAction>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PlanDefinitionActionParticipant {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub role: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlanDefinitionSubject {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
    Canonical(Canonical),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PlanDefinition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    #[serde(rename = "subjectCodeableConcept")]
    pub subject_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "subjectReference")]
    pub subject_reference: Option<Reference>,
    #[serde(rename = "subjectCanonical")]
    pub subject_canonical: Option<Canonical>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub usage: Option<String>,
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod")]
    pub effective_period: Option<Period>,
    pub topic: Option<Vec<CodeableConcept>>,
    pub author: Option<Vec<ContactDetail>>,
    pub editor: Option<Vec<ContactDetail>>,
    pub reviewer: Option<Vec<ContactDetail>>,
    pub endorser: Option<Vec<ContactDetail>>,
    #[serde(rename = "relatedArtifact")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    pub library: Option<Vec<Canonical>>,
    pub goal: Option<Vec<PlanDefinitionGoal>>,
    pub action: Option<Vec<PlanDefinitionAction>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PlanDefinitionActionCondition {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub kind: Code,
    pub expression: Option<Expression>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlanDefinitionActionRelatedActionOffset {
    Duration(Duration),
    Range(Range),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PlanDefinitionActionRelatedAction {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "actionId")]
    pub action_id: Id,
    pub relationship: Code,
    #[serde(rename = "offsetDuration")]
    pub offset_duration: Option<Duration>,
    #[serde(rename = "offsetRange")]
    pub offset_range: Option<Range>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PlanDefinitionGoal {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: Option<CodeableConcept>,
    pub description: CodeableConcept,
    pub priority: Option<CodeableConcept>,
    pub start: Option<CodeableConcept>,
    pub addresses: Option<Vec<CodeableConcept>>,
    pub documentation: Option<Vec<RelatedArtifact>>,
    pub target: Option<Vec<PlanDefinitionGoalTarget>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct Practitioner {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub active: Option<Boolean>,
    pub name: Option<Vec<HumanName>>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub address: Option<Vec<Address>>,
    pub gender: Option<Code>,
    #[serde(rename = "birthDate")]
    pub birth_date: Option<Date>,
    pub photo: Option<Vec<Attachment>>,
    pub qualification: Option<Vec<PractitionerQualification>>,
    pub communication: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PractitionerQualification {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub code: CodeableConcept,
    pub period: Option<Period>,
    pub issuer: Option<Reference>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct PractitionerRole {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub active: Option<Boolean>,
    pub period: Option<Period>,
    pub practitioner: Option<Reference>,
    pub organization: Option<Reference>,
    pub code: Option<Vec<CodeableConcept>>,
    pub specialty: Option<Vec<CodeableConcept>>,
    pub location: Option<Vec<Reference>>,
    #[serde(rename = "healthcareService")]
    pub healthcare_service: Option<Vec<Reference>>,
    pub telecom: Option<Vec<ContactPoint>>,
    #[serde(rename = "availableTime")]
    pub available_time: Option<Vec<PractitionerRoleAvailableTime>>,
    #[serde(rename = "notAvailable")]
    pub not_available: Option<Vec<PractitionerRoleNotAvailable>>,
    #[serde(rename = "availabilityExceptions")]
    pub availability_exceptions: Option<String>,
    pub endpoint: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PractitionerRoleAvailableTime {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "daysOfWeek")]
    pub days_of_week: Option<Vec<Code>>,
    #[serde(rename = "allDay")]
    pub all_day: Option<Boolean>,
    #[serde(rename = "availableStartTime")]
    pub available_start_time: Option<Time>,
    #[serde(rename = "availableEndTime")]
    pub available_end_time: Option<Time>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PractitionerRoleNotAvailable {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: String,
    pub during: Option<Period>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct ProcedurePerformer {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
    #[serde(rename = "onBehalfOf")]
    pub on_behalf_of: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProcedurePerformed {
    DateTime(DateTime),
    Period(Period),
    String(String),
    Age(Age),
    Range(Range),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Procedure {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "instantiatesCanonical")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[serde(rename = "instantiatesUri")]
    pub instantiates_uri: Option<Vec<Uri>>,
    #[serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(rename = "partOf")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    #[serde(rename = "statusReason")]
    pub status_reason: Option<CodeableConcept>,
    pub category: Option<CodeableConcept>,
    pub code: Option<CodeableConcept>,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    #[serde(rename = "performedDateTime")]
    pub performed_date_time: Option<DateTime>,
    #[serde(rename = "performedPeriod")]
    pub performed_period: Option<Period>,
    #[serde(rename = "performedString")]
    pub performed_string: Option<String>,
    #[serde(rename = "performedAge")]
    pub performed_age: Option<Age>,
    #[serde(rename = "performedRange")]
    pub performed_range: Option<Range>,
    pub recorder: Option<Reference>,
    pub asserter: Option<Reference>,
    pub performer: Option<Vec<ProcedurePerformer>>,
    pub location: Option<Reference>,
    #[serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(rename = "bodySite")]
    pub body_site: Option<Vec<CodeableConcept>>,
    pub outcome: Option<CodeableConcept>,
    pub report: Option<Vec<Reference>>,
    pub complication: Option<Vec<CodeableConcept>>,
    #[serde(rename = "complicationDetail")]
    pub complication_detail: Option<Vec<Reference>>,
    #[serde(rename = "followUp")]
    pub follow_up: Option<Vec<CodeableConcept>>,
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "focalDevice")]
    pub focal_device: Option<Vec<ProcedureFocalDevice>>,
    #[serde(rename = "usedReference")]
    pub used_reference: Option<Vec<Reference>>,
    #[serde(rename = "usedCode")]
    pub used_code: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProcedureFocalDevice {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub action: Option<CodeableConcept>,
    pub manipulated: Reference,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct ProvenanceEntity {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub role: Code,
    pub what: Reference,
    pub agent: Option<Vec<ProvenanceAgent>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProvenanceOccurred {
    Period(Period),
    DateTime(DateTime),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Provenance {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub target: Option<Vec<Reference>>,
    #[serde(rename = "occurredPeriod")]
    pub occurred_period: Option<Period>,
    #[serde(rename = "occurredDateTime")]
    pub occurred_date_time: Option<DateTime>,
    pub recorded: Instant,
    pub policy: Option<Vec<Uri>>,
    pub location: Option<Reference>,
    pub reason: Option<Vec<CodeableConcept>>,
    pub activity: Option<CodeableConcept>,
    pub agent: Option<Vec<ProvenanceAgent>>,
    pub entity: Option<Vec<ProvenanceEntity>>,
    pub signature: Option<Vec<Signature>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProvenanceAgent {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub role: Option<Vec<CodeableConcept>>,
    pub who: Reference,
    #[serde(rename = "onBehalfOf")]
    pub on_behalf_of: Option<Reference>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct Questionnaire {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    #[serde(rename = "derivedFrom")]
    pub derived_from: Option<Vec<Canonical>>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    #[serde(rename = "subjectType")]
    pub subject_type: Option<Vec<Code>>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod")]
    pub effective_period: Option<Period>,
    pub code: Option<Vec<Coding>>,
    pub item: Option<Vec<QuestionnaireItem>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum QuestionnaireItemEnableWhenAnswer {
    Boolean(Boolean),
    Decimal(Decimal),
    Integer(Integer),
    Date(Date),
    DateTime(DateTime),
    Time(Time),
    String(String),
    Coding(Coding),
    Quantity(Quantity),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct QuestionnaireItemEnableWhen {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub question: String,
    pub operator: Code,
    #[serde(rename = "answerBoolean")]
    pub answer_boolean: Boolean,
    #[serde(rename = "answerDecimal")]
    pub answer_decimal: Decimal,
    #[serde(rename = "answerInteger")]
    pub answer_integer: Integer,
    #[serde(rename = "answerDate")]
    pub answer_date: Date,
    #[serde(rename = "answerDateTime")]
    pub answer_date_time: DateTime,
    #[serde(rename = "answerTime")]
    pub answer_time: Time,
    #[serde(rename = "answerString")]
    pub answer_string: String,
    #[serde(rename = "answerCoding")]
    pub answer_coding: Coding,
    #[serde(rename = "answerQuantity")]
    pub answer_quantity: Quantity,
    #[serde(rename = "answerReference")]
    pub answer_reference: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum QuestionnaireItemAnswerOptionValue {
    Integer(Integer),
    Date(Date),
    Time(Time),
    String(String),
    Coding(Coding),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct QuestionnaireItemAnswerOption {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "valueInteger")]
    pub value_integer: Integer,
    #[serde(rename = "valueDate")]
    pub value_date: Date,
    #[serde(rename = "valueTime")]
    pub value_time: Time,
    #[serde(rename = "valueString")]
    pub value_string: String,
    #[serde(rename = "valueCoding")]
    pub value_coding: Coding,
    #[serde(rename = "valueReference")]
    pub value_reference: Reference,
    #[serde(rename = "initialSelected")]
    pub initial_selected: Option<Boolean>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum QuestionnaireItemInitialValue {
    Boolean(Boolean),
    Decimal(Decimal),
    Integer(Integer),
    Date(Date),
    DateTime(DateTime),
    Time(Time),
    String(String),
    Uri(Uri),
    Attachment(Attachment),
    Coding(Coding),
    Quantity(Quantity),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct QuestionnaireItemInitial {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "valueBoolean")]
    pub value_boolean: Boolean,
    #[serde(rename = "valueDecimal")]
    pub value_decimal: Decimal,
    #[serde(rename = "valueInteger")]
    pub value_integer: Integer,
    #[serde(rename = "valueDate")]
    pub value_date: Date,
    #[serde(rename = "valueDateTime")]
    pub value_date_time: DateTime,
    #[serde(rename = "valueTime")]
    pub value_time: Time,
    #[serde(rename = "valueString")]
    pub value_string: String,
    #[serde(rename = "valueUri")]
    pub value_uri: Uri,
    #[serde(rename = "valueAttachment")]
    pub value_attachment: Attachment,
    #[serde(rename = "valueCoding")]
    pub value_coding: Coding,
    #[serde(rename = "valueQuantity")]
    pub value_quantity: Quantity,
    #[serde(rename = "valueReference")]
    pub value_reference: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct QuestionnaireItem {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "linkId")]
    pub link_id: String,
    pub definition: Option<Uri>,
    pub code: Option<Vec<Coding>>,
    pub prefix: Option<String>,
    pub text: Option<String>,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(rename = "enableWhen")]
    pub enable_when: Option<Vec<QuestionnaireItemEnableWhen>>,
    #[serde(rename = "enableBehavior")]
    pub enable_behavior: Option<Code>,
    pub required: Option<Boolean>,
    pub repeats: Option<Boolean>,
    #[serde(rename = "readOnly")]
    pub read_only: Option<Boolean>,
    #[serde(rename = "maxLength")]
    pub max_length: Option<Integer>,
    #[serde(rename = "answerValueSet")]
    pub answer_value_set: Option<Canonical>,
    #[serde(rename = "answerOption")]
    pub answer_option: Option<Vec<QuestionnaireItemAnswerOption>>,
    pub initial: Option<Vec<QuestionnaireItemInitial>>,
    pub item: Option<Vec<QuestionnaireItem>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum QuestionnaireResponseItemAnswerValue {
    Boolean(Boolean),
    Decimal(Decimal),
    Integer(Integer),
    Date(Date),
    DateTime(DateTime),
    Time(Time),
    String(String),
    Uri(Uri),
    Attachment(Attachment),
    Coding(Coding),
    Quantity(Quantity),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct QuestionnaireResponseItemAnswer {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "valueBoolean")]
    pub value_boolean: Option<Boolean>,
    #[serde(rename = "valueDecimal")]
    pub value_decimal: Option<Decimal>,
    #[serde(rename = "valueInteger")]
    pub value_integer: Option<Integer>,
    #[serde(rename = "valueDate")]
    pub value_date: Option<Date>,
    #[serde(rename = "valueDateTime")]
    pub value_date_time: Option<DateTime>,
    #[serde(rename = "valueTime")]
    pub value_time: Option<Time>,
    #[serde(rename = "valueString")]
    pub value_string: Option<String>,
    #[serde(rename = "valueUri")]
    pub value_uri: Option<Uri>,
    #[serde(rename = "valueAttachment")]
    pub value_attachment: Option<Attachment>,
    #[serde(rename = "valueCoding")]
    pub value_coding: Option<Coding>,
    #[serde(rename = "valueQuantity")]
    pub value_quantity: Option<Quantity>,
    #[serde(rename = "valueReference")]
    pub value_reference: Option<Reference>,
    pub item: Option<Vec<QuestionnaireResponseItem>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct QuestionnaireResponse {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    #[serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(rename = "partOf")]
    pub part_of: Option<Vec<Reference>>,
    pub questionnaire: Option<Canonical>,
    pub status: Code,
    pub subject: Option<Reference>,
    pub encounter: Option<Reference>,
    pub authored: Option<DateTime>,
    pub author: Option<Reference>,
    pub source: Option<Reference>,
    pub item: Option<Vec<QuestionnaireResponseItem>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct QuestionnaireResponseItem {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "linkId")]
    pub link_id: String,
    pub definition: Option<Uri>,
    pub text: Option<String>,
    pub answer: Option<Vec<QuestionnaireResponseItemAnswer>>,
    pub item: Option<Vec<QuestionnaireResponseItem>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct RegulatedAuthorization {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub subject: Option<Vec<Reference>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub description: Option<Markdown>,
    pub region: Option<Vec<CodeableConcept>>,
    pub status: Option<CodeableConcept>,
    #[serde(rename = "statusDate")]
    pub status_date: Option<DateTime>,
    #[serde(rename = "validityPeriod")]
    pub validity_period: Option<Period>,
    pub indication: Option<CodeableReference>,
    #[serde(rename = "intendedUse")]
    pub intended_use: Option<CodeableConcept>,
    pub basis: Option<Vec<CodeableConcept>>,
    pub holder: Option<Reference>,
    pub regulator: Option<Reference>,
    pub case: Option<RegulatedAuthorizationCase>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RegulatedAuthorizationCaseDate {
    Period(Period),
    DateTime(DateTime),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RegulatedAuthorizationCase {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub status: Option<CodeableConcept>,
    #[serde(rename = "datePeriod")]
    pub date_period: Option<Period>,
    #[serde(rename = "dateDateTime")]
    pub date_date_time: Option<DateTime>,
    pub application: Option<Vec<RegulatedAuthorizationCase>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct RelatedPerson {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub active: Option<Boolean>,
    pub patient: Reference,
    pub relationship: Option<Vec<CodeableConcept>>,
    pub name: Option<Vec<HumanName>>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub gender: Option<Code>,
    #[serde(rename = "birthDate")]
    pub birth_date: Option<Date>,
    pub address: Option<Vec<Address>>,
    pub photo: Option<Vec<Attachment>>,
    pub period: Option<Period>,
    pub communication: Option<Vec<RelatedPersonCommunication>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RelatedPersonCommunication {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub language: CodeableConcept,
    pub preferred: Option<Boolean>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RequestGroupActionTiming {
    DateTime(DateTime),
    Age(Age),
    Period(Period),
    Duration(Duration),
    Range(Range),
    Timing(Timing),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RequestGroupAction {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub prefix: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    #[serde(rename = "textEquivalent")]
    pub text_equivalent: Option<String>,
    pub priority: Option<Code>,
    pub code: Option<Vec<CodeableConcept>>,
    pub documentation: Option<Vec<RelatedArtifact>>,
    pub condition: Option<Vec<RequestGroupActionCondition>>,
    #[serde(rename = "relatedAction")]
    pub related_action: Option<Vec<RequestGroupActionRelatedAction>>,
    #[serde(rename = "timingDateTime")]
    pub timing_date_time: Option<DateTime>,
    #[serde(rename = "timingAge")]
    pub timing_age: Option<Age>,
    #[serde(rename = "timingPeriod")]
    pub timing_period: Option<Period>,
    #[serde(rename = "timingDuration")]
    pub timing_duration: Option<Duration>,
    #[serde(rename = "timingRange")]
    pub timing_range: Option<Range>,
    #[serde(rename = "timingTiming")]
    pub timing_timing: Option<Timing>,
    pub participant: Option<Vec<Reference>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "groupingBehavior")]
    pub grouping_behavior: Option<Code>,
    #[serde(rename = "selectionBehavior")]
    pub selection_behavior: Option<Code>,
    #[serde(rename = "requiredBehavior")]
    pub required_behavior: Option<Code>,
    #[serde(rename = "precheckBehavior")]
    pub precheck_behavior: Option<Code>,
    #[serde(rename = "cardinalityBehavior")]
    pub cardinality_behavior: Option<Code>,
    pub resource: Option<Reference>,
    pub action: Option<Vec<RequestGroupAction>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RequestGroup {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "instantiatesCanonical")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[serde(rename = "instantiatesUri")]
    pub instantiates_uri: Option<Vec<Uri>>,
    #[serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    pub replaces: Option<Vec<Reference>>,
    #[serde(rename = "groupIdentifier")]
    pub group_identifier: Option<Identifier>,
    pub status: Code,
    pub intent: Code,
    pub priority: Option<Code>,
    pub code: Option<CodeableConcept>,
    pub subject: Option<Reference>,
    pub encounter: Option<Reference>,
    #[serde(rename = "authoredOn")]
    pub authored_on: Option<DateTime>,
    pub author: Option<Reference>,
    #[serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    pub action: Option<Vec<RequestGroupAction>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RequestGroupActionCondition {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub kind: Code,
    pub expression: Option<Expression>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RequestGroupActionRelatedActionOffset {
    Duration(Duration),
    Range(Range),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RequestGroupActionRelatedAction {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "actionId")]
    pub action_id: Id,
    pub relationship: Code,
    #[serde(rename = "offsetDuration")]
    pub offset_duration: Option<Duration>,
    #[serde(rename = "offsetRange")]
    pub offset_range: Option<Range>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResearchDefinitionSubject {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResearchDefinition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    #[serde(rename = "shortTitle")]
    pub short_title: Option<String>,
    pub subtitle: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    #[serde(rename = "subjectCodeableConcept")]
    pub subject_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "subjectReference")]
    pub subject_reference: Option<Reference>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    pub comment: Option<Vec<String>>,
    #[serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub usage: Option<String>,
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod")]
    pub effective_period: Option<Period>,
    pub topic: Option<Vec<CodeableConcept>>,
    pub author: Option<Vec<ContactDetail>>,
    pub editor: Option<Vec<ContactDetail>>,
    pub reviewer: Option<Vec<ContactDetail>>,
    pub endorser: Option<Vec<ContactDetail>>,
    #[serde(rename = "relatedArtifact")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    pub library: Option<Vec<Canonical>>,
    pub population: Reference,
    pub exposure: Option<Reference>,
    #[serde(rename = "exposureAlternative")]
    pub exposure_alternative: Option<Reference>,
    pub outcome: Option<Reference>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResearchElementDefinitionCharacteristicDefinition {
    CodeableConcept(CodeableConcept),
    Canonical(Canonical),
    Expression(Expression),
    DataRequirement(DataRequirement),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResearchElementDefinitionCharacteristicStudyEffective {
    DateTime(DateTime),
    Period(Period),
    Duration(Duration),
    Timing(Timing),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResearchElementDefinitionCharacteristicParticipantEffective {
    DateTime(DateTime),
    Period(Period),
    Duration(Duration),
    Timing(Timing),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResearchElementDefinitionCharacteristic {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "definitionCodeableConcept")]
    pub definition_codeable_concept: CodeableConcept,
    #[serde(rename = "definitionCanonical")]
    pub definition_canonical: Canonical,
    #[serde(rename = "definitionExpression")]
    pub definition_expression: Expression,
    #[serde(rename = "definitionDataRequirement")]
    pub definition_data_requirement: DataRequirement,
    #[serde(rename = "usageContext")]
    pub usage_context: Option<Vec<UsageContext>>,
    pub exclude: Option<Boolean>,
    #[serde(rename = "unitOfMeasure")]
    pub unit_of_measure: Option<CodeableConcept>,
    #[serde(rename = "studyEffectiveDescription")]
    pub study_effective_description: Option<String>,
    #[serde(rename = "studyEffectiveDateTime")]
    pub study_effective_date_time: Option<DateTime>,
    #[serde(rename = "studyEffectivePeriod")]
    pub study_effective_period: Option<Period>,
    #[serde(rename = "studyEffectiveDuration")]
    pub study_effective_duration: Option<Duration>,
    #[serde(rename = "studyEffectiveTiming")]
    pub study_effective_timing: Option<Timing>,
    #[serde(rename = "studyEffectiveTimeFromStart")]
    pub study_effective_time_from_start: Option<Duration>,
    #[serde(rename = "studyEffectiveGroupMeasure")]
    pub study_effective_group_measure: Option<Code>,
    #[serde(rename = "participantEffectiveDescription")]
    pub participant_effective_description: Option<String>,
    #[serde(rename = "participantEffectiveDateTime")]
    pub participant_effective_date_time: Option<DateTime>,
    #[serde(rename = "participantEffectivePeriod")]
    pub participant_effective_period: Option<Period>,
    #[serde(rename = "participantEffectiveDuration")]
    pub participant_effective_duration: Option<Duration>,
    #[serde(rename = "participantEffectiveTiming")]
    pub participant_effective_timing: Option<Timing>,
    #[serde(rename = "participantEffectiveTimeFromStart")]
    pub participant_effective_time_from_start: Option<Duration>,
    #[serde(rename = "participantEffectiveGroupMeasure")]
    pub participant_effective_group_measure: Option<Code>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResearchElementDefinitionSubject {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResearchElementDefinition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    #[serde(rename = "shortTitle")]
    pub short_title: Option<String>,
    pub subtitle: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    #[serde(rename = "subjectCodeableConcept")]
    pub subject_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "subjectReference")]
    pub subject_reference: Option<Reference>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    pub comment: Option<Vec<String>>,
    #[serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub usage: Option<String>,
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod")]
    pub effective_period: Option<Period>,
    pub topic: Option<Vec<CodeableConcept>>,
    pub author: Option<Vec<ContactDetail>>,
    pub editor: Option<Vec<ContactDetail>>,
    pub reviewer: Option<Vec<ContactDetail>>,
    pub endorser: Option<Vec<ContactDetail>>,
    #[serde(rename = "relatedArtifact")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    pub library: Option<Vec<Canonical>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(rename = "variableType")]
    pub variable_type: Option<Code>,
    pub characteristic: Option<Vec<ResearchElementDefinitionCharacteristic>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct ResearchStudyObjective {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResearchStudy {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub title: Option<String>,
    pub protocol: Option<Vec<Reference>>,
    #[serde(rename = "partOf")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    #[serde(rename = "primaryPurposeType")]
    pub primary_purpose_type: Option<CodeableConcept>,
    pub phase: Option<CodeableConcept>,
    pub category: Option<Vec<CodeableConcept>>,
    pub focus: Option<Vec<CodeableConcept>>,
    pub condition: Option<Vec<CodeableConcept>>,
    pub contact: Option<Vec<ContactDetail>>,
    #[serde(rename = "relatedArtifact")]
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    pub keyword: Option<Vec<CodeableConcept>>,
    pub location: Option<Vec<CodeableConcept>>,
    pub description: Option<Markdown>,
    pub enrollment: Option<Vec<Reference>>,
    pub period: Option<Period>,
    pub sponsor: Option<Reference>,
    #[serde(rename = "principalInvestigator")]
    pub principal_investigator: Option<Reference>,
    pub site: Option<Vec<Reference>>,
    #[serde(rename = "reasonStopped")]
    pub reason_stopped: Option<CodeableConcept>,
    pub note: Option<Vec<Annotation>>,
    pub arm: Option<Vec<ResearchStudyArm>>,
    pub objective: Option<Vec<ResearchStudyObjective>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResearchStudyArm {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub description: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct ResearchSubject {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub period: Option<Period>,
    pub study: Reference,
    pub individual: Reference,
    #[serde(rename = "assignedArm")]
    pub assigned_arm: Option<String>,
    #[serde(rename = "actualArm")]
    pub actual_arm: Option<String>,
    pub consent: Option<Reference>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RiskAssessmentPredictionProbability {
    Decimal(Decimal),
    Range(Range),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RiskAssessmentPredictionWhen {
    Period(Period),
    Range(Range),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RiskAssessmentPrediction {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub outcome: Option<CodeableConcept>,
    #[serde(rename = "probabilityDecimal")]
    pub probability_decimal: Option<Decimal>,
    #[serde(rename = "probabilityRange")]
    pub probability_range: Option<Range>,
    #[serde(rename = "qualitativeRisk")]
    pub qualitative_risk: Option<CodeableConcept>,
    #[serde(rename = "relativeRisk")]
    pub relative_risk: Option<Decimal>,
    #[serde(rename = "whenPeriod")]
    pub when_period: Option<Period>,
    #[serde(rename = "whenRange")]
    pub when_range: Option<Range>,
    pub rationale: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RiskAssessmentOccurrence {
    DateTime(DateTime),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RiskAssessment {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "basedOn")]
    pub based_on: Option<Reference>,
    pub parent: Option<Reference>,
    pub status: Code,
    pub method: Option<CodeableConcept>,
    pub code: Option<CodeableConcept>,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    #[serde(rename = "occurrenceDateTime")]
    pub occurrence_date_time: Option<DateTime>,
    #[serde(rename = "occurrencePeriod")]
    pub occurrence_period: Option<Period>,
    pub condition: Option<Reference>,
    pub performer: Option<Reference>,
    #[serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    pub basis: Option<Vec<Reference>>,
    pub prediction: Option<Vec<RiskAssessmentPrediction>>,
    pub mitigation: Option<String>,
    pub note: Option<Vec<Annotation>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct Schedule {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub active: Option<Boolean>,
    #[serde(rename = "serviceCategory")]
    pub service_category: Option<Vec<CodeableConcept>>,
    #[serde(rename = "serviceType")]
    pub service_type: Option<Vec<CodeableConcept>>,
    pub specialty: Option<Vec<CodeableConcept>>,
    pub actor: Option<Vec<Reference>>,
    #[serde(rename = "planningHorizon")]
    pub planning_horizon: Option<Period>,
    pub comment: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct SearchParameterComponent {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub definition: Canonical,
    pub expression: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchParameter {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Uri,
    pub version: Option<String>,
    pub name: String,
    #[serde(rename = "derivedFrom")]
    pub derived_from: Option<Canonical>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Markdown,
    #[serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub code: Code,
    pub base: Option<Vec<Code>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub expression: Option<String>,
    pub xpath: Option<String>,
    #[serde(rename = "xpathUsage")]
    pub xpath_usage: Option<Code>,
    pub target: Option<Vec<Code>>,
    #[serde(rename = "multipleOr")]
    pub multiple_or: Option<Boolean>,
    #[serde(rename = "multipleAnd")]
    pub multiple_and: Option<Boolean>,
    pub comparator: Option<Vec<Code>>,
    pub modifier: Option<Vec<Code>>,
    pub chain: Option<Vec<String>>,
    pub component: Option<Vec<SearchParameterComponent>>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ServiceRequestQuantity {
    Quantity(Quantity),
    Ratio(Ratio),
    Range(Range),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ServiceRequestOccurrence {
    DateTime(DateTime),
    Period(Period),
    Timing(Timing),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ServiceRequestAsNeeded {
    Boolean(Boolean),
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ServiceRequest {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "instantiatesCanonical")]
    pub instantiates_canonical: Option<Vec<Canonical>>,
    #[serde(rename = "instantiatesUri")]
    pub instantiates_uri: Option<Vec<Uri>>,
    #[serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    pub replaces: Option<Vec<Reference>>,
    pub requisition: Option<Identifier>,
    pub status: Code,
    pub intent: Code,
    pub category: Option<Vec<CodeableConcept>>,
    pub priority: Option<Code>,
    #[serde(rename = "doNotPerform")]
    pub do_not_perform: Option<Boolean>,
    pub code: Option<CodeableConcept>,
    #[serde(rename = "orderDetail")]
    pub order_detail: Option<Vec<CodeableConcept>>,
    #[serde(rename = "quantityQuantity")]
    pub quantity_quantity: Option<Quantity>,
    #[serde(rename = "quantityRatio")]
    pub quantity_ratio: Option<Ratio>,
    #[serde(rename = "quantityRange")]
    pub quantity_range: Option<Range>,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    #[serde(rename = "occurrenceDateTime")]
    pub occurrence_date_time: Option<DateTime>,
    #[serde(rename = "occurrencePeriod")]
    pub occurrence_period: Option<Period>,
    #[serde(rename = "occurrenceTiming")]
    pub occurrence_timing: Option<Timing>,
    #[serde(rename = "asNeededBoolean")]
    pub as_needed_boolean: Option<Boolean>,
    #[serde(rename = "asNeededCodeableConcept")]
    pub as_needed_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "authoredOn")]
    pub authored_on: Option<DateTime>,
    pub requester: Option<Reference>,
    #[serde(rename = "performerType")]
    pub performer_type: Option<CodeableConcept>,
    pub performer: Option<Vec<Reference>>,
    #[serde(rename = "locationCode")]
    pub location_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "locationReference")]
    pub location_reference: Option<Vec<Reference>>,
    #[serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    pub insurance: Option<Vec<Reference>>,
    #[serde(rename = "supportingInfo")]
    pub supporting_info: Option<Vec<Reference>>,
    pub specimen: Option<Vec<Reference>>,
    #[serde(rename = "bodySite")]
    pub body_site: Option<Vec<CodeableConcept>>,
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "patientInstruction")]
    pub patient_instruction: Option<String>,
    #[serde(rename = "relevantHistory")]
    pub relevant_history: Option<Vec<Reference>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct Slot {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "serviceCategory")]
    pub service_category: Option<Vec<CodeableConcept>>,
    #[serde(rename = "serviceType")]
    pub service_type: Option<Vec<CodeableConcept>>,
    pub specialty: Option<Vec<CodeableConcept>>,
    #[serde(rename = "appointmentType")]
    pub appointment_type: Option<CodeableConcept>,
    pub schedule: Reference,
    pub status: Code,
    pub start: Instant,
    pub end: Instant,
    pub overbooked: Option<Boolean>,
    pub comment: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpecimenContainerAdditive {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SpecimenContainer {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub capacity: Option<Quantity>,
    #[serde(rename = "specimenQuantity")]
    pub specimen_quantity: Option<Quantity>,
    #[serde(rename = "additiveCodeableConcept")]
    pub additive_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "additiveReference")]
    pub additive_reference: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpecimenCollectionCollected {
    DateTime(DateTime),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpecimenCollectionFastingStatus {
    CodeableConcept(CodeableConcept),
    Duration(Duration),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SpecimenCollection {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub collector: Option<Reference>,
    #[serde(rename = "collectedDateTime")]
    pub collected_date_time: Option<DateTime>,
    #[serde(rename = "collectedPeriod")]
    pub collected_period: Option<Period>,
    pub duration: Option<Duration>,
    pub quantity: Option<Quantity>,
    pub method: Option<CodeableConcept>,
    #[serde(rename = "bodySite")]
    pub body_site: Option<CodeableConcept>,
    #[serde(rename = "fastingStatusCodeableConcept")]
    pub fasting_status_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "fastingStatusDuration")]
    pub fasting_status_duration: Option<Duration>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Specimen {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "accessionIdentifier")]
    pub accession_identifier: Option<Identifier>,
    pub status: Option<Code>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub subject: Option<Reference>,
    #[serde(rename = "receivedTime")]
    pub received_time: Option<DateTime>,
    pub parent: Option<Vec<Reference>>,
    pub request: Option<Vec<Reference>>,
    pub collection: Option<SpecimenCollection>,
    pub processing: Option<Vec<SpecimenProcessing>>,
    pub container: Option<Vec<SpecimenContainer>>,
    pub condition: Option<Vec<CodeableConcept>>,
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpecimenProcessingTime {
    DateTime(DateTime),
    Period(Period),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SpecimenProcessing {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    pub procedure: Option<CodeableConcept>,
    pub additive: Option<Vec<Reference>>,
    #[serde(rename = "timeDateTime")]
    pub time_date_time: Option<DateTime>,
    #[serde(rename = "timePeriod")]
    pub time_period: Option<Period>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct SpecimenDefinitionTypeTested {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "isDerived")]
    pub is_derived: Option<Boolean>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub preference: Code,
    pub container: Option<SpecimenDefinitionTypeTestedContainer>,
    pub requirement: Option<String>,
    #[serde(rename = "retentionTime")]
    pub retention_time: Option<Duration>,
    #[serde(rename = "rejectionCriterion")]
    pub rejection_criterion: Option<Vec<CodeableConcept>>,
    pub handling: Option<Vec<SpecimenDefinitionTypeTestedHandling>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SpecimenDefinition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    #[serde(rename = "typeCollected")]
    pub type_collected: Option<CodeableConcept>,
    #[serde(rename = "patientPreparation")]
    pub patient_preparation: Option<Vec<CodeableConcept>>,
    #[serde(rename = "timeAspect")]
    pub time_aspect: Option<String>,
    pub collection: Option<Vec<CodeableConcept>>,
    #[serde(rename = "typeTested")]
    pub type_tested: Option<Vec<SpecimenDefinitionTypeTested>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpecimenDefinitionTypeTestedContainerAdditiveAdditive {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SpecimenDefinitionTypeTestedContainerAdditive {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "additiveCodeableConcept")]
    pub additive_codeable_concept: CodeableConcept,
    #[serde(rename = "additiveReference")]
    pub additive_reference: Reference,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SpecimenDefinitionTypeTestedHandling {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "temperatureQualifier")]
    pub temperature_qualifier: Option<CodeableConcept>,
    #[serde(rename = "temperatureRange")]
    pub temperature_range: Option<Range>,
    #[serde(rename = "maxDuration")]
    pub max_duration: Option<Duration>,
    pub instruction: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SpecimenDefinitionTypeTestedContainerMinimumVolume {
    Quantity(Quantity),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SpecimenDefinitionTypeTestedContainer {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub material: Option<CodeableConcept>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub cap: Option<CodeableConcept>,
    pub description: Option<String>,
    pub capacity: Option<Quantity>,
    #[serde(rename = "minimumVolumeQuantity")]
    pub minimum_volume_quantity: Option<Quantity>,
    #[serde(rename = "minimumVolumeString")]
    pub minimum_volume_string: Option<String>,
    pub additive: Option<Vec<SpecimenDefinitionTypeTestedContainerAdditive>>,
    pub preparation: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct StructureDefinitionContext {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub expression: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StructureDefinitionMapping {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identity: Id,
    pub uri: Option<Uri>,
    pub name: Option<String>,
    pub comment: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StructureDefinitionSnapshot {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub element: Option<Vec<ElementDefinition>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StructureDefinitionDifferential {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub element: Option<Vec<ElementDefinition>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StructureDefinition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Uri,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: String,
    pub title: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    pub keyword: Option<Vec<Coding>>,
    #[serde(rename = "fhirVersion")]
    pub fhir_version: Option<Code>,
    pub mapping: Option<Vec<StructureDefinitionMapping>>,
    pub kind: Code,
    #[serde(rename = "abstract")]
    pub r#abstract: Boolean,
    pub context: Option<Vec<StructureDefinitionContext>>,
    #[serde(rename = "contextInvariant")]
    pub context_invariant: Option<Vec<String>>,
    #[serde(rename = "type")]
    pub r#type: Uri,
    #[serde(rename = "baseDefinition")]
    pub base_definition: Option<Canonical>,
    pub derivation: Option<Code>,
    pub snapshot: Option<StructureDefinitionSnapshot>,
    pub differential: Option<StructureDefinitionDifferential>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct StructureMapGroupRuleDependent {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Id,
    pub variable: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StructureMapGroupRuleTargetParameterValue {
    Id(Id),
    String(String),
    Boolean(Boolean),
    Integer(Integer),
    Decimal(Decimal),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StructureMapGroupRuleTargetParameter {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "valueId")]
    pub value_id: Id,
    #[serde(rename = "valueString")]
    pub value_string: String,
    #[serde(rename = "valueBoolean")]
    pub value_boolean: Boolean,
    #[serde(rename = "valueInteger")]
    pub value_integer: Integer,
    #[serde(rename = "valueDecimal")]
    pub value_decimal: Decimal,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StructureMap {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Uri,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: String,
    pub title: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    pub structure: Option<Vec<StructureMapStructure>>,
    pub import: Option<Vec<Canonical>>,
    pub group: Option<Vec<StructureMapGroup>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StructureMapGroupInput {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Id,
    #[serde(rename = "type")]
    pub r#type: Option<String>,
    pub mode: Code,
    pub documentation: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StructureMapGroupRuleSourceDefaultValue {
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
pub struct StructureMapGroupRuleSource {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub context: Id,
    pub min: Option<Integer>,
    pub max: Option<String>,
    #[serde(rename = "type")]
    pub r#type: Option<String>,
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
    pub element: Option<String>,
    #[serde(rename = "listMode")]
    pub list_mode: Option<Code>,
    pub variable: Option<Id>,
    pub condition: Option<String>,
    pub check: Option<String>,
    #[serde(rename = "logMessage")]
    pub log_message: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StructureMapGroup {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Id,
    pub extends: Option<Id>,
    #[serde(rename = "typeMode")]
    pub type_mode: Code,
    pub documentation: Option<String>,
    pub input: Option<Vec<StructureMapGroupInput>>,
    pub rule: Option<Vec<StructureMapGroupRule>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StructureMapGroupRuleTarget {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub context: Option<Id>,
    #[serde(rename = "contextType")]
    pub context_type: Option<Code>,
    pub element: Option<String>,
    pub variable: Option<Id>,
    #[serde(rename = "listMode")]
    pub list_mode: Option<Vec<Code>>,
    #[serde(rename = "listRuleId")]
    pub list_rule_id: Option<Id>,
    pub transform: Option<Code>,
    pub parameter: Option<Vec<StructureMapGroupRuleTargetParameter>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StructureMapStructure {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Canonical,
    pub mode: Code,
    pub alias: Option<String>,
    pub documentation: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StructureMapGroupRule {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Id,
    pub source: Option<Vec<StructureMapGroupRuleSource>>,
    pub target: Option<Vec<StructureMapGroupRuleTarget>>,
    pub rule: Option<Vec<StructureMapGroupRule>>,
    pub dependent: Option<Vec<StructureMapGroupRuleDependent>>,
    pub documentation: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct SubscriptionChannel {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub endpoint: Option<Url>,
    pub payload: Option<Code>,
    pub header: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Subscription {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub status: Code,
    pub contact: Option<Vec<ContactPoint>>,
    pub end: Option<Instant>,
    pub reason: String,
    pub criteria: String,
    pub error: Option<String>,
    pub channel: SubscriptionChannel,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct SubscriptionStatusNotificationEvent {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "eventNumber")]
    pub event_number: String,
    pub timestamp: Option<Instant>,
    pub focus: Option<Reference>,
    #[serde(rename = "additionalContext")]
    pub additional_context: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SubscriptionStatus {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub status: Option<Code>,
    #[serde(rename = "type")]
    pub r#type: Code,
    #[serde(rename = "eventsSinceSubscriptionStart")]
    pub events_since_subscription_start: Option<String>,
    #[serde(rename = "notificationEvent")]
    pub notification_event: Option<Vec<SubscriptionStatusNotificationEvent>>,
    pub subscription: Reference,
    pub topic: Option<Canonical>,
    pub error: Option<Vec<CodeableConcept>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct SubscriptionTopicEventTrigger {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<Markdown>,
    pub event: CodeableConcept,
    pub resource: Uri,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SubscriptionTopicCanFilterBy {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<Markdown>,
    pub resource: Option<Uri>,
    #[serde(rename = "filterParameter")]
    pub filter_parameter: String,
    #[serde(rename = "filterDefinition")]
    pub filter_definition: Option<Uri>,
    pub modifier: Option<Vec<Code>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SubscriptionTopicNotificationShape {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub resource: Uri,
    pub include: Option<Vec<String>>,
    #[serde(rename = "revInclude")]
    pub rev_include: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SubscriptionTopic {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Uri,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub title: Option<String>,
    #[serde(rename = "derivedFrom")]
    pub derived_from: Option<Vec<Canonical>>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    #[serde(rename = "approvalDate")]
    pub approval_date: Option<Date>,
    #[serde(rename = "lastReviewDate")]
    pub last_review_date: Option<Date>,
    #[serde(rename = "effectivePeriod")]
    pub effective_period: Option<Period>,
    #[serde(rename = "resourceTrigger")]
    pub resource_trigger: Option<Vec<SubscriptionTopicResourceTrigger>>,
    #[serde(rename = "eventTrigger")]
    pub event_trigger: Option<Vec<SubscriptionTopicEventTrigger>>,
    #[serde(rename = "canFilterBy")]
    pub can_filter_by: Option<Vec<SubscriptionTopicCanFilterBy>>,
    #[serde(rename = "notificationShape")]
    pub notification_shape: Option<Vec<SubscriptionTopicNotificationShape>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SubscriptionTopicResourceTriggerQueryCriteria {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub previous: Option<String>,
    #[serde(rename = "resultForCreate")]
    pub result_for_create: Option<Code>,
    pub current: Option<String>,
    #[serde(rename = "resultForDelete")]
    pub result_for_delete: Option<Code>,
    #[serde(rename = "requireBoth")]
    pub require_both: Option<Boolean>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SubscriptionTopicResourceTrigger {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<Markdown>,
    pub resource: Uri,
    #[serde(rename = "supportedInteraction")]
    pub supported_interaction: Option<Vec<Code>>,
    #[serde(rename = "queryCriteria")]
    pub query_criteria: Option<SubscriptionTopicResourceTriggerQueryCriteria>,
    #[serde(rename = "fhirPathCriteria")]
    pub fhir_path_criteria: Option<String>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct SubstanceInstance {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    pub expiry: Option<DateTime>,
    pub quantity: Option<Quantity>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Substance {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Option<Code>,
    pub category: Option<Vec<CodeableConcept>>,
    pub code: CodeableConcept,
    pub description: Option<String>,
    pub instance: Option<Vec<SubstanceInstance>>,
    pub ingredient: Option<Vec<SubstanceIngredient>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubstanceIngredientSubstance {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SubstanceIngredient {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub quantity: Option<Ratio>,
    #[serde(rename = "substanceCodeableConcept")]
    pub substance_codeable_concept: CodeableConcept,
    #[serde(rename = "substanceReference")]
    pub substance_reference: Reference,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct SubstanceDefinitionCode {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub status: Option<CodeableConcept>,
    #[serde(rename = "statusDate")]
    pub status_date: Option<DateTime>,
    pub note: Option<Vec<Annotation>>,
    pub source: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SubstanceDefinitionStructureRepresentation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub representation: Option<String>,
    pub format: Option<CodeableConcept>,
    pub document: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubstanceDefinitionPropertyValue {
    CodeableConcept(CodeableConcept),
    Quantity(Quantity),
    Date(Date),
    Boolean(Boolean),
    Attachment(Attachment),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SubstanceDefinitionProperty {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(rename = "valueCodeableConcept")]
    pub value_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "valueQuantity")]
    pub value_quantity: Option<Quantity>,
    #[serde(rename = "valueDate")]
    pub value_date: Option<Date>,
    #[serde(rename = "valueBoolean")]
    pub value_boolean: Option<Boolean>,
    #[serde(rename = "valueAttachment")]
    pub value_attachment: Option<Attachment>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SubstanceDefinitionName {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub status: Option<CodeableConcept>,
    pub preferred: Option<Boolean>,
    pub language: Option<Vec<CodeableConcept>>,
    pub domain: Option<Vec<CodeableConcept>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub synonym: Option<Vec<SubstanceDefinitionName>>,
    pub translation: Option<Vec<SubstanceDefinitionName>>,
    pub official: Option<Vec<SubstanceDefinitionNameOfficial>>,
    pub source: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubstanceDefinitionMoietyAmount {
    Quantity(Quantity),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SubstanceDefinitionMoiety {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub role: Option<CodeableConcept>,
    pub identifier: Option<Identifier>,
    pub name: Option<String>,
    pub stereochemistry: Option<CodeableConcept>,
    #[serde(rename = "opticalActivity")]
    pub optical_activity: Option<CodeableConcept>,
    #[serde(rename = "molecularFormula")]
    pub molecular_formula: Option<String>,
    #[serde(rename = "amountQuantity")]
    pub amount_quantity: Option<Quantity>,
    #[serde(rename = "amountString")]
    pub amount_string: Option<String>,
    #[serde(rename = "measurementType")]
    pub measurement_type: Option<CodeableConcept>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubstanceDefinitionRelationshipSubstanceDefinition {
    Reference(Reference),
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SubstanceDefinitionRelationshipAmount {
    Quantity(Quantity),
    Ratio(Ratio),
    String(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SubstanceDefinitionRelationship {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "substanceDefinitionReference")]
    pub substance_definition_reference: Option<Reference>,
    #[serde(rename = "substanceDefinitionCodeableConcept")]
    pub substance_definition_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
    #[serde(rename = "isDefining")]
    pub is_defining: Option<Boolean>,
    #[serde(rename = "amountQuantity")]
    pub amount_quantity: Option<Quantity>,
    #[serde(rename = "amountRatio")]
    pub amount_ratio: Option<Ratio>,
    #[serde(rename = "amountString")]
    pub amount_string: Option<String>,
    #[serde(rename = "ratioHighLimitAmount")]
    pub ratio_high_limit_amount: Option<Ratio>,
    pub comparator: Option<CodeableConcept>,
    pub source: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SubstanceDefinition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub status: Option<CodeableConcept>,
    pub classification: Option<Vec<CodeableConcept>>,
    pub domain: Option<CodeableConcept>,
    pub grade: Option<Vec<CodeableConcept>>,
    pub description: Option<Markdown>,
    #[serde(rename = "informationSource")]
    pub information_source: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    pub manufacturer: Option<Vec<Reference>>,
    pub supplier: Option<Vec<Reference>>,
    pub moiety: Option<Vec<SubstanceDefinitionMoiety>>,
    pub property: Option<Vec<SubstanceDefinitionProperty>>,
    #[serde(rename = "molecularWeight")]
    pub molecular_weight: Option<Vec<SubstanceDefinitionMolecularWeight>>,
    pub structure: Option<SubstanceDefinitionStructure>,
    pub code: Option<Vec<SubstanceDefinitionCode>>,
    pub name: Option<Vec<SubstanceDefinitionName>>,
    pub relationship: Option<Vec<SubstanceDefinitionRelationship>>,
    #[serde(rename = "sourceMaterial")]
    pub source_material: Option<SubstanceDefinitionSourceMaterial>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SubstanceDefinitionStructure {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub stereochemistry: Option<CodeableConcept>,
    #[serde(rename = "opticalActivity")]
    pub optical_activity: Option<CodeableConcept>,
    #[serde(rename = "molecularFormula")]
    pub molecular_formula: Option<String>,
    #[serde(rename = "molecularFormulaByMoiety")]
    pub molecular_formula_by_moiety: Option<String>,
    #[serde(rename = "molecularWeight")]
    pub molecular_weight: Option<SubstanceDefinitionMolecularWeight>,
    pub technique: Option<Vec<CodeableConcept>>,
    #[serde(rename = "sourceDocument")]
    pub source_document: Option<Vec<Reference>>,
    pub representation: Option<Vec<SubstanceDefinitionStructureRepresentation>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SubstanceDefinitionNameOfficial {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub authority: Option<CodeableConcept>,
    pub status: Option<CodeableConcept>,
    pub date: Option<DateTime>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SubstanceDefinitionSourceMaterial {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub genus: Option<CodeableConcept>,
    pub species: Option<CodeableConcept>,
    pub part: Option<CodeableConcept>,
    #[serde(rename = "countryOfOrigin")]
    pub country_of_origin: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SubstanceDefinitionMolecularWeight {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub method: Option<CodeableConcept>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    pub amount: Quantity,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SupplyDeliveryOccurrence {
    DateTime(DateTime),
    Period(Period),
    Timing(Timing),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SupplyDelivery {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(rename = "partOf")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Option<Code>,
    pub patient: Option<Reference>,
    #[serde(rename = "type")]
    pub r#type: Option<CodeableConcept>,
    #[serde(rename = "suppliedItem")]
    pub supplied_item: Option<SupplyDeliverySuppliedItem>,
    #[serde(rename = "occurrenceDateTime")]
    pub occurrence_date_time: Option<DateTime>,
    #[serde(rename = "occurrencePeriod")]
    pub occurrence_period: Option<Period>,
    #[serde(rename = "occurrenceTiming")]
    pub occurrence_timing: Option<Timing>,
    pub supplier: Option<Reference>,
    pub destination: Option<Reference>,
    pub receiver: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SupplyDeliverySuppliedItemItem {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SupplyDeliverySuppliedItem {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub quantity: Option<Quantity>,
    #[serde(rename = "itemCodeableConcept")]
    pub item_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "itemReference")]
    pub item_reference: Option<Reference>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SupplyRequestItem {
    CodeableConcept(CodeableConcept),
    Reference(Reference),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SupplyRequestOccurrence {
    DateTime(DateTime),
    Period(Period),
    Timing(Timing),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SupplyRequest {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Option<Code>,
    pub category: Option<CodeableConcept>,
    pub priority: Option<Code>,
    #[serde(rename = "itemCodeableConcept")]
    pub item_codeable_concept: CodeableConcept,
    #[serde(rename = "itemReference")]
    pub item_reference: Reference,
    pub quantity: Quantity,
    pub parameter: Option<Vec<SupplyRequestParameter>>,
    #[serde(rename = "occurrenceDateTime")]
    pub occurrence_date_time: Option<DateTime>,
    #[serde(rename = "occurrencePeriod")]
    pub occurrence_period: Option<Period>,
    #[serde(rename = "occurrenceTiming")]
    pub occurrence_timing: Option<Timing>,
    #[serde(rename = "authoredOn")]
    pub authored_on: Option<DateTime>,
    pub requester: Option<Reference>,
    pub supplier: Option<Vec<Reference>>,
    #[serde(rename = "reasonCode")]
    pub reason_code: Option<Vec<CodeableConcept>>,
    #[serde(rename = "reasonReference")]
    pub reason_reference: Option<Vec<Reference>>,
    #[serde(rename = "deliverFrom")]
    pub deliver_from: Option<Reference>,
    #[serde(rename = "deliverTo")]
    pub deliver_to: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SupplyRequestParameterValue {
    CodeableConcept(CodeableConcept),
    Quantity(Quantity),
    Range(Range),
    Boolean(Boolean),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SupplyRequestParameter {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    #[serde(rename = "valueCodeableConcept")]
    pub value_codeable_concept: Option<CodeableConcept>,
    #[serde(rename = "valueQuantity")]
    pub value_quantity: Option<Quantity>,
    #[serde(rename = "valueRange")]
    pub value_range: Option<Range>,
    #[serde(rename = "valueBoolean")]
    pub value_boolean: Option<Boolean>,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskInputValue {
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
pub struct TaskInput {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
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
pub struct Task {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    #[serde(rename = "instantiatesCanonical")]
    pub instantiates_canonical: Option<Canonical>,
    #[serde(rename = "instantiatesUri")]
    pub instantiates_uri: Option<Uri>,
    #[serde(rename = "basedOn")]
    pub based_on: Option<Vec<Reference>>,
    #[serde(rename = "groupIdentifier")]
    pub group_identifier: Option<Identifier>,
    #[serde(rename = "partOf")]
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    #[serde(rename = "statusReason")]
    pub status_reason: Option<CodeableConcept>,
    #[serde(rename = "businessStatus")]
    pub business_status: Option<CodeableConcept>,
    pub intent: Code,
    pub priority: Option<Code>,
    pub code: Option<CodeableConcept>,
    pub description: Option<String>,
    pub focus: Option<Reference>,
    #[serde(rename = "for")]
    pub r#for: Option<Reference>,
    pub encounter: Option<Reference>,
    #[serde(rename = "executionPeriod")]
    pub execution_period: Option<Period>,
    #[serde(rename = "authoredOn")]
    pub authored_on: Option<DateTime>,
    #[serde(rename = "lastModified")]
    pub last_modified: Option<DateTime>,
    pub requester: Option<Reference>,
    #[serde(rename = "performerType")]
    pub performer_type: Option<Vec<CodeableConcept>>,
    pub owner: Option<Reference>,
    pub location: Option<Reference>,
    #[serde(rename = "reasonCode")]
    pub reason_code: Option<CodeableConcept>,
    #[serde(rename = "reasonReference")]
    pub reason_reference: Option<Reference>,
    pub insurance: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    #[serde(rename = "relevantHistory")]
    pub relevant_history: Option<Vec<Reference>>,
    pub restriction: Option<TaskRestriction>,
    pub input: Option<Vec<TaskInput>>,
    pub output: Option<Vec<TaskOutput>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TaskRestriction {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub repetitions: Option<PositiveInt>,
    pub period: Option<Period>,
    pub recipient: Option<Vec<Reference>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskOutputValue {
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
pub struct TaskOutput {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: CodeableConcept,
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
pub struct TerminologyCapabilitiesSoftware {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub version: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TerminologyCapabilitiesCodeSystemVersionFilter {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub op: Option<Vec<Code>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TerminologyCapabilitiesValidateCode {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub translations: Boolean,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TerminologyCapabilitiesExpansionParameter {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Code,
    pub documentation: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TerminologyCapabilitiesExpansion {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub hierarchical: Option<Boolean>,
    pub paging: Option<Boolean>,
    pub incomplete: Option<Boolean>,
    pub parameter: Option<Vec<TerminologyCapabilitiesExpansionParameter>>,
    #[serde(rename = "textFilter")]
    pub text_filter: Option<Markdown>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TerminologyCapabilitiesCodeSystem {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub uri: Option<Canonical>,
    pub version: Option<Vec<TerminologyCapabilitiesCodeSystemVersion>>,
    pub subsumption: Option<Boolean>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TerminologyCapabilitiesImplementation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: String,
    pub url: Option<Url>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TerminologyCapabilitiesCodeSystemVersion {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<String>,
    #[serde(rename = "isDefault")]
    pub is_default: Option<Boolean>,
    pub compositional: Option<Boolean>,
    pub language: Option<Vec<Code>>,
    pub filter: Option<Vec<TerminologyCapabilitiesCodeSystemVersionFilter>>,
    pub property: Option<Vec<Code>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TerminologyCapabilities {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: DateTime,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    pub kind: Code,
    pub software: Option<TerminologyCapabilitiesSoftware>,
    pub implementation: Option<TerminologyCapabilitiesImplementation>,
    #[serde(rename = "lockedDate")]
    pub locked_date: Option<Boolean>,
    #[serde(rename = "codeSystem")]
    pub code_system: Option<Vec<TerminologyCapabilitiesCodeSystem>>,
    pub expansion: Option<TerminologyCapabilitiesExpansion>,
    #[serde(rename = "codeSearch")]
    pub code_search: Option<Code>,
    #[serde(rename = "validateCode")]
    pub validate_code: Option<TerminologyCapabilitiesValidateCode>,
    pub translation: Option<TerminologyCapabilitiesTranslation>,
    pub closure: Option<TerminologyCapabilitiesClosure>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TerminologyCapabilitiesTranslation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "needsMap")]
    pub needs_map: Boolean,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TerminologyCapabilitiesClosure {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub translation: Option<Boolean>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct TestReportParticipant {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Code,
    pub uri: Uri,
    pub display: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TestReportSetupActionAssert {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub result: Code,
    pub message: Option<Markdown>,
    pub detail: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TestReportSetupActionOperation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub result: Code,
    pub message: Option<Markdown>,
    pub detail: Option<Uri>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TestReportTestAction {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub operation: Option<TestReportSetupActionOperation>,
    pub assert: Option<TestReportSetupActionAssert>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TestReportTeardown {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub action: Option<Vec<TestReportTeardownAction>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TestReportSetupAction {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub operation: Option<TestReportSetupActionOperation>,
    pub assert: Option<TestReportSetupActionAssert>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TestReportTeardownAction {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub operation: TestReportSetupActionOperation,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TestReport {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    pub name: Option<String>,
    pub status: Code,
    #[serde(rename = "testScript")]
    pub test_script: Reference,
    pub result: Code,
    pub score: Option<Decimal>,
    pub tester: Option<String>,
    pub issued: Option<DateTime>,
    pub participant: Option<Vec<TestReportParticipant>>,
    pub setup: Option<TestReportSetup>,
    pub test: Option<Vec<TestReportTest>>,
    pub teardown: Option<TestReportTeardown>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TestReportSetup {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub action: Option<Vec<TestReportSetupAction>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TestReportTest {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub action: Option<Vec<TestReportTestAction>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct TestScriptVariable {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[serde(rename = "defaultValue")]
    pub default_value: Option<String>,
    pub description: Option<String>,
    pub expression: Option<String>,
    #[serde(rename = "headerField")]
    pub header_field: Option<String>,
    pub hint: Option<String>,
    pub path: Option<String>,
    #[serde(rename = "sourceId")]
    pub source_id: Option<Id>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TestScriptMetadataLink {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Uri,
    pub description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TestScriptMetadata {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub link: Option<Vec<TestScriptMetadataLink>>,
    pub capability: Option<Vec<TestScriptMetadataCapability>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TestScriptDestination {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub index: Integer,
    pub profile: Coding,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TestScriptSetupActionOperation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "type")]
    pub r#type: Option<Coding>,
    pub resource: Option<Code>,
    pub label: Option<String>,
    pub description: Option<String>,
    pub accept: Option<Code>,
    #[serde(rename = "contentType")]
    pub content_type: Option<Code>,
    pub destination: Option<Integer>,
    #[serde(rename = "encodeRequestUrl")]
    pub encode_request_url: Boolean,
    pub method: Option<Code>,
    pub origin: Option<Integer>,
    pub params: Option<String>,
    #[serde(rename = "requestHeader")]
    pub request_header: Option<Vec<TestScriptSetupActionOperationRequestHeader>>,
    #[serde(rename = "requestId")]
    pub request_id: Option<Id>,
    #[serde(rename = "responseId")]
    pub response_id: Option<Id>,
    #[serde(rename = "sourceId")]
    pub source_id: Option<Id>,
    #[serde(rename = "targetId")]
    pub target_id: Option<Id>,
    pub url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TestScriptTestAction {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub operation: Option<TestScriptSetupActionOperation>,
    pub assert: Option<TestScriptSetupActionAssert>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TestScriptTeardown {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub action: Option<Vec<TestScriptTeardownAction>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TestScriptSetupAction {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub operation: Option<TestScriptSetupActionOperation>,
    pub assert: Option<TestScriptSetupActionAssert>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TestScriptOrigin {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub index: Integer,
    pub profile: Coding,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TestScriptFixture {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub autocreate: Boolean,
    pub autodelete: Boolean,
    pub resource: Option<Reference>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TestScriptTest {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub action: Option<Vec<TestScriptTestAction>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TestScriptMetadataCapability {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub required: Boolean,
    pub validated: Boolean,
    pub description: Option<String>,
    pub origin: Option<Vec<Integer>>,
    pub destination: Option<Integer>,
    pub link: Option<Vec<Uri>>,
    pub capabilities: Canonical,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TestScript {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Uri,
    pub identifier: Option<Identifier>,
    pub version: Option<String>,
    pub name: String,
    pub title: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    pub origin: Option<Vec<TestScriptOrigin>>,
    pub destination: Option<Vec<TestScriptDestination>>,
    pub metadata: Option<TestScriptMetadata>,
    pub fixture: Option<Vec<TestScriptFixture>>,
    pub profile: Option<Vec<Reference>>,
    pub variable: Option<Vec<TestScriptVariable>>,
    pub setup: Option<TestScriptSetup>,
    pub test: Option<Vec<TestScriptTest>>,
    pub teardown: Option<TestScriptTeardown>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TestScriptSetupActionAssert {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub label: Option<String>,
    pub description: Option<String>,
    pub direction: Option<Code>,
    #[serde(rename = "compareToSourceId")]
    pub compare_to_source_id: Option<String>,
    #[serde(rename = "compareToSourceExpression")]
    pub compare_to_source_expression: Option<String>,
    #[serde(rename = "compareToSourcePath")]
    pub compare_to_source_path: Option<String>,
    #[serde(rename = "contentType")]
    pub content_type: Option<Code>,
    pub expression: Option<String>,
    #[serde(rename = "headerField")]
    pub header_field: Option<String>,
    #[serde(rename = "minimumId")]
    pub minimum_id: Option<String>,
    #[serde(rename = "navigationLinks")]
    pub navigation_links: Option<Boolean>,
    pub operator: Option<Code>,
    pub path: Option<String>,
    #[serde(rename = "requestMethod")]
    pub request_method: Option<Code>,
    #[serde(rename = "requestURL")]
    pub request_u_r_l: Option<String>,
    pub resource: Option<Code>,
    pub response: Option<Code>,
    #[serde(rename = "responseCode")]
    pub response_code: Option<String>,
    #[serde(rename = "sourceId")]
    pub source_id: Option<Id>,
    #[serde(rename = "validateProfileId")]
    pub validate_profile_id: Option<Id>,
    pub value: Option<String>,
    #[serde(rename = "warningOnly")]
    pub warning_only: Boolean,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TestScriptSetup {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub action: Option<Vec<TestScriptSetupAction>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TestScriptSetupActionOperationRequestHeader {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub field: String,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TestScriptTeardownAction {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub operation: TestScriptSetupActionOperation,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct ValueSetComposeIncludeFilter {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub property: Code,
    pub op: Code,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ValueSet {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    #[serde(rename = "useContext")]
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub immutable: Option<Boolean>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    pub compose: Option<ValueSetCompose>,
    pub expansion: Option<ValueSetExpansion>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ValueSetExpansion {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Uri>,
    pub timestamp: DateTime,
    pub total: Option<Integer>,
    pub offset: Option<Integer>,
    pub parameter: Option<Vec<ValueSetExpansionParameter>>,
    pub contains: Option<Vec<ValueSetExpansionContains>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ValueSetExpansionParameterValue {
    String(String),
    Boolean(Boolean),
    Integer(Integer),
    Decimal(Decimal),
    Uri(Uri),
    Code(Code),
    DateTime(DateTime),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ValueSetExpansionParameter {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    #[serde(rename = "valueString")]
    pub value_string: Option<String>,
    #[serde(rename = "valueBoolean")]
    pub value_boolean: Option<Boolean>,
    #[serde(rename = "valueInteger")]
    pub value_integer: Option<Integer>,
    #[serde(rename = "valueDecimal")]
    pub value_decimal: Option<Decimal>,
    #[serde(rename = "valueUri")]
    pub value_uri: Option<Uri>,
    #[serde(rename = "valueCode")]
    pub value_code: Option<Code>,
    #[serde(rename = "valueDateTime")]
    pub value_date_time: Option<DateTime>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ValueSetComposeIncludeConcept {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub display: Option<String>,
    pub designation: Option<Vec<ValueSetComposeIncludeConceptDesignation>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ValueSetCompose {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    #[serde(rename = "lockedDate")]
    pub locked_date: Option<Date>,
    pub inactive: Option<Boolean>,
    pub include: Option<Vec<ValueSetComposeInclude>>,
    pub exclude: Option<Vec<ValueSetComposeInclude>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ValueSetComposeInclude {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub system: Option<Uri>,
    pub version: Option<String>,
    pub concept: Option<Vec<ValueSetComposeIncludeConcept>>,
    pub filter: Option<Vec<ValueSetComposeIncludeFilter>>,
    #[serde(rename = "valueSet")]
    pub value_set: Option<Vec<Canonical>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ValueSetComposeIncludeConceptDesignation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub language: Option<Code>,
    #[serde(rename = "use")]
    pub r#use: Option<Coding>,
    pub value: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ValueSetExpansionContains {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub system: Option<Uri>,
    #[serde(rename = "abstract")]
    pub r#abstract: Option<Boolean>,
    pub inactive: Option<Boolean>,
    pub version: Option<String>,
    pub code: Option<Code>,
    pub display: Option<String>,
    pub designation: Option<Vec<ValueSetComposeIncludeConceptDesignation>>,
    pub contains: Option<Vec<ValueSetExpansionContains>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct VerificationResultPrimarySource {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub who: Option<Reference>,
    #[serde(rename = "type")]
    pub r#type: Option<Vec<CodeableConcept>>,
    #[serde(rename = "communicationMethod")]
    pub communication_method: Option<Vec<CodeableConcept>>,
    #[serde(rename = "validationStatus")]
    pub validation_status: Option<CodeableConcept>,
    #[serde(rename = "validationDate")]
    pub validation_date: Option<DateTime>,
    #[serde(rename = "canPushUpdates")]
    pub can_push_updates: Option<CodeableConcept>,
    #[serde(rename = "pushTypeAvailable")]
    pub push_type_available: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VerificationResultAttestation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub who: Option<Reference>,
    #[serde(rename = "onBehalfOf")]
    pub on_behalf_of: Option<Reference>,
    #[serde(rename = "communicationMethod")]
    pub communication_method: Option<CodeableConcept>,
    pub date: Option<Date>,
    #[serde(rename = "sourceIdentityCertificate")]
    pub source_identity_certificate: Option<String>,
    #[serde(rename = "proxyIdentityCertificate")]
    pub proxy_identity_certificate: Option<String>,
    #[serde(rename = "proxySignature")]
    pub proxy_signature: Option<Signature>,
    #[serde(rename = "sourceSignature")]
    pub source_signature: Option<Signature>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VerificationResultValidator {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub organization: Reference,
    #[serde(rename = "identityCertificate")]
    pub identity_certificate: Option<String>,
    #[serde(rename = "attestationSignature")]
    pub attestation_signature: Option<Signature>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VerificationResult {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub target: Option<Vec<Reference>>,
    #[serde(rename = "targetLocation")]
    pub target_location: Option<Vec<String>>,
    pub need: Option<CodeableConcept>,
    pub status: Code,
    #[serde(rename = "statusDate")]
    pub status_date: Option<DateTime>,
    #[serde(rename = "validationType")]
    pub validation_type: Option<CodeableConcept>,
    #[serde(rename = "validationProcess")]
    pub validation_process: Option<Vec<CodeableConcept>>,
    pub frequency: Option<Timing>,
    #[serde(rename = "lastPerformed")]
    pub last_performed: Option<DateTime>,
    #[serde(rename = "nextScheduled")]
    pub next_scheduled: Option<Date>,
    #[serde(rename = "failureAction")]
    pub failure_action: Option<CodeableConcept>,
    #[serde(rename = "primarySource")]
    pub primary_source: Option<Vec<VerificationResultPrimarySource>>,
    pub attestation: Option<VerificationResultAttestation>,
    pub validator: Option<Vec<VerificationResultValidator>>,
}


#[derive(Debug, Serialize, Deserialize)]
pub struct VisionPrescriptionLensSpecification {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub product: CodeableConcept,
    pub eye: Code,
    pub sphere: Option<Decimal>,
    pub cylinder: Option<Decimal>,
    pub axis: Option<Integer>,
    pub prism: Option<Vec<VisionPrescriptionLensSpecificationPrism>>,
    pub add: Option<Decimal>,
    pub power: Option<Decimal>,
    #[serde(rename = "backCurve")]
    pub back_curve: Option<Decimal>,
    pub diameter: Option<Decimal>,
    pub duration: Option<Quantity>,
    pub color: Option<String>,
    pub brand: Option<String>,
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VisionPrescription {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    #[serde(rename = "implicitRules")]
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub created: DateTime,
    pub patient: Reference,
    pub encounter: Option<Reference>,
    #[serde(rename = "dateWritten")]
    pub date_written: DateTime,
    pub prescriber: Reference,
    #[serde(rename = "lensSpecification")]
    pub lens_specification: Option<Vec<VisionPrescriptionLensSpecification>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VisionPrescriptionLensSpecificationPrism {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    #[serde(rename = "modifierExtension")]
    pub modifier_extension: Option<Vec<Extension>>,
    pub amount: Decimal,
    pub base: Code,
}


#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "resourceType")]
pub enum Resource {
    Account(Account),
    ActivityDefinition(ActivityDefinition),
    AdministrableProductDefinition(AdministrableProductDefinition),
    AdverseEvent(AdverseEvent),
    AllergyIntolerance(AllergyIntolerance),
    Appointment(Appointment),
    AppointmentResponse(AppointmentResponse),
    AuditEvent(AuditEvent),
    Basic(Basic),
    Binary(Binary),
    BiologicallyDerivedProduct(BiologicallyDerivedProduct),
    BodyStructure(BodyStructure),
    Bundle(Bundle),
    CapabilityStatement(CapabilityStatement),
    CarePlan(CarePlan),
    CareTeam(CareTeam),
    CatalogEntry(CatalogEntry),
    ChargeItem(ChargeItem),
    ChargeItemDefinition(ChargeItemDefinition),
    Citation(Citation),
    Claim(Claim),
    ClaimResponse(ClaimResponse),
    ClinicalImpression(ClinicalImpression),
    ClinicalUseDefinition(ClinicalUseDefinition),
    CodeSystem(CodeSystem),
    Communication(Communication),
    CommunicationRequest(CommunicationRequest),
    CompartmentDefinition(CompartmentDefinition),
    Composition(Composition),
    ConceptMap(ConceptMap),
    Condition(Condition),
    Consent(Consent),
    Contract(Contract),
    Coverage(Coverage),
    CoverageEligibilityRequest(CoverageEligibilityRequest),
    CoverageEligibilityResponse(CoverageEligibilityResponse),
    DetectedIssue(DetectedIssue),
    Device(Device),
    DeviceDefinition(DeviceDefinition),
    DeviceMetric(DeviceMetric),
    DeviceRequest(DeviceRequest),
    DeviceUseStatement(DeviceUseStatement),
    DiagnosticReport(DiagnosticReport),
    DocumentManifest(DocumentManifest),
    DocumentReference(DocumentReference),
    Encounter(Encounter),
    Endpoint(Endpoint),
    EnrollmentRequest(EnrollmentRequest),
    EnrollmentResponse(EnrollmentResponse),
    EpisodeOfCare(EpisodeOfCare),
    EventDefinition(EventDefinition),
    Evidence(Evidence),
    EvidenceReport(EvidenceReport),
    EvidenceVariable(EvidenceVariable),
    ExampleScenario(ExampleScenario),
    ExplanationOfBenefit(ExplanationOfBenefit),
    FamilyMemberHistory(FamilyMemberHistory),
    Flag(Flag),
    Goal(Goal),
    GraphDefinition(GraphDefinition),
    Group(Group),
    GuidanceResponse(GuidanceResponse),
    HealthcareService(HealthcareService),
    ImagingStudy(ImagingStudy),
    Immunization(Immunization),
    ImmunizationEvaluation(ImmunizationEvaluation),
    ImmunizationRecommendation(ImmunizationRecommendation),
    ImplementationGuide(ImplementationGuide),
    Ingredient(Ingredient),
    InsurancePlan(InsurancePlan),
    Invoice(Invoice),
    Library(Library),
    Linkage(Linkage),
    List(List),
    Location(Location),
    ManufacturedItemDefinition(ManufacturedItemDefinition),
    Measure(Measure),
    MeasureReport(MeasureReport),
    Media(Media),
    Medication(Medication),
    MedicationAdministration(MedicationAdministration),
    MedicationDispense(MedicationDispense),
    MedicationKnowledge(MedicationKnowledge),
    MedicationRequest(MedicationRequest),
    MedicationStatement(MedicationStatement),
    MedicinalProductDefinition(MedicinalProductDefinition),
    MessageDefinition(MessageDefinition),
    MessageHeader(MessageHeader),
    MolecularSequence(MolecularSequence),
    NamingSystem(NamingSystem),
    NutritionOrder(NutritionOrder),
    NutritionProduct(NutritionProduct),
    Observation(Observation),
    ObservationDefinition(ObservationDefinition),
    OperationDefinition(OperationDefinition),
    OperationOutcome(OperationOutcome),
    Organization(Organization),
    OrganizationAffiliation(OrganizationAffiliation),
    PackagedProductDefinition(PackagedProductDefinition),
    Parameters(Parameters),
    Patient(Patient),
    PaymentNotice(PaymentNotice),
    PaymentReconciliation(PaymentReconciliation),
    Person(Person),
    PlanDefinition(PlanDefinition),
    Practitioner(Practitioner),
    PractitionerRole(PractitionerRole),
    Procedure(Procedure),
    Provenance(Provenance),
    Questionnaire(Questionnaire),
    QuestionnaireResponse(QuestionnaireResponse),
    RegulatedAuthorization(RegulatedAuthorization),
    RelatedPerson(RelatedPerson),
    RequestGroup(RequestGroup),
    ResearchDefinition(ResearchDefinition),
    ResearchElementDefinition(ResearchElementDefinition),
    ResearchStudy(ResearchStudy),
    ResearchSubject(ResearchSubject),
    RiskAssessment(RiskAssessment),
    Schedule(Schedule),
    SearchParameter(SearchParameter),
    ServiceRequest(ServiceRequest),
    Slot(Slot),
    Specimen(Specimen),
    SpecimenDefinition(SpecimenDefinition),
    StructureDefinition(StructureDefinition),
    StructureMap(StructureMap),
    Subscription(Subscription),
    SubscriptionStatus(SubscriptionStatus),
    SubscriptionTopic(SubscriptionTopic),
    Substance(Substance),
    SubstanceDefinition(SubstanceDefinition),
    SupplyDelivery(SupplyDelivery),
    SupplyRequest(SupplyRequest),
    Task(Task),
    TerminologyCapabilities(TerminologyCapabilities),
    TestReport(TestReport),
    TestScript(TestScript),
    ValueSet(ValueSet),
    VerificationResult(VerificationResult),
    VisionPrescription(VisionPrescription),
}


