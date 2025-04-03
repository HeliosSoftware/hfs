use fhir_macro::FhirSerde;
use serde::{Serialize, Deserialize};
use crate::{Element, DecimalElement};

pub type Base64Binary = Element<std::string::String, Extension>;

pub type Boolean = Element<bool, Extension>;

pub type Canonical = Element<std::string::String, Extension>;

pub type Code = Element<std::string::String, Extension>;

pub type Date = Element<std::string::String, Extension>;

pub type DateTime = Element<std::string::String, Extension>;

pub type Decimal = DecimalElement<Extension>;

pub type Id = Element<std::string::String, Extension>;

pub type Instant = Element<std::string::String, Extension>;

pub type Integer = Element<std::primitive::i32, Extension>;

pub type Markdown = Element<std::string::String, Extension>;

pub type Oid = Element<std::string::String, Extension>;

pub type PositiveInt = Element<std::primitive::i32, Extension>;

pub type String = Element<std::string::String, Extension>;

pub type Time = Element<std::string::String, Extension>;

pub type UnsignedInt = Element<std::primitive::i32, Extension>;

pub type Uri = Element<std::string::String, Extension>;

pub type Url = Element<std::string::String, Extension>;

pub type Uuid = Element<std::string::String, Extension>;

pub type Xhtml = Element<std::string::String, Extension>;

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Address {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub r#use: Option<Code>,
    pub r#type: Option<Code>,
    pub text: Option<String>,
    pub line: Option<Vec<String>>,
    pub city: Option<String>,
    pub district: Option<String>,
    pub state: Option<String>,
    pub postal_code: Option<String>,
    pub country: Option<String>,
    pub period: Option<Period>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Age {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<Decimal>,
    pub comparator: Option<Code>,
    pub unit: Option<String>,
    pub system: Option<Uri>,
    pub code: Option<Code>,
}


/// Choice of types for the author[x] field in Annotation
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AnnotationAuthor {
    /// Variant accepting the Reference type.
    #[serde(rename = "authorReference")]
    Reference(Reference),
    /// Variant accepting the String type.
    #[serde(rename = "authorString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Annotation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub author: Option<AnnotationAuthor>,
    pub time: Option<DateTime>,
    pub text: Markdown,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Attachment {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub content_type: Option<Code>,
    pub language: Option<Code>,
    pub data: Option<Base64Binary>,
    pub url: Option<Url>,
    pub size: Option<UnsignedInt>,
    pub hash: Option<Base64Binary>,
    pub title: Option<String>,
    pub creation: Option<DateTime>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CodeableConcept {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub coding: Option<Vec<Coding>>,
    pub text: Option<String>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Coding {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub system: Option<Uri>,
    pub version: Option<String>,
    pub code: Option<Code>,
    pub display: Option<String>,
    pub user_selected: Option<Boolean>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ContactDetail {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub name: Option<String>,
    pub telecom: Option<Vec<ContactPoint>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ContactPoint {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub system: Option<Code>,
    pub value: Option<String>,
    pub r#use: Option<Code>,
    pub rank: Option<PositiveInt>,
    pub period: Option<Period>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Contributor {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub r#type: Code,
    pub name: String,
    pub contact: Option<Vec<ContactDetail>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Count {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<Decimal>,
    pub comparator: Option<Code>,
    pub unit: Option<String>,
    pub system: Option<Uri>,
    pub code: Option<Code>,
}


/// Choice of types for the subject[x] field in DataRequirement
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DataRequirementSubject {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "subjectReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct DataRequirement {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub r#type: Code,
    pub profile: Option<Vec<Canonical>>,
    pub subject: Option<DataRequirementSubject>,
    pub must_support: Option<Vec<String>>,
    pub code_filter: Option<Vec<DataRequirementCodeFilter>>,
    pub date_filter: Option<Vec<DataRequirementDateFilter>>,
    pub limit: Option<PositiveInt>,
    pub sort: Option<Vec<DataRequirementSort>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct DataRequirementSort {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub path: String,
    pub direction: Code,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct DataRequirementCodeFilter {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub path: Option<String>,
    pub search_param: Option<String>,
    pub value_set: Option<Canonical>,
    pub code: Option<Vec<Coding>>,
}

/// Choice of types for the value[x] field in DataRequirementDateFilter
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DataRequirementDateFilterValue {
    /// Variant accepting the DateTime type.
    #[serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "valuePeriod")]
    Period(Period),
    /// Variant accepting the Duration type.
    #[serde(rename = "valueDuration")]
    Duration(Duration),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct DataRequirementDateFilter {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub path: Option<String>,
    pub search_param: Option<String>,
    pub value: Option<DataRequirementDateFilterValue>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Distance {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<Decimal>,
    pub comparator: Option<Code>,
    pub unit: Option<String>,
    pub system: Option<Uri>,
    pub code: Option<Code>,
}


/// Choice of types for the asNeeded[x] field in Dosage
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DosageAsNeeded {
    /// Variant accepting the Boolean type.
    #[serde(rename = "asNeededBoolean")]
    Boolean(Boolean),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "asNeededCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Dosage {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: Option<Integer>,
    pub text: Option<String>,
    pub additional_instruction: Option<Vec<CodeableConcept>>,
    pub patient_instruction: Option<String>,
    pub timing: Option<Timing>,
    pub as_needed: Option<DosageAsNeeded>,
    pub site: Option<CodeableConcept>,
    pub route: Option<CodeableConcept>,
    pub method: Option<CodeableConcept>,
    pub dose_and_rate: Option<Vec<DosageDoseAndRate>>,
    pub max_dose_per_period: Option<Ratio>,
    pub max_dose_per_administration: Option<Quantity>,
    pub max_dose_per_lifetime: Option<Quantity>,
}

/// Choice of types for the dose[x] field in DosageDoseAndRate
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DosageDoseAndRateDose {
    /// Variant accepting the Range type.
    #[serde(rename = "doseRange")]
    Range(Range),
    /// Variant accepting the Quantity type.
    #[serde(rename = "doseQuantity")]
    Quantity(Quantity),
}

/// Choice of types for the rate[x] field in DosageDoseAndRate
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DosageDoseAndRateRate {
    /// Variant accepting the Ratio type.
    #[serde(rename = "rateRatio")]
    Ratio(Ratio),
    /// Variant accepting the Range type.
    #[serde(rename = "rateRange")]
    Range(Range),
    /// Variant accepting the Quantity type.
    #[serde(rename = "rateQuantity")]
    Quantity(Quantity),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct DosageDoseAndRate {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub r#type: Option<CodeableConcept>,
    pub dose: Option<DosageDoseAndRateDose>,
    pub rate: Option<DosageDoseAndRateRate>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Duration {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<Decimal>,
    pub comparator: Option<Code>,
    pub unit: Option<String>,
    pub system: Option<Uri>,
    pub code: Option<Code>,
}


/// Choice of types for the defaultValue[x] field in ElementDefinition
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ElementDefinitionDefaultValue {
    /// Variant accepting the Base64Binary type.
    #[serde(rename = "defaultValueBase64Binary")]
    Base64Binary(Base64Binary),
    /// Variant accepting the Boolean type.
    #[serde(rename = "defaultValueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Canonical type.
    #[serde(rename = "defaultValueCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Code type.
    #[serde(rename = "defaultValueCode")]
    Code(Code),
    /// Variant accepting the Date type.
    #[serde(rename = "defaultValueDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[serde(rename = "defaultValueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Decimal type.
    #[serde(rename = "defaultValueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Id type.
    #[serde(rename = "defaultValueId")]
    Id(Id),
    /// Variant accepting the Instant type.
    #[serde(rename = "defaultValueInstant")]
    Instant(Instant),
    /// Variant accepting the Integer type.
    #[serde(rename = "defaultValueInteger")]
    Integer(Integer),
    /// Variant accepting the Markdown type.
    #[serde(rename = "defaultValueMarkdown")]
    Markdown(Markdown),
    /// Variant accepting the Oid type.
    #[serde(rename = "defaultValueOid")]
    Oid(Oid),
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "defaultValuePositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[serde(rename = "defaultValueString")]
    String(String),
    /// Variant accepting the Time type.
    #[serde(rename = "defaultValueTime")]
    Time(Time),
    /// Variant accepting the UnsignedInt type.
    #[serde(rename = "defaultValueUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Uri type.
    #[serde(rename = "defaultValueUri")]
    Uri(Uri),
    /// Variant accepting the Url type.
    #[serde(rename = "defaultValueUrl")]
    Url(Url),
    /// Variant accepting the Uuid type.
    #[serde(rename = "defaultValueUuid")]
    Uuid(Uuid),
    /// Variant accepting the Address type.
    #[serde(rename = "defaultValueAddress")]
    Address(Address),
    /// Variant accepting the Age type.
    #[serde(rename = "defaultValueAge")]
    Age(Age),
    /// Variant accepting the Annotation type.
    #[serde(rename = "defaultValueAnnotation")]
    Annotation(Annotation),
    /// Variant accepting the Attachment type.
    #[serde(rename = "defaultValueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "defaultValueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Coding type.
    #[serde(rename = "defaultValueCoding")]
    Coding(Coding),
    /// Variant accepting the ContactPoint type.
    #[serde(rename = "defaultValueContactPoint")]
    ContactPoint(ContactPoint),
    /// Variant accepting the Count type.
    #[serde(rename = "defaultValueCount")]
    Count(Count),
    /// Variant accepting the Distance type.
    #[serde(rename = "defaultValueDistance")]
    Distance(Distance),
    /// Variant accepting the Duration type.
    #[serde(rename = "defaultValueDuration")]
    Duration(Duration),
    /// Variant accepting the HumanName type.
    #[serde(rename = "defaultValueHumanName")]
    HumanName(HumanName),
    /// Variant accepting the Identifier type.
    #[serde(rename = "defaultValueIdentifier")]
    Identifier(Identifier),
    /// Variant accepting the Money type.
    #[serde(rename = "defaultValueMoney")]
    Money(Money),
    /// Variant accepting the Period type.
    #[serde(rename = "defaultValuePeriod")]
    Period(Period),
    /// Variant accepting the Quantity type.
    #[serde(rename = "defaultValueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[serde(rename = "defaultValueRange")]
    Range(Range),
    /// Variant accepting the Ratio type.
    #[serde(rename = "defaultValueRatio")]
    Ratio(Ratio),
    /// Variant accepting the Reference type.
    #[serde(rename = "defaultValueReference")]
    Reference(Reference),
    /// Variant accepting the SampledData type.
    #[serde(rename = "defaultValueSampledData")]
    SampledData(SampledData),
    /// Variant accepting the Signature type.
    #[serde(rename = "defaultValueSignature")]
    Signature(Signature),
    /// Variant accepting the Timing type.
    #[serde(rename = "defaultValueTiming")]
    Timing(Timing),
    /// Variant accepting the ContactDetail type.
    #[serde(rename = "defaultValueContactDetail")]
    ContactDetail(ContactDetail),
    /// Variant accepting the Contributor type.
    #[serde(rename = "defaultValueContributor")]
    Contributor(Contributor),
    /// Variant accepting the DataRequirement type.
    #[serde(rename = "defaultValueDataRequirement")]
    DataRequirement(DataRequirement),
    /// Variant accepting the Expression type.
    #[serde(rename = "defaultValueExpression")]
    Expression(Expression),
    /// Variant accepting the ParameterDefinition type.
    #[serde(rename = "defaultValueParameterDefinition")]
    ParameterDefinition(ParameterDefinition),
    /// Variant accepting the RelatedArtifact type.
    #[serde(rename = "defaultValueRelatedArtifact")]
    RelatedArtifact(RelatedArtifact),
    /// Variant accepting the TriggerDefinition type.
    #[serde(rename = "defaultValueTriggerDefinition")]
    TriggerDefinition(TriggerDefinition),
    /// Variant accepting the UsageContext type.
    #[serde(rename = "defaultValueUsageContext")]
    UsageContext(UsageContext),
    /// Variant accepting the Dosage type.
    #[serde(rename = "defaultValueDosage")]
    Dosage(Dosage),
    /// Variant accepting the Meta type.
    #[serde(rename = "defaultValueMeta")]
    Meta(Meta),
}

/// Choice of types for the fixed[x] field in ElementDefinition
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ElementDefinitionFixed {
    /// Variant accepting the Base64Binary type.
    #[serde(rename = "fixedBase64Binary")]
    Base64Binary(Base64Binary),
    /// Variant accepting the Boolean type.
    #[serde(rename = "fixedBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Canonical type.
    #[serde(rename = "fixedCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Code type.
    #[serde(rename = "fixedCode")]
    Code(Code),
    /// Variant accepting the Date type.
    #[serde(rename = "fixedDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[serde(rename = "fixedDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Decimal type.
    #[serde(rename = "fixedDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Id type.
    #[serde(rename = "fixedId")]
    Id(Id),
    /// Variant accepting the Instant type.
    #[serde(rename = "fixedInstant")]
    Instant(Instant),
    /// Variant accepting the Integer type.
    #[serde(rename = "fixedInteger")]
    Integer(Integer),
    /// Variant accepting the Markdown type.
    #[serde(rename = "fixedMarkdown")]
    Markdown(Markdown),
    /// Variant accepting the Oid type.
    #[serde(rename = "fixedOid")]
    Oid(Oid),
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "fixedPositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[serde(rename = "fixedString")]
    String(String),
    /// Variant accepting the Time type.
    #[serde(rename = "fixedTime")]
    Time(Time),
    /// Variant accepting the UnsignedInt type.
    #[serde(rename = "fixedUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Uri type.
    #[serde(rename = "fixedUri")]
    Uri(Uri),
    /// Variant accepting the Url type.
    #[serde(rename = "fixedUrl")]
    Url(Url),
    /// Variant accepting the Uuid type.
    #[serde(rename = "fixedUuid")]
    Uuid(Uuid),
    /// Variant accepting the Address type.
    #[serde(rename = "fixedAddress")]
    Address(Address),
    /// Variant accepting the Age type.
    #[serde(rename = "fixedAge")]
    Age(Age),
    /// Variant accepting the Annotation type.
    #[serde(rename = "fixedAnnotation")]
    Annotation(Annotation),
    /// Variant accepting the Attachment type.
    #[serde(rename = "fixedAttachment")]
    Attachment(Attachment),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "fixedCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Coding type.
    #[serde(rename = "fixedCoding")]
    Coding(Coding),
    /// Variant accepting the ContactPoint type.
    #[serde(rename = "fixedContactPoint")]
    ContactPoint(ContactPoint),
    /// Variant accepting the Count type.
    #[serde(rename = "fixedCount")]
    Count(Count),
    /// Variant accepting the Distance type.
    #[serde(rename = "fixedDistance")]
    Distance(Distance),
    /// Variant accepting the Duration type.
    #[serde(rename = "fixedDuration")]
    Duration(Duration),
    /// Variant accepting the HumanName type.
    #[serde(rename = "fixedHumanName")]
    HumanName(HumanName),
    /// Variant accepting the Identifier type.
    #[serde(rename = "fixedIdentifier")]
    Identifier(Identifier),
    /// Variant accepting the Money type.
    #[serde(rename = "fixedMoney")]
    Money(Money),
    /// Variant accepting the Period type.
    #[serde(rename = "fixedPeriod")]
    Period(Period),
    /// Variant accepting the Quantity type.
    #[serde(rename = "fixedQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[serde(rename = "fixedRange")]
    Range(Range),
    /// Variant accepting the Ratio type.
    #[serde(rename = "fixedRatio")]
    Ratio(Ratio),
    /// Variant accepting the Reference type.
    #[serde(rename = "fixedReference")]
    Reference(Reference),
    /// Variant accepting the SampledData type.
    #[serde(rename = "fixedSampledData")]
    SampledData(SampledData),
    /// Variant accepting the Signature type.
    #[serde(rename = "fixedSignature")]
    Signature(Signature),
    /// Variant accepting the Timing type.
    #[serde(rename = "fixedTiming")]
    Timing(Timing),
    /// Variant accepting the ContactDetail type.
    #[serde(rename = "fixedContactDetail")]
    ContactDetail(ContactDetail),
    /// Variant accepting the Contributor type.
    #[serde(rename = "fixedContributor")]
    Contributor(Contributor),
    /// Variant accepting the DataRequirement type.
    #[serde(rename = "fixedDataRequirement")]
    DataRequirement(DataRequirement),
    /// Variant accepting the Expression type.
    #[serde(rename = "fixedExpression")]
    Expression(Expression),
    /// Variant accepting the ParameterDefinition type.
    #[serde(rename = "fixedParameterDefinition")]
    ParameterDefinition(ParameterDefinition),
    /// Variant accepting the RelatedArtifact type.
    #[serde(rename = "fixedRelatedArtifact")]
    RelatedArtifact(RelatedArtifact),
    /// Variant accepting the TriggerDefinition type.
    #[serde(rename = "fixedTriggerDefinition")]
    TriggerDefinition(TriggerDefinition),
    /// Variant accepting the UsageContext type.
    #[serde(rename = "fixedUsageContext")]
    UsageContext(UsageContext),
    /// Variant accepting the Dosage type.
    #[serde(rename = "fixedDosage")]
    Dosage(Dosage),
    /// Variant accepting the Meta type.
    #[serde(rename = "fixedMeta")]
    Meta(Meta),
}

/// Choice of types for the pattern[x] field in ElementDefinition
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ElementDefinitionPattern {
    /// Variant accepting the Base64Binary type.
    #[serde(rename = "patternBase64Binary")]
    Base64Binary(Base64Binary),
    /// Variant accepting the Boolean type.
    #[serde(rename = "patternBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Canonical type.
    #[serde(rename = "patternCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Code type.
    #[serde(rename = "patternCode")]
    Code(Code),
    /// Variant accepting the Date type.
    #[serde(rename = "patternDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[serde(rename = "patternDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Decimal type.
    #[serde(rename = "patternDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Id type.
    #[serde(rename = "patternId")]
    Id(Id),
    /// Variant accepting the Instant type.
    #[serde(rename = "patternInstant")]
    Instant(Instant),
    /// Variant accepting the Integer type.
    #[serde(rename = "patternInteger")]
    Integer(Integer),
    /// Variant accepting the Markdown type.
    #[serde(rename = "patternMarkdown")]
    Markdown(Markdown),
    /// Variant accepting the Oid type.
    #[serde(rename = "patternOid")]
    Oid(Oid),
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "patternPositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[serde(rename = "patternString")]
    String(String),
    /// Variant accepting the Time type.
    #[serde(rename = "patternTime")]
    Time(Time),
    /// Variant accepting the UnsignedInt type.
    #[serde(rename = "patternUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Uri type.
    #[serde(rename = "patternUri")]
    Uri(Uri),
    /// Variant accepting the Url type.
    #[serde(rename = "patternUrl")]
    Url(Url),
    /// Variant accepting the Uuid type.
    #[serde(rename = "patternUuid")]
    Uuid(Uuid),
    /// Variant accepting the Address type.
    #[serde(rename = "patternAddress")]
    Address(Address),
    /// Variant accepting the Age type.
    #[serde(rename = "patternAge")]
    Age(Age),
    /// Variant accepting the Annotation type.
    #[serde(rename = "patternAnnotation")]
    Annotation(Annotation),
    /// Variant accepting the Attachment type.
    #[serde(rename = "patternAttachment")]
    Attachment(Attachment),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "patternCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Coding type.
    #[serde(rename = "patternCoding")]
    Coding(Coding),
    /// Variant accepting the ContactPoint type.
    #[serde(rename = "patternContactPoint")]
    ContactPoint(ContactPoint),
    /// Variant accepting the Count type.
    #[serde(rename = "patternCount")]
    Count(Count),
    /// Variant accepting the Distance type.
    #[serde(rename = "patternDistance")]
    Distance(Distance),
    /// Variant accepting the Duration type.
    #[serde(rename = "patternDuration")]
    Duration(Duration),
    /// Variant accepting the HumanName type.
    #[serde(rename = "patternHumanName")]
    HumanName(HumanName),
    /// Variant accepting the Identifier type.
    #[serde(rename = "patternIdentifier")]
    Identifier(Identifier),
    /// Variant accepting the Money type.
    #[serde(rename = "patternMoney")]
    Money(Money),
    /// Variant accepting the Period type.
    #[serde(rename = "patternPeriod")]
    Period(Period),
    /// Variant accepting the Quantity type.
    #[serde(rename = "patternQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[serde(rename = "patternRange")]
    Range(Range),
    /// Variant accepting the Ratio type.
    #[serde(rename = "patternRatio")]
    Ratio(Ratio),
    /// Variant accepting the Reference type.
    #[serde(rename = "patternReference")]
    Reference(Reference),
    /// Variant accepting the SampledData type.
    #[serde(rename = "patternSampledData")]
    SampledData(SampledData),
    /// Variant accepting the Signature type.
    #[serde(rename = "patternSignature")]
    Signature(Signature),
    /// Variant accepting the Timing type.
    #[serde(rename = "patternTiming")]
    Timing(Timing),
    /// Variant accepting the ContactDetail type.
    #[serde(rename = "patternContactDetail")]
    ContactDetail(ContactDetail),
    /// Variant accepting the Contributor type.
    #[serde(rename = "patternContributor")]
    Contributor(Contributor),
    /// Variant accepting the DataRequirement type.
    #[serde(rename = "patternDataRequirement")]
    DataRequirement(DataRequirement),
    /// Variant accepting the Expression type.
    #[serde(rename = "patternExpression")]
    Expression(Expression),
    /// Variant accepting the ParameterDefinition type.
    #[serde(rename = "patternParameterDefinition")]
    ParameterDefinition(ParameterDefinition),
    /// Variant accepting the RelatedArtifact type.
    #[serde(rename = "patternRelatedArtifact")]
    RelatedArtifact(RelatedArtifact),
    /// Variant accepting the TriggerDefinition type.
    #[serde(rename = "patternTriggerDefinition")]
    TriggerDefinition(TriggerDefinition),
    /// Variant accepting the UsageContext type.
    #[serde(rename = "patternUsageContext")]
    UsageContext(UsageContext),
    /// Variant accepting the Dosage type.
    #[serde(rename = "patternDosage")]
    Dosage(Dosage),
    /// Variant accepting the Meta type.
    #[serde(rename = "patternMeta")]
    Meta(Meta),
}

/// Choice of types for the minValue[x] field in ElementDefinition
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ElementDefinitionMinValue {
    /// Variant accepting the Date type.
    #[serde(rename = "minValueDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[serde(rename = "minValueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Instant type.
    #[serde(rename = "minValueInstant")]
    Instant(Instant),
    /// Variant accepting the Time type.
    #[serde(rename = "minValueTime")]
    Time(Time),
    /// Variant accepting the Decimal type.
    #[serde(rename = "minValueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Integer type.
    #[serde(rename = "minValueInteger")]
    Integer(Integer),
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "minValuePositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the UnsignedInt type.
    #[serde(rename = "minValueUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Quantity type.
    #[serde(rename = "minValueQuantity")]
    Quantity(Quantity),
}

/// Choice of types for the maxValue[x] field in ElementDefinition
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ElementDefinitionMaxValue {
    /// Variant accepting the Date type.
    #[serde(rename = "maxValueDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[serde(rename = "maxValueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Instant type.
    #[serde(rename = "maxValueInstant")]
    Instant(Instant),
    /// Variant accepting the Time type.
    #[serde(rename = "maxValueTime")]
    Time(Time),
    /// Variant accepting the Decimal type.
    #[serde(rename = "maxValueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Integer type.
    #[serde(rename = "maxValueInteger")]
    Integer(Integer),
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "maxValuePositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the UnsignedInt type.
    #[serde(rename = "maxValueUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Quantity type.
    #[serde(rename = "maxValueQuantity")]
    Quantity(Quantity),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ElementDefinition {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub path: String,
    pub representation: Option<Vec<Code>>,
    pub slice_name: Option<String>,
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
    pub content_reference: Option<Uri>,
    pub r#type: Option<Vec<ElementDefinitionType>>,
    pub default_value: Option<ElementDefinitionDefaultValue>,
    pub meaning_when_missing: Option<Markdown>,
    pub order_meaning: Option<String>,
    pub fixed: Option<ElementDefinitionFixed>,
    pub pattern: Option<ElementDefinitionPattern>,
    pub example: Option<Vec<ElementDefinitionExample>>,
    pub min_value: Option<ElementDefinitionMinValue>,
    pub max_value: Option<ElementDefinitionMaxValue>,
    pub max_length: Option<Integer>,
    pub condition: Option<Vec<Id>>,
    pub constraint: Option<Vec<ElementDefinitionConstraint>>,
    pub must_support: Option<Boolean>,
    pub is_modifier: Option<Boolean>,
    pub is_modifier_reason: Option<String>,
    pub is_summary: Option<Boolean>,
    pub binding: Option<ElementDefinitionBinding>,
    pub mapping: Option<Vec<ElementDefinitionMapping>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ElementDefinitionSlicing {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub discriminator: Option<Vec<ElementDefinitionSlicingDiscriminator>>,
    pub description: Option<String>,
    pub ordered: Option<Boolean>,
    pub rules: Code,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
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

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ElementDefinitionType {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub code: Uri,
    pub profile: Option<Vec<Canonical>>,
    pub target_profile: Option<Vec<Canonical>>,
    pub aggregation: Option<Vec<Code>>,
    pub versioning: Option<Code>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ElementDefinitionSlicingDiscriminator {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub r#type: Code,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ElementDefinitionMapping {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub identity: Id,
    pub language: Option<Code>,
    pub map: String,
    pub comment: Option<String>,
}

/// Choice of types for the value[x] field in ElementDefinitionExample
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ElementDefinitionExampleValue {
    /// Variant accepting the Base64Binary type.
    #[serde(rename = "valueBase64Binary")]
    Base64Binary(Base64Binary),
    /// Variant accepting the Boolean type.
    #[serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Canonical type.
    #[serde(rename = "valueCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Code type.
    #[serde(rename = "valueCode")]
    Code(Code),
    /// Variant accepting the Date type.
    #[serde(rename = "valueDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Decimal type.
    #[serde(rename = "valueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Id type.
    #[serde(rename = "valueId")]
    Id(Id),
    /// Variant accepting the Instant type.
    #[serde(rename = "valueInstant")]
    Instant(Instant),
    /// Variant accepting the Integer type.
    #[serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Markdown type.
    #[serde(rename = "valueMarkdown")]
    Markdown(Markdown),
    /// Variant accepting the Oid type.
    #[serde(rename = "valueOid")]
    Oid(Oid),
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "valuePositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Time type.
    #[serde(rename = "valueTime")]
    Time(Time),
    /// Variant accepting the UnsignedInt type.
    #[serde(rename = "valueUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Uri type.
    #[serde(rename = "valueUri")]
    Uri(Uri),
    /// Variant accepting the Url type.
    #[serde(rename = "valueUrl")]
    Url(Url),
    /// Variant accepting the Uuid type.
    #[serde(rename = "valueUuid")]
    Uuid(Uuid),
    /// Variant accepting the Address type.
    #[serde(rename = "valueAddress")]
    Address(Address),
    /// Variant accepting the Age type.
    #[serde(rename = "valueAge")]
    Age(Age),
    /// Variant accepting the Annotation type.
    #[serde(rename = "valueAnnotation")]
    Annotation(Annotation),
    /// Variant accepting the Attachment type.
    #[serde(rename = "valueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Coding type.
    #[serde(rename = "valueCoding")]
    Coding(Coding),
    /// Variant accepting the ContactPoint type.
    #[serde(rename = "valueContactPoint")]
    ContactPoint(ContactPoint),
    /// Variant accepting the Count type.
    #[serde(rename = "valueCount")]
    Count(Count),
    /// Variant accepting the Distance type.
    #[serde(rename = "valueDistance")]
    Distance(Distance),
    /// Variant accepting the Duration type.
    #[serde(rename = "valueDuration")]
    Duration(Duration),
    /// Variant accepting the HumanName type.
    #[serde(rename = "valueHumanName")]
    HumanName(HumanName),
    /// Variant accepting the Identifier type.
    #[serde(rename = "valueIdentifier")]
    Identifier(Identifier),
    /// Variant accepting the Money type.
    #[serde(rename = "valueMoney")]
    Money(Money),
    /// Variant accepting the Period type.
    #[serde(rename = "valuePeriod")]
    Period(Period),
    /// Variant accepting the Quantity type.
    #[serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[serde(rename = "valueRange")]
    Range(Range),
    /// Variant accepting the Ratio type.
    #[serde(rename = "valueRatio")]
    Ratio(Ratio),
    /// Variant accepting the Reference type.
    #[serde(rename = "valueReference")]
    Reference(Reference),
    /// Variant accepting the SampledData type.
    #[serde(rename = "valueSampledData")]
    SampledData(SampledData),
    /// Variant accepting the Signature type.
    #[serde(rename = "valueSignature")]
    Signature(Signature),
    /// Variant accepting the Timing type.
    #[serde(rename = "valueTiming")]
    Timing(Timing),
    /// Variant accepting the ContactDetail type.
    #[serde(rename = "valueContactDetail")]
    ContactDetail(ContactDetail),
    /// Variant accepting the Contributor type.
    #[serde(rename = "valueContributor")]
    Contributor(Contributor),
    /// Variant accepting the DataRequirement type.
    #[serde(rename = "valueDataRequirement")]
    DataRequirement(DataRequirement),
    /// Variant accepting the Expression type.
    #[serde(rename = "valueExpression")]
    Expression(Expression),
    /// Variant accepting the ParameterDefinition type.
    #[serde(rename = "valueParameterDefinition")]
    ParameterDefinition(ParameterDefinition),
    /// Variant accepting the RelatedArtifact type.
    #[serde(rename = "valueRelatedArtifact")]
    RelatedArtifact(RelatedArtifact),
    /// Variant accepting the TriggerDefinition type.
    #[serde(rename = "valueTriggerDefinition")]
    TriggerDefinition(TriggerDefinition),
    /// Variant accepting the UsageContext type.
    #[serde(rename = "valueUsageContext")]
    UsageContext(UsageContext),
    /// Variant accepting the Dosage type.
    #[serde(rename = "valueDosage")]
    Dosage(Dosage),
    /// Variant accepting the Meta type.
    #[serde(rename = "valueMeta")]
    Meta(Meta),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ElementDefinitionExample {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub label: String,
    pub value: Option<ElementDefinitionExampleValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ElementDefinitionBinding {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub strength: Code,
    pub description: Option<String>,
    pub value_set: Option<Canonical>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ElementDefinitionBase {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub path: String,
    pub min: UnsignedInt,
    pub max: String,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Expression {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    pub name: Option<Id>,
    pub language: Code,
    pub expression: Option<String>,
    pub reference: Option<Uri>,
}


/// Choice of types for the value[x] field in Extension
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ExtensionValue {
    /// Variant accepting the Base64Binary type.
    #[serde(rename = "valueBase64Binary")]
    Base64Binary(Base64Binary),
    /// Variant accepting the Boolean type.
    #[serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Canonical type.
    #[serde(rename = "valueCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Code type.
    #[serde(rename = "valueCode")]
    Code(Code),
    /// Variant accepting the Date type.
    #[serde(rename = "valueDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Decimal type.
    #[serde(rename = "valueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Id type.
    #[serde(rename = "valueId")]
    Id(Id),
    /// Variant accepting the Instant type.
    #[serde(rename = "valueInstant")]
    Instant(Instant),
    /// Variant accepting the Integer type.
    #[serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Markdown type.
    #[serde(rename = "valueMarkdown")]
    Markdown(Markdown),
    /// Variant accepting the Oid type.
    #[serde(rename = "valueOid")]
    Oid(Oid),
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "valuePositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Time type.
    #[serde(rename = "valueTime")]
    Time(Time),
    /// Variant accepting the UnsignedInt type.
    #[serde(rename = "valueUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Uri type.
    #[serde(rename = "valueUri")]
    Uri(Uri),
    /// Variant accepting the Url type.
    #[serde(rename = "valueUrl")]
    Url(Url),
    /// Variant accepting the Uuid type.
    #[serde(rename = "valueUuid")]
    Uuid(Uuid),
    /// Variant accepting the Address type.
    #[serde(rename = "valueAddress")]
    Address(Address),
    /// Variant accepting the Age type.
    #[serde(rename = "valueAge")]
    Age(Age),
    /// Variant accepting the Annotation type.
    #[serde(rename = "valueAnnotation")]
    Annotation(Annotation),
    /// Variant accepting the Attachment type.
    #[serde(rename = "valueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Coding type.
    #[serde(rename = "valueCoding")]
    Coding(Coding),
    /// Variant accepting the ContactPoint type.
    #[serde(rename = "valueContactPoint")]
    ContactPoint(ContactPoint),
    /// Variant accepting the Count type.
    #[serde(rename = "valueCount")]
    Count(Count),
    /// Variant accepting the Distance type.
    #[serde(rename = "valueDistance")]
    Distance(Distance),
    /// Variant accepting the Duration type.
    #[serde(rename = "valueDuration")]
    Duration(Duration),
    /// Variant accepting the HumanName type.
    #[serde(rename = "valueHumanName")]
    HumanName(HumanName),
    /// Variant accepting the Identifier type.
    #[serde(rename = "valueIdentifier")]
    Identifier(Identifier),
    /// Variant accepting the Money type.
    #[serde(rename = "valueMoney")]
    Money(Money),
    /// Variant accepting the Period type.
    #[serde(rename = "valuePeriod")]
    Period(Period),
    /// Variant accepting the Quantity type.
    #[serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[serde(rename = "valueRange")]
    Range(Range),
    /// Variant accepting the Ratio type.
    #[serde(rename = "valueRatio")]
    Ratio(Ratio),
    /// Variant accepting the Reference type.
    #[serde(rename = "valueReference")]
    Reference(Reference),
    /// Variant accepting the SampledData type.
    #[serde(rename = "valueSampledData")]
    SampledData(SampledData),
    /// Variant accepting the Signature type.
    #[serde(rename = "valueSignature")]
    Signature(Signature),
    /// Variant accepting the Timing type.
    #[serde(rename = "valueTiming")]
    Timing(Timing),
    /// Variant accepting the ContactDetail type.
    #[serde(rename = "valueContactDetail")]
    ContactDetail(ContactDetail),
    /// Variant accepting the Contributor type.
    #[serde(rename = "valueContributor")]
    Contributor(Contributor),
    /// Variant accepting the DataRequirement type.
    #[serde(rename = "valueDataRequirement")]
    DataRequirement(DataRequirement),
    /// Variant accepting the Expression type.
    #[serde(rename = "valueExpression")]
    Expression(Expression),
    /// Variant accepting the ParameterDefinition type.
    #[serde(rename = "valueParameterDefinition")]
    ParameterDefinition(ParameterDefinition),
    /// Variant accepting the RelatedArtifact type.
    #[serde(rename = "valueRelatedArtifact")]
    RelatedArtifact(RelatedArtifact),
    /// Variant accepting the TriggerDefinition type.
    #[serde(rename = "valueTriggerDefinition")]
    TriggerDefinition(TriggerDefinition),
    /// Variant accepting the UsageContext type.
    #[serde(rename = "valueUsageContext")]
    UsageContext(UsageContext),
    /// Variant accepting the Dosage type.
    #[serde(rename = "valueDosage")]
    Dosage(Dosage),
    /// Variant accepting the Meta type.
    #[serde(rename = "valueMeta")]
    Meta(Meta),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Extension {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub url: std::string::String,
    pub value: Option<ExtensionValue>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct HumanName {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub r#use: Option<Code>,
    pub text: Option<String>,
    pub family: Option<String>,
    pub given: Option<Vec<String>>,
    pub prefix: Option<Vec<String>>,
    pub suffix: Option<Vec<String>>,
    pub period: Option<Period>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Identifier {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub r#use: Option<Code>,
    pub r#type: Option<CodeableConcept>,
    pub system: Option<Uri>,
    pub value: Option<String>,
    pub period: Option<Period>,
    pub assigner: Option<Box<Reference>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MarketingStatus {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub country: CodeableConcept,
    pub jurisdiction: Option<CodeableConcept>,
    pub status: CodeableConcept,
    pub date_range: Period,
    pub restore_date: Option<DateTime>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Meta {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub version_id: Option<Id>,
    pub last_updated: Option<Instant>,
    pub source: Option<Uri>,
    pub profile: Option<Vec<Canonical>>,
    pub security: Option<Vec<Coding>>,
    pub tag: Option<Vec<Coding>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Money {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<Decimal>,
    pub currency: Option<Code>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Narrative {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub status: Code,
    pub div: Xhtml,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ParameterDefinition {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub name: Option<Code>,
    pub r#use: Code,
    pub min: Option<Integer>,
    pub max: Option<String>,
    pub documentation: Option<String>,
    pub r#type: Code,
    pub profile: Option<Canonical>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Period {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub start: Option<DateTime>,
    pub end: Option<DateTime>,
}


/// Choice of types for the age[x] field in Population
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PopulationAge {
    /// Variant accepting the Range type.
    #[serde(rename = "ageRange")]
    Range(Range),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "ageCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Population {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub age: Option<PopulationAge>,
    pub gender: Option<CodeableConcept>,
    pub race: Option<CodeableConcept>,
    pub physiological_condition: Option<CodeableConcept>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ProdCharacteristic {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub height: Option<Quantity>,
    pub width: Option<Quantity>,
    pub depth: Option<Quantity>,
    pub weight: Option<Quantity>,
    pub nominal_volume: Option<Quantity>,
    pub external_diameter: Option<Quantity>,
    pub shape: Option<String>,
    pub color: Option<Vec<String>>,
    pub imprint: Option<Vec<String>>,
    pub image: Option<Vec<Attachment>>,
    pub scoring: Option<CodeableConcept>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ProductShelfLife {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    pub r#type: CodeableConcept,
    pub period: Quantity,
    pub special_precautions_for_storage: Option<Vec<CodeableConcept>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Quantity {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub value: Option<Decimal>,
    pub comparator: Option<Code>,
    pub unit: Option<String>,
    pub system: Option<Uri>,
    pub code: Option<Code>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Range {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub low: Option<Quantity>,
    pub high: Option<Quantity>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Ratio {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub numerator: Option<Quantity>,
    pub denominator: Option<Quantity>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Reference {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub reference: Option<String>,
    pub r#type: Option<Uri>,
    pub identifier: Option<Box<Identifier>>,
    pub display: Option<String>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct RelatedArtifact {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub r#type: Code,
    pub label: Option<String>,
    pub display: Option<String>,
    pub citation: Option<Markdown>,
    pub url: Option<Url>,
    pub document: Option<Attachment>,
    pub resource: Option<Canonical>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SampledData {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub origin: Quantity,
    pub period: Decimal,
    pub factor: Option<Decimal>,
    pub lower_limit: Option<Decimal>,
    pub upper_limit: Option<Decimal>,
    pub dimensions: PositiveInt,
    pub data: Option<String>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Signature {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub r#type: Option<Vec<Coding>>,
    pub when: Instant,
    pub who: Reference,
    pub on_behalf_of: Option<Reference>,
    pub target_format: Option<Code>,
    pub sig_format: Option<Code>,
    pub data: Option<Base64Binary>,
}


/// Choice of types for the amount[x] field in SubstanceAmount
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SubstanceAmountAmount {
    /// Variant accepting the Quantity type.
    #[serde(rename = "amountQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[serde(rename = "amountRange")]
    Range(Range),
    /// Variant accepting the String type.
    #[serde(rename = "amountString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstanceAmount {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub amount: Option<SubstanceAmountAmount>,
    pub amount_type: Option<CodeableConcept>,
    pub amount_text: Option<String>,
    pub reference_range: Option<SubstanceAmountReferenceRange>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstanceAmountReferenceRange {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub low_limit: Option<Quantity>,
    pub high_limit: Option<Quantity>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Timing {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub event: Option<Vec<DateTime>>,
    pub repeat: Option<TimingRepeat>,
    pub code: Option<CodeableConcept>,
}

/// Choice of types for the bounds[x] field in TimingRepeat
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TimingRepeatBounds {
    /// Variant accepting the Duration type.
    #[serde(rename = "boundsDuration")]
    Duration(Duration),
    /// Variant accepting the Range type.
    #[serde(rename = "boundsRange")]
    Range(Range),
    /// Variant accepting the Period type.
    #[serde(rename = "boundsPeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TimingRepeat {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub bounds: Option<TimingRepeatBounds>,
    pub count: Option<PositiveInt>,
    pub count_max: Option<PositiveInt>,
    pub duration: Option<Decimal>,
    pub duration_max: Option<Decimal>,
    pub duration_unit: Option<Code>,
    pub frequency: Option<PositiveInt>,
    pub frequency_max: Option<PositiveInt>,
    pub period: Option<Decimal>,
    pub period_max: Option<Decimal>,
    pub period_unit: Option<Code>,
    pub day_of_week: Option<Vec<Code>>,
    pub time_of_day: Option<Vec<Time>>,
    pub when: Option<Vec<Code>>,
    pub offset: Option<UnsignedInt>,
}


/// Choice of types for the timing[x] field in TriggerDefinition
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TriggerDefinitionTiming {
    /// Variant accepting the Timing type.
    #[serde(rename = "timingTiming")]
    Timing(Timing),
    /// Variant accepting the Reference type.
    #[serde(rename = "timingReference")]
    Reference(Reference),
    /// Variant accepting the Date type.
    #[serde(rename = "timingDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[serde(rename = "timingDateTime")]
    DateTime(DateTime),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TriggerDefinition {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub r#type: Code,
    pub name: Option<String>,
    pub timing: Option<TriggerDefinitionTiming>,
    pub data: Option<Vec<DataRequirement>>,
    pub condition: Option<Expression>,
}


/// Choice of types for the value[x] field in UsageContext
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum UsageContextValue {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Quantity type.
    #[serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[serde(rename = "valueRange")]
    Range(Range),
    /// Variant accepting the Reference type.
    #[serde(rename = "valueReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct UsageContext {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub code: Coding,
    pub value: Option<UsageContextValue>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct AccountGuarantor {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub party: Reference,
    pub on_hold: Option<Boolean>,
    pub period: Option<Period>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct AccountCoverage {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub coverage: Reference,
    pub priority: Option<PositiveInt>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Account {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub r#type: Option<CodeableConcept>,
    pub name: Option<String>,
    pub subject: Option<Vec<Reference>>,
    pub service_period: Option<Period>,
    pub coverage: Option<Vec<AccountCoverage>>,
    pub owner: Option<Reference>,
    pub description: Option<String>,
    pub guarantor: Option<Vec<AccountGuarantor>>,
    pub part_of: Option<Reference>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ActivityDefinitionDynamicValue {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub path: String,
    pub expression: Expression,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ActivityDefinitionParticipant {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Code,
    pub role: Option<CodeableConcept>,
}

/// Choice of types for the subject[x] field in ActivityDefinition
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ActivityDefinitionSubject {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "subjectReference")]
    Reference(Reference),
}

/// Choice of types for the timing[x] field in ActivityDefinition
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ActivityDefinitionTiming {
    /// Variant accepting the Timing type.
    #[serde(rename = "timingTiming")]
    Timing(Timing),
    /// Variant accepting the DateTime type.
    #[serde(rename = "timingDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Age type.
    #[serde(rename = "timingAge")]
    Age(Age),
    /// Variant accepting the Period type.
    #[serde(rename = "timingPeriod")]
    Period(Period),
    /// Variant accepting the Range type.
    #[serde(rename = "timingRange")]
    Range(Range),
    /// Variant accepting the Duration type.
    #[serde(rename = "timingDuration")]
    Duration(Duration),
}

/// Choice of types for the product[x] field in ActivityDefinition
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ActivityDefinitionProduct {
    /// Variant accepting the Reference type.
    #[serde(rename = "productReference")]
    Reference(Reference),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "productCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ActivityDefinition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub subject: Option<ActivityDefinitionSubject>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub usage: Option<String>,
    pub copyright: Option<Markdown>,
    pub approval_date: Option<Date>,
    pub last_review_date: Option<Date>,
    pub effective_period: Option<Period>,
    pub topic: Option<Vec<CodeableConcept>>,
    pub author: Option<Vec<ContactDetail>>,
    pub editor: Option<Vec<ContactDetail>>,
    pub reviewer: Option<Vec<ContactDetail>>,
    pub endorser: Option<Vec<ContactDetail>>,
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    pub library: Option<Vec<Canonical>>,
    pub kind: Option<Code>,
    pub profile: Option<Canonical>,
    pub code: Option<CodeableConcept>,
    pub intent: Option<Code>,
    pub priority: Option<Code>,
    pub do_not_perform: Option<Boolean>,
    pub timing: Option<ActivityDefinitionTiming>,
    pub location: Option<Reference>,
    pub participant: Option<Vec<ActivityDefinitionParticipant>>,
    pub product: Option<ActivityDefinitionProduct>,
    pub quantity: Option<Quantity>,
    pub dosage: Option<Vec<Dosage>>,
    pub body_site: Option<Vec<CodeableConcept>>,
    pub specimen_requirement: Option<Vec<Reference>>,
    pub observation_requirement: Option<Vec<Reference>>,
    pub observation_result_requirement: Option<Vec<Reference>>,
    pub transform: Option<Canonical>,
    pub dynamic_value: Option<Vec<ActivityDefinitionDynamicValue>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct AdverseEvent {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    pub actuality: Code,
    pub category: Option<Vec<CodeableConcept>>,
    pub event: Option<CodeableConcept>,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    pub date: Option<DateTime>,
    pub detected: Option<DateTime>,
    pub recorded_date: Option<DateTime>,
    pub resulting_condition: Option<Vec<Reference>>,
    pub location: Option<Reference>,
    pub seriousness: Option<CodeableConcept>,
    pub severity: Option<CodeableConcept>,
    pub outcome: Option<CodeableConcept>,
    pub recorder: Option<Reference>,
    pub contributor: Option<Vec<Reference>>,
    pub suspect_entity: Option<Vec<AdverseEventSuspectEntity>>,
    pub subject_medical_history: Option<Vec<Reference>>,
    pub reference_document: Option<Vec<Reference>>,
    pub study: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct AdverseEventSuspectEntity {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub instance: Reference,
    pub causality: Option<Vec<AdverseEventSuspectEntityCausality>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct AdverseEventSuspectEntityCausality {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub assessment: Option<CodeableConcept>,
    pub product_relatedness: Option<String>,
    pub author: Option<Reference>,
    pub method: Option<CodeableConcept>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct AllergyIntoleranceReaction {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub substance: Option<CodeableConcept>,
    pub manifestation: Option<Vec<CodeableConcept>>,
    pub description: Option<String>,
    pub onset: Option<DateTime>,
    pub severity: Option<Code>,
    pub exposure_route: Option<CodeableConcept>,
    pub note: Option<Vec<Annotation>>,
}

/// Choice of types for the onset[x] field in AllergyIntolerance
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AllergyIntoleranceOnset {
    /// Variant accepting the DateTime type.
    #[serde(rename = "onsetDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Age type.
    #[serde(rename = "onsetAge")]
    Age(Age),
    /// Variant accepting the Period type.
    #[serde(rename = "onsetPeriod")]
    Period(Period),
    /// Variant accepting the Range type.
    #[serde(rename = "onsetRange")]
    Range(Range),
    /// Variant accepting the String type.
    #[serde(rename = "onsetString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct AllergyIntolerance {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub clinical_status: Option<CodeableConcept>,
    pub verification_status: Option<CodeableConcept>,
    pub r#type: Option<Code>,
    pub category: Option<Vec<Code>>,
    pub criticality: Option<Code>,
    pub code: Option<CodeableConcept>,
    pub patient: Reference,
    pub encounter: Option<Reference>,
    pub onset: Option<AllergyIntoleranceOnset>,
    pub recorded_date: Option<DateTime>,
    pub recorder: Option<Reference>,
    pub asserter: Option<Reference>,
    pub last_occurrence: Option<DateTime>,
    pub note: Option<Vec<Annotation>>,
    pub reaction: Option<Vec<AllergyIntoleranceReaction>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct AppointmentParticipant {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Option<Vec<CodeableConcept>>,
    pub actor: Option<Reference>,
    pub required: Option<Code>,
    pub status: Code,
    pub period: Option<Period>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Appointment {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub cancelation_reason: Option<CodeableConcept>,
    pub service_category: Option<Vec<CodeableConcept>>,
    pub service_type: Option<Vec<CodeableConcept>>,
    pub specialty: Option<Vec<CodeableConcept>>,
    pub appointment_type: Option<CodeableConcept>,
    pub reason_code: Option<Vec<CodeableConcept>>,
    pub reason_reference: Option<Vec<Reference>>,
    pub priority: Option<UnsignedInt>,
    pub description: Option<String>,
    pub supporting_information: Option<Vec<Reference>>,
    pub start: Option<Instant>,
    pub end: Option<Instant>,
    pub minutes_duration: Option<PositiveInt>,
    pub slot: Option<Vec<Reference>>,
    pub created: Option<DateTime>,
    pub comment: Option<String>,
    pub patient_instruction: Option<String>,
    pub based_on: Option<Vec<Reference>>,
    pub participant: Option<Vec<AppointmentParticipant>>,
    pub requested_period: Option<Vec<Period>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct AppointmentResponse {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub appointment: Reference,
    pub start: Option<Instant>,
    pub end: Option<Instant>,
    pub participant_type: Option<Vec<CodeableConcept>>,
    pub actor: Option<Reference>,
    pub participant_status: Code,
    pub comment: Option<String>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct AuditEventEntity {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub what: Option<Reference>,
    pub r#type: Option<Coding>,
    pub role: Option<Coding>,
    pub lifecycle: Option<Coding>,
    pub security_label: Option<Vec<Coding>>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub query: Option<Base64Binary>,
    pub detail: Option<Vec<AuditEventEntityDetail>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct AuditEventSource {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub site: Option<String>,
    pub observer: Reference,
    pub r#type: Option<Vec<Coding>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct AuditEventAgent {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Option<CodeableConcept>,
    pub role: Option<Vec<CodeableConcept>>,
    pub who: Option<Reference>,
    pub alt_id: Option<String>,
    pub name: Option<String>,
    pub requestor: Boolean,
    pub location: Option<Reference>,
    pub policy: Option<Vec<Uri>>,
    pub media: Option<Coding>,
    pub network: Option<AuditEventAgentNetwork>,
    pub purpose_of_use: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct AuditEvent {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Coding,
    pub subtype: Option<Vec<Coding>>,
    pub action: Option<Code>,
    pub period: Option<Period>,
    pub recorded: Instant,
    pub outcome: Option<Code>,
    pub outcome_desc: Option<String>,
    pub purpose_of_event: Option<Vec<CodeableConcept>>,
    pub agent: Option<Vec<AuditEventAgent>>,
    pub source: AuditEventSource,
    pub entity: Option<Vec<AuditEventEntity>>,
}

/// Choice of types for the value[x] field in AuditEventEntityDetail
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AuditEventEntityDetailValue {
    /// Variant accepting the String type.
    #[serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Base64Binary type.
    #[serde(rename = "valueBase64Binary")]
    Base64Binary(Base64Binary),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct AuditEventEntityDetail {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: String,
    pub value: Option<AuditEventEntityDetailValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct AuditEventAgentNetwork {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub address: Option<String>,
    pub r#type: Option<Code>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Basic {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub code: CodeableConcept,
    pub subject: Option<Reference>,
    pub created: Option<Date>,
    pub author: Option<Reference>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Binary {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub content_type: Code,
    pub security_context: Option<Reference>,
    pub data: Option<Base64Binary>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct BiologicallyDerivedProduct {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub product_category: Option<Code>,
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

/// Choice of types for the collected[x] field in BiologicallyDerivedProductCollection
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum BiologicallyDerivedProductCollectionCollected {
    /// Variant accepting the DateTime type.
    #[serde(rename = "collectedDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "collectedPeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct BiologicallyDerivedProductCollection {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub collector: Option<Reference>,
    pub source: Option<Reference>,
    pub collected: Option<BiologicallyDerivedProductCollectionCollected>,
}

/// Choice of types for the time[x] field in BiologicallyDerivedProductManipulation
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum BiologicallyDerivedProductManipulationTime {
    /// Variant accepting the DateTime type.
    #[serde(rename = "timeDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "timePeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct BiologicallyDerivedProductManipulation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    pub time: Option<BiologicallyDerivedProductManipulationTime>,
}

/// Choice of types for the time[x] field in BiologicallyDerivedProductProcessing
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum BiologicallyDerivedProductProcessingTime {
    /// Variant accepting the DateTime type.
    #[serde(rename = "timeDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "timePeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct BiologicallyDerivedProductProcessing {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    pub procedure: Option<CodeableConcept>,
    pub additive: Option<Reference>,
    pub time: Option<BiologicallyDerivedProductProcessingTime>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct BiologicallyDerivedProductStorage {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    pub temperature: Option<Decimal>,
    pub scale: Option<Code>,
    pub duration: Option<Period>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct BodyStructure {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub active: Option<Boolean>,
    pub morphology: Option<CodeableConcept>,
    pub location: Option<CodeableConcept>,
    pub location_qualifier: Option<Vec<CodeableConcept>>,
    pub description: Option<String>,
    pub image: Option<Vec<Attachment>>,
    pub patient: Reference,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct BundleLink {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub relation: String,
    pub url: Uri,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct BundleEntryResponse {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub status: String,
    pub location: Option<Uri>,
    pub etag: Option<String>,
    pub last_modified: Option<Instant>,
    pub outcome: Option<Resource>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Bundle {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub identifier: Option<Identifier>,
    pub r#type: Code,
    pub timestamp: Option<Instant>,
    pub total: Option<UnsignedInt>,
    pub link: Option<Vec<BundleLink>>,
    pub entry: Option<Vec<BundleEntry>>,
    pub signature: Option<Signature>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct BundleEntrySearch {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub mode: Option<Code>,
    pub score: Option<Decimal>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct BundleEntryRequest {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub method: Code,
    pub url: Uri,
    pub if_none_match: Option<String>,
    pub if_modified_since: Option<Instant>,
    pub if_match: Option<String>,
    pub if_none_exist: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct BundleEntry {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub link: Option<Vec<BundleLink>>,
    pub full_url: Option<Uri>,
    pub resource: Option<Resource>,
    pub search: Option<BundleEntrySearch>,
    pub request: Option<BundleEntryRequest>,
    pub response: Option<BundleEntryResponse>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CapabilityStatementRest {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub mode: Code,
    pub documentation: Option<Markdown>,
    pub security: Option<CapabilityStatementRestSecurity>,
    pub resource: Option<Vec<CapabilityStatementRestResource>>,
    pub interaction: Option<Vec<CapabilityStatementRestInteraction>>,
    pub search_param: Option<Vec<CapabilityStatementRestResourceSearchParam>>,
    pub operation: Option<Vec<CapabilityStatementRestResourceOperation>>,
    pub compartment: Option<Vec<Canonical>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CapabilityStatementMessagingEndpoint {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub protocol: Coding,
    pub address: Url,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CapabilityStatementRestResourceInteraction {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub documentation: Option<Markdown>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CapabilityStatementRestSecurity {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub cors: Option<Boolean>,
    pub service: Option<Vec<CodeableConcept>>,
    pub description: Option<Markdown>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CapabilityStatementRestResource {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Code,
    pub profile: Option<Canonical>,
    pub supported_profile: Option<Vec<Canonical>>,
    pub documentation: Option<Markdown>,
    pub interaction: Option<Vec<CapabilityStatementRestResourceInteraction>>,
    pub versioning: Option<Code>,
    pub read_history: Option<Boolean>,
    pub update_create: Option<Boolean>,
    pub conditional_create: Option<Boolean>,
    pub conditional_read: Option<Code>,
    pub conditional_update: Option<Boolean>,
    pub conditional_delete: Option<Code>,
    pub reference_policy: Option<Vec<Code>>,
    pub search_include: Option<Vec<String>>,
    pub search_rev_include: Option<Vec<String>>,
    pub search_param: Option<Vec<CapabilityStatementRestResourceSearchParam>>,
    pub operation: Option<Vec<CapabilityStatementRestResourceOperation>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CapabilityStatementRestResourceSearchParam {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub definition: Option<Canonical>,
    pub r#type: Code,
    pub documentation: Option<Markdown>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CapabilityStatementMessagingSupportedMessage {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub mode: Code,
    pub definition: Canonical,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CapabilityStatementDocument {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub mode: Code,
    pub documentation: Option<Markdown>,
    pub profile: Canonical,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CapabilityStatementSoftware {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub version: Option<String>,
    pub release_date: Option<DateTime>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CapabilityStatementImplementation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: String,
    pub url: Option<Url>,
    pub custodian: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CapabilityStatementRestResourceOperation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub definition: Canonical,
    pub documentation: Option<Markdown>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CapabilityStatementRestInteraction {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub documentation: Option<Markdown>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CapabilityStatementMessaging {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub endpoint: Option<Vec<CapabilityStatementMessagingEndpoint>>,
    pub reliable_cache: Option<UnsignedInt>,
    pub documentation: Option<Markdown>,
    pub supported_message: Option<Vec<CapabilityStatementMessagingSupportedMessage>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CapabilityStatement {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
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
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    pub kind: Code,
    pub instantiates: Option<Vec<Canonical>>,
    pub imports: Option<Vec<Canonical>>,
    pub software: Option<CapabilityStatementSoftware>,
    pub implementation: Option<CapabilityStatementImplementation>,
    pub fhir_version: Code,
    pub format: Option<Vec<Code>>,
    pub patch_format: Option<Vec<Code>>,
    pub implementation_guide: Option<Vec<Canonical>>,
    pub rest: Option<Vec<CapabilityStatementRest>>,
    pub messaging: Option<Vec<CapabilityStatementMessaging>>,
    pub document: Option<Vec<CapabilityStatementDocument>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CarePlanActivity {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub outcome_codeable_concept: Option<Vec<CodeableConcept>>,
    pub outcome_reference: Option<Vec<Reference>>,
    pub progress: Option<Vec<Annotation>>,
    pub reference: Option<Reference>,
    pub detail: Option<CarePlanActivityDetail>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CarePlan {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub instantiates_canonical: Option<Vec<Canonical>>,
    pub instantiates_uri: Option<Vec<Uri>>,
    pub based_on: Option<Vec<Reference>>,
    pub replaces: Option<Vec<Reference>>,
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
    pub care_team: Option<Vec<Reference>>,
    pub addresses: Option<Vec<Reference>>,
    pub supporting_info: Option<Vec<Reference>>,
    pub goal: Option<Vec<Reference>>,
    pub activity: Option<Vec<CarePlanActivity>>,
    pub note: Option<Vec<Annotation>>,
}

/// Choice of types for the scheduled[x] field in CarePlanActivityDetail
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CarePlanActivityDetailScheduled {
    /// Variant accepting the Timing type.
    #[serde(rename = "scheduledTiming")]
    Timing(Timing),
    /// Variant accepting the Period type.
    #[serde(rename = "scheduledPeriod")]
    Period(Period),
    /// Variant accepting the String type.
    #[serde(rename = "scheduledString")]
    String(String),
}

/// Choice of types for the product[x] field in CarePlanActivityDetail
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CarePlanActivityDetailProduct {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "productCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "productReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CarePlanActivityDetail {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub kind: Option<Code>,
    pub instantiates_canonical: Option<Vec<Canonical>>,
    pub instantiates_uri: Option<Vec<Uri>>,
    pub code: Option<CodeableConcept>,
    pub reason_code: Option<Vec<CodeableConcept>>,
    pub reason_reference: Option<Vec<Reference>>,
    pub goal: Option<Vec<Reference>>,
    pub status: Code,
    pub status_reason: Option<CodeableConcept>,
    pub do_not_perform: Option<Boolean>,
    pub scheduled: Option<CarePlanActivityDetailScheduled>,
    pub location: Option<Reference>,
    pub performer: Option<Vec<Reference>>,
    pub product: Option<CarePlanActivityDetailProduct>,
    pub daily_amount: Option<Quantity>,
    pub quantity: Option<Quantity>,
    pub description: Option<String>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CareTeamParticipant {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub role: Option<Vec<CodeableConcept>>,
    pub member: Option<Reference>,
    pub on_behalf_of: Option<Reference>,
    pub period: Option<Period>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CareTeam {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Option<Code>,
    pub category: Option<Vec<CodeableConcept>>,
    pub name: Option<String>,
    pub subject: Option<Reference>,
    pub encounter: Option<Reference>,
    pub period: Option<Period>,
    pub participant: Option<Vec<CareTeamParticipant>>,
    pub reason_code: Option<Vec<CodeableConcept>>,
    pub reason_reference: Option<Vec<Reference>>,
    pub managing_organization: Option<Vec<Reference>>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub note: Option<Vec<Annotation>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CatalogEntry {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub r#type: Option<CodeableConcept>,
    pub orderable: Boolean,
    pub referenced_item: Reference,
    pub additional_identifier: Option<Vec<Identifier>>,
    pub classification: Option<Vec<CodeableConcept>>,
    pub status: Option<Code>,
    pub validity_period: Option<Period>,
    pub valid_to: Option<DateTime>,
    pub last_updated: Option<DateTime>,
    pub additional_characteristic: Option<Vec<CodeableConcept>>,
    pub additional_classification: Option<Vec<CodeableConcept>>,
    pub related_entry: Option<Vec<CatalogEntryRelatedEntry>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CatalogEntryRelatedEntry {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub relationtype: Code,
    pub item: Reference,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ChargeItemPerformer {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
}

/// Choice of types for the occurrence[x] field in ChargeItem
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ChargeItemOccurrence {
    /// Variant accepting the DateTime type.
    #[serde(rename = "occurrenceDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "occurrencePeriod")]
    Period(Period),
    /// Variant accepting the Timing type.
    #[serde(rename = "occurrenceTiming")]
    Timing(Timing),
}

/// Choice of types for the product[x] field in ChargeItem
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ChargeItemProduct {
    /// Variant accepting the Reference type.
    #[serde(rename = "productReference")]
    Reference(Reference),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "productCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ChargeItem {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub definition_uri: Option<Vec<Uri>>,
    pub definition_canonical: Option<Vec<Canonical>>,
    pub status: Code,
    pub part_of: Option<Vec<Reference>>,
    pub code: CodeableConcept,
    pub subject: Reference,
    pub context: Option<Reference>,
    pub occurrence: Option<ChargeItemOccurrence>,
    pub performer: Option<Vec<ChargeItemPerformer>>,
    pub performing_organization: Option<Reference>,
    pub requesting_organization: Option<Reference>,
    pub cost_center: Option<Reference>,
    pub quantity: Option<Quantity>,
    pub bodysite: Option<Vec<CodeableConcept>>,
    pub factor_override: Option<Decimal>,
    pub price_override: Option<Money>,
    pub override_reason: Option<String>,
    pub enterer: Option<Reference>,
    pub entered_date: Option<DateTime>,
    pub reason: Option<Vec<CodeableConcept>>,
    pub service: Option<Vec<Reference>>,
    pub product: Option<ChargeItemProduct>,
    pub account: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    pub supporting_information: Option<Vec<Reference>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ChargeItemDefinitionApplicability {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    pub language: Option<String>,
    pub expression: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ChargeItemDefinition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Uri,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub title: Option<String>,
    pub derived_from_uri: Option<Vec<Uri>>,
    pub part_of: Option<Vec<Canonical>>,
    pub replaces: Option<Vec<Canonical>>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub copyright: Option<Markdown>,
    pub approval_date: Option<Date>,
    pub last_review_date: Option<Date>,
    pub effective_period: Option<Period>,
    pub code: Option<CodeableConcept>,
    pub instance: Option<Vec<Reference>>,
    pub applicability: Option<Vec<ChargeItemDefinitionApplicability>>,
    pub property_group: Option<Vec<ChargeItemDefinitionPropertyGroup>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ChargeItemDefinitionPropertyGroupPriceComponent {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Code,
    pub code: Option<CodeableConcept>,
    pub factor: Option<Decimal>,
    pub amount: Option<Money>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ChargeItemDefinitionPropertyGroup {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub applicability: Option<Vec<ChargeItemDefinitionApplicability>>,
    pub price_component: Option<Vec<ChargeItemDefinitionPropertyGroupPriceComponent>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ClaimCareTeam {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub provider: Reference,
    pub responsible: Option<Boolean>,
    pub role: Option<CodeableConcept>,
    pub qualification: Option<CodeableConcept>,
}

/// Choice of types for the timing[x] field in ClaimSupportingInfo
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ClaimSupportingInfoTiming {
    /// Variant accepting the Date type.
    #[serde(rename = "timingDate")]
    Date(Date),
    /// Variant accepting the Period type.
    #[serde(rename = "timingPeriod")]
    Period(Period),
}

/// Choice of types for the value[x] field in ClaimSupportingInfo
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ClaimSupportingInfoValue {
    /// Variant accepting the Boolean type.
    #[serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the String type.
    #[serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Quantity type.
    #[serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Attachment type.
    #[serde(rename = "valueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[serde(rename = "valueReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ClaimSupportingInfo {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub category: CodeableConcept,
    pub code: Option<CodeableConcept>,
    pub timing: Option<ClaimSupportingInfoTiming>,
    pub value: Option<ClaimSupportingInfoValue>,
    pub reason: Option<CodeableConcept>,
}

/// Choice of types for the diagnosis[x] field in ClaimDiagnosis
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ClaimDiagnosisDiagnosis {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "diagnosisCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "diagnosisReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ClaimDiagnosis {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub diagnosis: Option<ClaimDiagnosisDiagnosis>,
    pub r#type: Option<Vec<CodeableConcept>>,
    pub on_admission: Option<CodeableConcept>,
    pub package_code: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ClaimRelated {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub claim: Option<Reference>,
    pub relationship: Option<CodeableConcept>,
    pub reference: Option<Identifier>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ClaimInsurance {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub focal: Boolean,
    pub identifier: Option<Identifier>,
    pub coverage: Reference,
    pub business_arrangement: Option<String>,
    pub pre_auth_ref: Option<Vec<String>>,
    pub claim_response: Option<Reference>,
}

/// Choice of types for the location[x] field in ClaimAccident
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ClaimAccidentLocation {
    /// Variant accepting the Address type.
    #[serde(rename = "locationAddress")]
    Address(Address),
    /// Variant accepting the Reference type.
    #[serde(rename = "locationReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ClaimAccident {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub date: Date,
    pub r#type: Option<CodeableConcept>,
    pub location: Option<ClaimAccidentLocation>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Claim {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub r#type: CodeableConcept,
    pub sub_type: Option<CodeableConcept>,
    pub r#use: Code,
    pub patient: Reference,
    pub billable_period: Option<Period>,
    pub created: DateTime,
    pub enterer: Option<Reference>,
    pub insurer: Option<Reference>,
    pub provider: Reference,
    pub priority: CodeableConcept,
    pub funds_reserve: Option<CodeableConcept>,
    pub related: Option<Vec<ClaimRelated>>,
    pub prescription: Option<Reference>,
    pub original_prescription: Option<Reference>,
    pub payee: Option<ClaimPayee>,
    pub referral: Option<Reference>,
    pub facility: Option<Reference>,
    pub care_team: Option<Vec<ClaimCareTeam>>,
    pub supporting_info: Option<Vec<ClaimSupportingInfo>>,
    pub diagnosis: Option<Vec<ClaimDiagnosis>>,
    pub procedure: Option<Vec<ClaimProcedure>>,
    pub insurance: Option<Vec<ClaimInsurance>>,
    pub accident: Option<ClaimAccident>,
    pub item: Option<Vec<ClaimItem>>,
    pub total: Option<Money>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ClaimItemDetailSubDetail {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub revenue: Option<CodeableConcept>,
    pub category: Option<CodeableConcept>,
    pub product_or_service: CodeableConcept,
    pub modifier: Option<Vec<CodeableConcept>>,
    pub program_code: Option<Vec<CodeableConcept>>,
    pub quantity: Option<Quantity>,
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub net: Option<Money>,
    pub udi: Option<Vec<Reference>>,
}

/// Choice of types for the procedure[x] field in ClaimProcedure
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ClaimProcedureProcedure {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "procedureCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "procedureReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ClaimProcedure {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub r#type: Option<Vec<CodeableConcept>>,
    pub date: Option<DateTime>,
    pub procedure: Option<ClaimProcedureProcedure>,
    pub udi: Option<Vec<Reference>>,
}

/// Choice of types for the serviced[x] field in ClaimItem
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ClaimItemServiced {
    /// Variant accepting the Date type.
    #[serde(rename = "servicedDate")]
    Date(Date),
    /// Variant accepting the Period type.
    #[serde(rename = "servicedPeriod")]
    Period(Period),
}

/// Choice of types for the location[x] field in ClaimItem
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ClaimItemLocation {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "locationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Address type.
    #[serde(rename = "locationAddress")]
    Address(Address),
    /// Variant accepting the Reference type.
    #[serde(rename = "locationReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ClaimItem {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub care_team_sequence: Option<Vec<PositiveInt>>,
    pub diagnosis_sequence: Option<Vec<PositiveInt>>,
    pub procedure_sequence: Option<Vec<PositiveInt>>,
    pub information_sequence: Option<Vec<PositiveInt>>,
    pub revenue: Option<CodeableConcept>,
    pub category: Option<CodeableConcept>,
    pub product_or_service: CodeableConcept,
    pub modifier: Option<Vec<CodeableConcept>>,
    pub program_code: Option<Vec<CodeableConcept>>,
    pub serviced: Option<ClaimItemServiced>,
    pub location: Option<ClaimItemLocation>,
    pub quantity: Option<Quantity>,
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub net: Option<Money>,
    pub udi: Option<Vec<Reference>>,
    pub body_site: Option<CodeableConcept>,
    pub sub_site: Option<Vec<CodeableConcept>>,
    pub encounter: Option<Vec<Reference>>,
    pub detail: Option<Vec<ClaimItemDetail>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ClaimItemDetail {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub revenue: Option<CodeableConcept>,
    pub category: Option<CodeableConcept>,
    pub product_or_service: CodeableConcept,
    pub modifier: Option<Vec<CodeableConcept>>,
    pub program_code: Option<Vec<CodeableConcept>>,
    pub quantity: Option<Quantity>,
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub net: Option<Money>,
    pub udi: Option<Vec<Reference>>,
    pub sub_detail: Option<Vec<ClaimItemDetailSubDetail>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ClaimPayee {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: CodeableConcept,
    pub party: Option<Reference>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ClaimResponse {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub r#type: CodeableConcept,
    pub sub_type: Option<CodeableConcept>,
    pub r#use: Code,
    pub patient: Reference,
    pub created: DateTime,
    pub insurer: Reference,
    pub requestor: Option<Reference>,
    pub request: Option<Reference>,
    pub outcome: Code,
    pub disposition: Option<String>,
    pub pre_auth_ref: Option<String>,
    pub pre_auth_period: Option<Period>,
    pub payee_type: Option<CodeableConcept>,
    pub item: Option<Vec<ClaimResponseItem>>,
    pub add_item: Option<Vec<ClaimResponseAddItem>>,
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
    pub total: Option<Vec<ClaimResponseTotal>>,
    pub payment: Option<ClaimResponsePayment>,
    pub funds_reserve: Option<CodeableConcept>,
    pub form_code: Option<CodeableConcept>,
    pub form: Option<Attachment>,
    pub process_note: Option<Vec<ClaimResponseProcessNote>>,
    pub communication_request: Option<Vec<Reference>>,
    pub insurance: Option<Vec<ClaimResponseInsurance>>,
    pub error: Option<Vec<ClaimResponseError>>,
}

/// Choice of types for the serviced[x] field in ClaimResponseAddItem
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ClaimResponseAddItemServiced {
    /// Variant accepting the Date type.
    #[serde(rename = "servicedDate")]
    Date(Date),
    /// Variant accepting the Period type.
    #[serde(rename = "servicedPeriod")]
    Period(Period),
}

/// Choice of types for the location[x] field in ClaimResponseAddItem
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ClaimResponseAddItemLocation {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "locationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Address type.
    #[serde(rename = "locationAddress")]
    Address(Address),
    /// Variant accepting the Reference type.
    #[serde(rename = "locationReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ClaimResponseAddItem {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub item_sequence: Option<Vec<PositiveInt>>,
    pub detail_sequence: Option<Vec<PositiveInt>>,
    pub subdetail_sequence: Option<Vec<PositiveInt>>,
    pub provider: Option<Vec<Reference>>,
    pub product_or_service: CodeableConcept,
    pub modifier: Option<Vec<CodeableConcept>>,
    pub program_code: Option<Vec<CodeableConcept>>,
    pub serviced: Option<ClaimResponseAddItemServiced>,
    pub location: Option<ClaimResponseAddItemLocation>,
    pub quantity: Option<Quantity>,
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub net: Option<Money>,
    pub body_site: Option<CodeableConcept>,
    pub sub_site: Option<Vec<CodeableConcept>>,
    pub note_number: Option<Vec<PositiveInt>>,
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
    pub detail: Option<Vec<ClaimResponseAddItemDetail>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ClaimResponseInsurance {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub focal: Boolean,
    pub coverage: Reference,
    pub business_arrangement: Option<String>,
    pub claim_response: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ClaimResponseError {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub item_sequence: Option<PositiveInt>,
    pub detail_sequence: Option<PositiveInt>,
    pub sub_detail_sequence: Option<PositiveInt>,
    pub code: CodeableConcept,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ClaimResponseAddItemDetailSubDetail {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub product_or_service: CodeableConcept,
    pub modifier: Option<Vec<CodeableConcept>>,
    pub quantity: Option<Quantity>,
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub net: Option<Money>,
    pub note_number: Option<Vec<PositiveInt>>,
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ClaimResponseTotal {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: CodeableConcept,
    pub amount: Money,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ClaimResponseItemDetailSubDetail {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub sub_detail_sequence: PositiveInt,
    pub note_number: Option<Vec<PositiveInt>>,
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ClaimResponseItemDetail {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub detail_sequence: PositiveInt,
    pub note_number: Option<Vec<PositiveInt>>,
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
    pub sub_detail: Option<Vec<ClaimResponseItemDetailSubDetail>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ClaimResponseProcessNote {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub number: Option<PositiveInt>,
    pub r#type: Option<Code>,
    pub text: String,
    pub language: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ClaimResponseItem {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub item_sequence: PositiveInt,
    pub note_number: Option<Vec<PositiveInt>>,
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
    pub detail: Option<Vec<ClaimResponseItemDetail>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ClaimResponsePayment {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: CodeableConcept,
    pub adjustment: Option<Money>,
    pub adjustment_reason: Option<CodeableConcept>,
    pub date: Option<Date>,
    pub amount: Money,
    pub identifier: Option<Identifier>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ClaimResponseAddItemDetail {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub product_or_service: CodeableConcept,
    pub modifier: Option<Vec<CodeableConcept>>,
    pub quantity: Option<Quantity>,
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub net: Option<Money>,
    pub note_number: Option<Vec<PositiveInt>>,
    pub adjudication: Option<Vec<ClaimResponseItemAdjudication>>,
    pub sub_detail: Option<Vec<ClaimResponseAddItemDetailSubDetail>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ClaimResponseItemAdjudication {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: CodeableConcept,
    pub reason: Option<CodeableConcept>,
    pub amount: Option<Money>,
    pub value: Option<Decimal>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ClinicalImpressionFinding {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub item_codeable_concept: Option<CodeableConcept>,
    pub item_reference: Option<Reference>,
    pub basis: Option<String>,
}

/// Choice of types for the effective[x] field in ClinicalImpression
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ClinicalImpressionEffective {
    /// Variant accepting the DateTime type.
    #[serde(rename = "effectiveDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "effectivePeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ClinicalImpression {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub status_reason: Option<CodeableConcept>,
    pub code: Option<CodeableConcept>,
    pub description: Option<String>,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    pub effective: Option<ClinicalImpressionEffective>,
    pub date: Option<DateTime>,
    pub assessor: Option<Reference>,
    pub previous: Option<Reference>,
    pub problem: Option<Vec<Reference>>,
    pub investigation: Option<Vec<ClinicalImpressionInvestigation>>,
    pub protocol: Option<Vec<Uri>>,
    pub summary: Option<String>,
    pub finding: Option<Vec<ClinicalImpressionFinding>>,
    pub prognosis_codeable_concept: Option<Vec<CodeableConcept>>,
    pub prognosis_reference: Option<Vec<Reference>>,
    pub supporting_info: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ClinicalImpressionInvestigation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    pub item: Option<Vec<Reference>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CodeSystem {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
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
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    pub case_sensitive: Option<Boolean>,
    pub value_set: Option<Canonical>,
    pub hierarchy_meaning: Option<Code>,
    pub compositional: Option<Boolean>,
    pub version_needed: Option<Boolean>,
    pub content: Code,
    pub supplements: Option<Canonical>,
    pub count: Option<UnsignedInt>,
    pub filter: Option<Vec<CodeSystemFilter>>,
    pub property: Option<Vec<CodeSystemProperty>>,
    pub concept: Option<Vec<CodeSystemConcept>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CodeSystemConcept {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub display: Option<String>,
    pub definition: Option<String>,
    pub designation: Option<Vec<CodeSystemConceptDesignation>>,
    pub property: Option<Vec<CodeSystemConceptProperty>>,
    pub concept: Option<Vec<CodeSystemConcept>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CodeSystemFilter {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub description: Option<String>,
    pub operator: Option<Vec<Code>>,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CodeSystemConceptDesignation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub language: Option<Code>,
    pub r#use: Option<Coding>,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CodeSystemProperty {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub uri: Option<Uri>,
    pub description: Option<String>,
    pub r#type: Code,
}

/// Choice of types for the value[x] field in CodeSystemConceptProperty
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CodeSystemConceptPropertyValue {
    /// Variant accepting the Code type.
    #[serde(rename = "valueCode")]
    Code(Code),
    /// Variant accepting the Coding type.
    #[serde(rename = "valueCoding")]
    Coding(Coding),
    /// Variant accepting the String type.
    #[serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Integer type.
    #[serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Boolean type.
    #[serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the DateTime type.
    #[serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Decimal type.
    #[serde(rename = "valueDecimal")]
    Decimal(Decimal),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CodeSystemConceptProperty {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub value: Option<CodeSystemConceptPropertyValue>,
}


/// Choice of types for the content[x] field in CommunicationPayload
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CommunicationPayloadContent {
    /// Variant accepting the String type.
    #[serde(rename = "contentString")]
    String(String),
    /// Variant accepting the Attachment type.
    #[serde(rename = "contentAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[serde(rename = "contentReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CommunicationPayload {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub content: Option<CommunicationPayloadContent>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Communication {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub instantiates_canonical: Option<Vec<Canonical>>,
    pub instantiates_uri: Option<Vec<Uri>>,
    pub based_on: Option<Vec<Reference>>,
    pub part_of: Option<Vec<Reference>>,
    pub in_response_to: Option<Vec<Reference>>,
    pub status: Code,
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
    pub reason_code: Option<Vec<CodeableConcept>>,
    pub reason_reference: Option<Vec<Reference>>,
    pub payload: Option<Vec<CommunicationPayload>>,
    pub note: Option<Vec<Annotation>>,
}


/// Choice of types for the occurrence[x] field in CommunicationRequest
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CommunicationRequestOccurrence {
    /// Variant accepting the DateTime type.
    #[serde(rename = "occurrenceDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "occurrencePeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CommunicationRequest {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub based_on: Option<Vec<Reference>>,
    pub replaces: Option<Vec<Reference>>,
    pub group_identifier: Option<Identifier>,
    pub status: Code,
    pub status_reason: Option<CodeableConcept>,
    pub category: Option<Vec<CodeableConcept>>,
    pub priority: Option<Code>,
    pub do_not_perform: Option<Boolean>,
    pub medium: Option<Vec<CodeableConcept>>,
    pub subject: Option<Reference>,
    pub about: Option<Vec<Reference>>,
    pub encounter: Option<Reference>,
    pub payload: Option<Vec<CommunicationRequestPayload>>,
    pub occurrence: Option<CommunicationRequestOccurrence>,
    pub authored_on: Option<DateTime>,
    pub requester: Option<Reference>,
    pub recipient: Option<Vec<Reference>>,
    pub sender: Option<Reference>,
    pub reason_code: Option<Vec<CodeableConcept>>,
    pub reason_reference: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
}

/// Choice of types for the content[x] field in CommunicationRequestPayload
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CommunicationRequestPayloadContent {
    /// Variant accepting the String type.
    #[serde(rename = "contentString")]
    String(String),
    /// Variant accepting the Attachment type.
    #[serde(rename = "contentAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[serde(rename = "contentReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CommunicationRequestPayload {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub content: Option<CommunicationRequestPayloadContent>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CompartmentDefinition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
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
    pub use_context: Option<Vec<UsageContext>>,
    pub purpose: Option<Markdown>,
    pub code: Code,
    pub search: Boolean,
    pub resource: Option<Vec<CompartmentDefinitionResource>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CompartmentDefinitionResource {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub param: Option<Vec<String>>,
    pub documentation: Option<String>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Composition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    pub status: Code,
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
    pub relates_to: Option<Vec<CompositionRelatesTo>>,
    pub event: Option<Vec<CompositionEvent>>,
    pub section: Option<Vec<CompositionSection>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CompositionAttester {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub mode: Code,
    pub time: Option<DateTime>,
    pub party: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CompositionSection {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub title: Option<String>,
    pub code: Option<CodeableConcept>,
    pub author: Option<Vec<Reference>>,
    pub focus: Option<Reference>,
    pub text: Option<Narrative>,
    pub mode: Option<Code>,
    pub ordered_by: Option<CodeableConcept>,
    pub entry: Option<Vec<Reference>>,
    pub empty_reason: Option<CodeableConcept>,
    pub section: Option<Vec<CompositionSection>>,
}

/// Choice of types for the target[x] field in CompositionRelatesTo
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CompositionRelatesToTarget {
    /// Variant accepting the Identifier type.
    #[serde(rename = "targetIdentifier")]
    Identifier(Identifier),
    /// Variant accepting the Reference type.
    #[serde(rename = "targetReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CompositionRelatesTo {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub target: Option<CompositionRelatesToTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CompositionEvent {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<Vec<CodeableConcept>>,
    pub period: Option<Period>,
    pub detail: Option<Vec<Reference>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ConceptMapGroupElementTarget {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<Code>,
    pub display: Option<String>,
    pub equivalence: Code,
    pub comment: Option<String>,
    pub depends_on: Option<Vec<ConceptMapGroupElementTargetDependsOn>>,
    pub product: Option<Vec<ConceptMapGroupElementTargetDependsOn>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ConceptMapGroupElement {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<Code>,
    pub display: Option<String>,
    pub target: Option<Vec<ConceptMapGroupElementTarget>>,
}

/// Choice of types for the source[x] field in ConceptMap
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ConceptMapSource {
    /// Variant accepting the Uri type.
    #[serde(rename = "sourceUri")]
    Uri(Uri),
    /// Variant accepting the Canonical type.
    #[serde(rename = "sourceCanonical")]
    Canonical(Canonical),
}

/// Choice of types for the target[x] field in ConceptMap
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ConceptMapTarget {
    /// Variant accepting the Uri type.
    #[serde(rename = "targetUri")]
    Uri(Uri),
    /// Variant accepting the Canonical type.
    #[serde(rename = "targetCanonical")]
    Canonical(Canonical),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ConceptMap {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
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
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    pub source: Option<ConceptMapSource>,
    pub target: Option<ConceptMapTarget>,
    pub group: Option<Vec<ConceptMapGroup>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ConceptMapGroupElementTargetDependsOn {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub property: Uri,
    pub system: Option<Canonical>,
    pub value: String,
    pub display: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ConceptMapGroupUnmapped {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub mode: Code,
    pub code: Option<Code>,
    pub display: Option<String>,
    pub url: Option<Canonical>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ConceptMapGroup {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub source: Option<Uri>,
    pub source_version: Option<String>,
    pub target: Option<Uri>,
    pub target_version: Option<String>,
    pub element: Option<Vec<ConceptMapGroupElement>>,
    pub unmapped: Option<ConceptMapGroupUnmapped>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ConditionEvidence {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<Vec<CodeableConcept>>,
    pub detail: Option<Vec<Reference>>,
}

/// Choice of types for the onset[x] field in Condition
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ConditionOnset {
    /// Variant accepting the DateTime type.
    #[serde(rename = "onsetDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Age type.
    #[serde(rename = "onsetAge")]
    Age(Age),
    /// Variant accepting the Period type.
    #[serde(rename = "onsetPeriod")]
    Period(Period),
    /// Variant accepting the Range type.
    #[serde(rename = "onsetRange")]
    Range(Range),
    /// Variant accepting the String type.
    #[serde(rename = "onsetString")]
    String(String),
}

/// Choice of types for the abatement[x] field in Condition
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ConditionAbatement {
    /// Variant accepting the DateTime type.
    #[serde(rename = "abatementDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Age type.
    #[serde(rename = "abatementAge")]
    Age(Age),
    /// Variant accepting the Period type.
    #[serde(rename = "abatementPeriod")]
    Period(Period),
    /// Variant accepting the Range type.
    #[serde(rename = "abatementRange")]
    Range(Range),
    /// Variant accepting the String type.
    #[serde(rename = "abatementString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Condition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub clinical_status: Option<CodeableConcept>,
    pub verification_status: Option<CodeableConcept>,
    pub category: Option<Vec<CodeableConcept>>,
    pub severity: Option<CodeableConcept>,
    pub code: Option<CodeableConcept>,
    pub body_site: Option<Vec<CodeableConcept>>,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    pub onset: Option<ConditionOnset>,
    pub abatement: Option<ConditionAbatement>,
    pub recorded_date: Option<DateTime>,
    pub recorder: Option<Reference>,
    pub asserter: Option<Reference>,
    pub stage: Option<Vec<ConditionStage>>,
    pub evidence: Option<Vec<ConditionEvidence>>,
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ConditionStage {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub summary: Option<CodeableConcept>,
    pub assessment: Option<Vec<Reference>>,
    pub r#type: Option<CodeableConcept>,
}


/// Choice of types for the source[x] field in Consent
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ConsentSource {
    /// Variant accepting the Attachment type.
    #[serde(rename = "sourceAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[serde(rename = "sourceReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Consent {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub scope: CodeableConcept,
    pub category: Option<Vec<CodeableConcept>>,
    pub patient: Option<Reference>,
    pub date_time: Option<DateTime>,
    pub performer: Option<Vec<Reference>>,
    pub organization: Option<Vec<Reference>>,
    pub source: Option<ConsentSource>,
    pub policy: Option<Vec<ConsentPolicy>>,
    pub policy_rule: Option<CodeableConcept>,
    pub verification: Option<Vec<ConsentVerification>>,
    pub provision: Option<ConsentProvision>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ConsentProvision {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Option<Code>,
    pub period: Option<Period>,
    pub actor: Option<Vec<ConsentProvisionActor>>,
    pub action: Option<Vec<CodeableConcept>>,
    pub security_label: Option<Vec<Coding>>,
    pub purpose: Option<Vec<Coding>>,
    pub class: Option<Vec<Coding>>,
    pub code: Option<Vec<CodeableConcept>>,
    pub data_period: Option<Period>,
    pub data: Option<Vec<ConsentProvisionData>>,
    pub provision: Option<Vec<ConsentProvision>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ConsentVerification {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub verified: Boolean,
    pub verified_with: Option<Reference>,
    pub verification_date: Option<DateTime>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ConsentProvisionData {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub meaning: Code,
    pub reference: Reference,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ConsentPolicy {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub authority: Option<Uri>,
    pub uri: Option<Uri>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ConsentProvisionActor {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub role: CodeableConcept,
    pub reference: Reference,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ContractTermOffer {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub party: Option<Vec<ContractTermOfferParty>>,
    pub topic: Option<Reference>,
    pub r#type: Option<CodeableConcept>,
    pub decision: Option<CodeableConcept>,
    pub decision_mode: Option<Vec<CodeableConcept>>,
    pub answer: Option<Vec<ContractTermOfferAnswer>>,
    pub text: Option<String>,
    pub link_id: Option<Vec<String>>,
    pub security_label_number: Option<Vec<UnsignedInt>>,
}

/// Choice of types for the content[x] field in ContractFriendly
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ContractFriendlyContent {
    /// Variant accepting the Attachment type.
    #[serde(rename = "contentAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[serde(rename = "contentReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ContractFriendly {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub content: Option<ContractFriendlyContent>,
}

/// Choice of types for the topic[x] field in Contract
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ContractTopic {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "topicCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "topicReference")]
    Reference(Reference),
}

/// Choice of types for the legallyBinding[x] field in Contract
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ContractLegallyBinding {
    /// Variant accepting the Attachment type.
    #[serde(rename = "legallyBindingAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[serde(rename = "legallyBindingReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Contract {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub url: Option<Uri>,
    pub version: Option<String>,
    pub status: Option<Code>,
    pub legal_state: Option<CodeableConcept>,
    pub instantiates_canonical: Option<Reference>,
    pub instantiates_uri: Option<Uri>,
    pub content_derivative: Option<CodeableConcept>,
    pub issued: Option<DateTime>,
    pub applies: Option<Period>,
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
    pub topic: Option<ContractTopic>,
    pub r#type: Option<CodeableConcept>,
    pub sub_type: Option<Vec<CodeableConcept>>,
    pub content_definition: Option<ContractContentDefinition>,
    pub term: Option<Vec<ContractTerm>>,
    pub supporting_info: Option<Vec<Reference>>,
    pub relevant_history: Option<Vec<Reference>>,
    pub signer: Option<Vec<ContractSigner>>,
    pub friendly: Option<Vec<ContractFriendly>>,
    pub legal: Option<Vec<ContractLegal>>,
    pub rule: Option<Vec<ContractRule>>,
    pub legally_binding: Option<ContractLegallyBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ContractSigner {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Coding,
    pub party: Reference,
    pub signature: Option<Vec<Signature>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ContractTermOfferParty {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub reference: Option<Vec<Reference>>,
    pub role: CodeableConcept,
}

/// Choice of types for the entity[x] field in ContractTermAssetValuedItem
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ContractTermAssetValuedItemEntity {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "entityCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "entityReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ContractTermAssetValuedItem {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub entity: Option<ContractTermAssetValuedItemEntity>,
    pub identifier: Option<Identifier>,
    pub effective_time: Option<DateTime>,
    pub quantity: Option<Quantity>,
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub points: Option<Decimal>,
    pub net: Option<Money>,
    pub payment: Option<String>,
    pub payment_date: Option<DateTime>,
    pub responsible: Option<Reference>,
    pub recipient: Option<Reference>,
    pub link_id: Option<Vec<String>>,
    pub security_label_number: Option<Vec<UnsignedInt>>,
}

/// Choice of types for the topic[x] field in ContractTerm
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ContractTermTopic {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "topicCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "topicReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ContractTerm {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    pub issued: Option<DateTime>,
    pub applies: Option<Period>,
    pub topic: Option<ContractTermTopic>,
    pub r#type: Option<CodeableConcept>,
    pub sub_type: Option<CodeableConcept>,
    pub text: Option<String>,
    pub security_label: Option<Vec<ContractTermSecurityLabel>>,
    pub offer: ContractTermOffer,
    pub asset: Option<Vec<ContractTermAsset>>,
    pub action: Option<Vec<ContractTermAction>>,
    pub group: Option<Vec<ContractTerm>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ContractTermAsset {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub scope: Option<CodeableConcept>,
    pub r#type: Option<Vec<CodeableConcept>>,
    pub type_reference: Option<Vec<Reference>>,
    pub subtype: Option<Vec<CodeableConcept>>,
    pub relationship: Option<Coding>,
    pub context: Option<Vec<ContractTermAssetContext>>,
    pub condition: Option<String>,
    pub period_type: Option<Vec<CodeableConcept>>,
    pub period: Option<Vec<Period>>,
    pub use_period: Option<Vec<Period>>,
    pub text: Option<String>,
    pub link_id: Option<Vec<String>>,
    pub answer: Option<Vec<ContractTermOfferAnswer>>,
    pub security_label_number: Option<Vec<UnsignedInt>>,
    pub valued_item: Option<Vec<ContractTermAssetValuedItem>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ContractTermAssetContext {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub reference: Option<Reference>,
    pub code: Option<Vec<CodeableConcept>>,
    pub text: Option<String>,
}

/// Choice of types for the value[x] field in ContractTermOfferAnswer
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ContractTermOfferAnswerValue {
    /// Variant accepting the Boolean type.
    #[serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Decimal type.
    #[serde(rename = "valueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Integer type.
    #[serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Date type.
    #[serde(rename = "valueDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Time type.
    #[serde(rename = "valueTime")]
    Time(Time),
    /// Variant accepting the String type.
    #[serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Uri type.
    #[serde(rename = "valueUri")]
    Uri(Uri),
    /// Variant accepting the Attachment type.
    #[serde(rename = "valueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Coding type.
    #[serde(rename = "valueCoding")]
    Coding(Coding),
    /// Variant accepting the Quantity type.
    #[serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Reference type.
    #[serde(rename = "valueReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ContractTermOfferAnswer {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub value: Option<ContractTermOfferAnswerValue>,
}

/// Choice of types for the occurrence[x] field in ContractTermAction
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ContractTermActionOccurrence {
    /// Variant accepting the DateTime type.
    #[serde(rename = "occurrenceDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "occurrencePeriod")]
    Period(Period),
    /// Variant accepting the Timing type.
    #[serde(rename = "occurrenceTiming")]
    Timing(Timing),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ContractTermAction {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub do_not_perform: Option<Boolean>,
    pub r#type: CodeableConcept,
    pub subject: Option<Vec<ContractTermActionSubject>>,
    pub intent: CodeableConcept,
    pub link_id: Option<Vec<String>>,
    pub status: CodeableConcept,
    pub context: Option<Reference>,
    pub context_link_id: Option<Vec<String>>,
    pub occurrence: Option<ContractTermActionOccurrence>,
    pub requester: Option<Vec<Reference>>,
    pub requester_link_id: Option<Vec<String>>,
    pub performer_type: Option<Vec<CodeableConcept>>,
    pub performer_role: Option<CodeableConcept>,
    pub performer: Option<Reference>,
    pub performer_link_id: Option<Vec<String>>,
    pub reason_code: Option<Vec<CodeableConcept>>,
    pub reason_reference: Option<Vec<Reference>>,
    pub reason: Option<Vec<String>>,
    pub reason_link_id: Option<Vec<String>>,
    pub note: Option<Vec<Annotation>>,
    pub security_label_number: Option<Vec<UnsignedInt>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ContractTermActionSubject {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub reference: Option<Vec<Reference>>,
    pub role: Option<CodeableConcept>,
}

/// Choice of types for the content[x] field in ContractLegal
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ContractLegalContent {
    /// Variant accepting the Attachment type.
    #[serde(rename = "contentAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[serde(rename = "contentReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ContractLegal {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub content: Option<ContractLegalContent>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ContractTermSecurityLabel {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub number: Option<Vec<UnsignedInt>>,
    pub classification: Coding,
    pub category: Option<Vec<Coding>>,
    pub control: Option<Vec<Coding>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ContractContentDefinition {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: CodeableConcept,
    pub sub_type: Option<CodeableConcept>,
    pub publisher: Option<Reference>,
    pub publication_date: Option<DateTime>,
    pub publication_status: Code,
    pub copyright: Option<Markdown>,
}

/// Choice of types for the content[x] field in ContractRule
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ContractRuleContent {
    /// Variant accepting the Attachment type.
    #[serde(rename = "contentAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[serde(rename = "contentReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ContractRule {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub content: Option<ContractRuleContent>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Coverage {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub r#type: Option<CodeableConcept>,
    pub policy_holder: Option<Reference>,
    pub subscriber: Option<Reference>,
    pub subscriber_id: Option<String>,
    pub beneficiary: Reference,
    pub dependent: Option<String>,
    pub relationship: Option<CodeableConcept>,
    pub period: Option<Period>,
    pub payor: Option<Vec<Reference>>,
    pub class: Option<Vec<CoverageClass>>,
    pub order: Option<PositiveInt>,
    pub network: Option<String>,
    pub cost_to_beneficiary: Option<Vec<CoverageCostToBeneficiary>>,
    pub subrogation: Option<Boolean>,
    pub contract: Option<Vec<Reference>>,
}

/// Choice of types for the value[x] field in CoverageCostToBeneficiary
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CoverageCostToBeneficiaryValue {
    /// Variant accepting the Quantity type.
    #[serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Money type.
    #[serde(rename = "valueMoney")]
    Money(Money),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CoverageCostToBeneficiary {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Option<CodeableConcept>,
    pub value: Option<CoverageCostToBeneficiaryValue>,
    pub exception: Option<Vec<CoverageCostToBeneficiaryException>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CoverageCostToBeneficiaryException {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: CodeableConcept,
    pub period: Option<Period>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CoverageClass {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: CodeableConcept,
    pub value: String,
    pub name: Option<String>,
}


/// Choice of types for the serviced[x] field in CoverageEligibilityRequest
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CoverageEligibilityRequestServiced {
    /// Variant accepting the Date type.
    #[serde(rename = "servicedDate")]
    Date(Date),
    /// Variant accepting the Period type.
    #[serde(rename = "servicedPeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CoverageEligibilityRequest {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub priority: Option<CodeableConcept>,
    pub purpose: Option<Vec<Code>>,
    pub patient: Reference,
    pub serviced: Option<CoverageEligibilityRequestServiced>,
    pub created: DateTime,
    pub enterer: Option<Reference>,
    pub provider: Option<Reference>,
    pub insurer: Reference,
    pub facility: Option<Reference>,
    pub supporting_info: Option<Vec<CoverageEligibilityRequestSupportingInfo>>,
    pub insurance: Option<Vec<CoverageEligibilityRequestInsurance>>,
    pub item: Option<Vec<CoverageEligibilityRequestItem>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CoverageEligibilityRequestItem {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub supporting_info_sequence: Option<Vec<PositiveInt>>,
    pub category: Option<CodeableConcept>,
    pub product_or_service: Option<CodeableConcept>,
    pub modifier: Option<Vec<CodeableConcept>>,
    pub provider: Option<Reference>,
    pub quantity: Option<Quantity>,
    pub unit_price: Option<Money>,
    pub facility: Option<Reference>,
    pub diagnosis: Option<Vec<CoverageEligibilityRequestItemDiagnosis>>,
    pub detail: Option<Vec<Reference>>,
}

/// Choice of types for the diagnosis[x] field in CoverageEligibilityRequestItemDiagnosis
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CoverageEligibilityRequestItemDiagnosisDiagnosis {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "diagnosisCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "diagnosisReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CoverageEligibilityRequestItemDiagnosis {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub diagnosis: Option<CoverageEligibilityRequestItemDiagnosisDiagnosis>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CoverageEligibilityRequestInsurance {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub focal: Option<Boolean>,
    pub coverage: Reference,
    pub business_arrangement: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CoverageEligibilityRequestSupportingInfo {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub information: Reference,
    pub applies_to_all: Option<Boolean>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CoverageEligibilityResponseInsuranceItem {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: Option<CodeableConcept>,
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
    pub authorization_required: Option<Boolean>,
    pub authorization_supporting: Option<Vec<CodeableConcept>>,
    pub authorization_url: Option<Uri>,
}

/// Choice of types for the allowed[x] field in CoverageEligibilityResponseInsuranceItemBenefit
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CoverageEligibilityResponseInsuranceItemBenefitAllowed {
    /// Variant accepting the UnsignedInt type.
    #[serde(rename = "allowedUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the String type.
    #[serde(rename = "allowedString")]
    String(String),
    /// Variant accepting the Money type.
    #[serde(rename = "allowedMoney")]
    Money(Money),
}

/// Choice of types for the used[x] field in CoverageEligibilityResponseInsuranceItemBenefit
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CoverageEligibilityResponseInsuranceItemBenefitUsed {
    /// Variant accepting the UnsignedInt type.
    #[serde(rename = "usedUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the String type.
    #[serde(rename = "usedString")]
    String(String),
    /// Variant accepting the Money type.
    #[serde(rename = "usedMoney")]
    Money(Money),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CoverageEligibilityResponseInsuranceItemBenefit {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: CodeableConcept,
    pub allowed: Option<CoverageEligibilityResponseInsuranceItemBenefitAllowed>,
    pub used: Option<CoverageEligibilityResponseInsuranceItemBenefitUsed>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CoverageEligibilityResponseError {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CoverageEligibilityResponseInsurance {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub coverage: Reference,
    pub inforce: Option<Boolean>,
    pub benefit_period: Option<Period>,
    pub item: Option<Vec<CoverageEligibilityResponseInsuranceItem>>,
}

/// Choice of types for the serviced[x] field in CoverageEligibilityResponse
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CoverageEligibilityResponseServiced {
    /// Variant accepting the Date type.
    #[serde(rename = "servicedDate")]
    Date(Date),
    /// Variant accepting the Period type.
    #[serde(rename = "servicedPeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct CoverageEligibilityResponse {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub purpose: Option<Vec<Code>>,
    pub patient: Reference,
    pub serviced: Option<CoverageEligibilityResponseServiced>,
    pub created: DateTime,
    pub requestor: Option<Reference>,
    pub request: Reference,
    pub outcome: Code,
    pub disposition: Option<String>,
    pub insurer: Reference,
    pub insurance: Option<Vec<CoverageEligibilityResponseInsurance>>,
    pub pre_auth_ref: Option<String>,
    pub form: Option<CodeableConcept>,
    pub error: Option<Vec<CoverageEligibilityResponseError>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct DetectedIssueMitigation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub action: CodeableConcept,
    pub date: Option<DateTime>,
    pub author: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct DetectedIssueEvidence {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<Vec<CodeableConcept>>,
    pub detail: Option<Vec<Reference>>,
}

/// Choice of types for the identified[x] field in DetectedIssue
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DetectedIssueIdentified {
    /// Variant accepting the DateTime type.
    #[serde(rename = "identifiedDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "identifiedPeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct DetectedIssue {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub code: Option<CodeableConcept>,
    pub severity: Option<Code>,
    pub patient: Option<Reference>,
    pub identified: Option<DetectedIssueIdentified>,
    pub author: Option<Reference>,
    pub implicated: Option<Vec<Reference>>,
    pub evidence: Option<Vec<DetectedIssueEvidence>>,
    pub detail: Option<String>,
    pub reference: Option<Uri>,
    pub mitigation: Option<Vec<DetectedIssueMitigation>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct DeviceProperty {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: CodeableConcept,
    pub value_quantity: Option<Vec<Quantity>>,
    pub value_code: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct DeviceVersion {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Option<CodeableConcept>,
    pub component: Option<Identifier>,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct DeviceUdiCarrier {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub device_identifier: Option<String>,
    pub issuer: Option<Uri>,
    pub jurisdiction: Option<Uri>,
    pub carrier_a_i_d_c: Option<Base64Binary>,
    pub carrier_h_r_f: Option<String>,
    pub entry_type: Option<Code>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Device {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub definition: Option<Reference>,
    pub udi_carrier: Option<Vec<DeviceUdiCarrier>>,
    pub status: Option<Code>,
    pub status_reason: Option<Vec<CodeableConcept>>,
    pub distinct_identifier: Option<String>,
    pub manufacturer: Option<String>,
    pub manufacture_date: Option<DateTime>,
    pub expiration_date: Option<DateTime>,
    pub lot_number: Option<String>,
    pub serial_number: Option<String>,
    pub device_name: Option<Vec<DeviceDeviceName>>,
    pub model_number: Option<String>,
    pub part_number: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct DeviceDeviceName {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub r#type: Code,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct DeviceSpecialization {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub system_type: CodeableConcept,
    pub version: Option<String>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct DeviceDefinitionMaterial {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub substance: CodeableConcept,
    pub alternate: Option<Boolean>,
    pub allergenic_indicator: Option<Boolean>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct DeviceDefinitionCapability {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: CodeableConcept,
    pub description: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct DeviceDefinitionUdiDeviceIdentifier {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub device_identifier: String,
    pub issuer: Uri,
    pub jurisdiction: Uri,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct DeviceDefinitionSpecialization {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub system_type: String,
    pub version: Option<String>,
}

/// Choice of types for the manufacturer[x] field in DeviceDefinition
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DeviceDefinitionManufacturer {
    /// Variant accepting the String type.
    #[serde(rename = "manufacturerString")]
    String(String),
    /// Variant accepting the Reference type.
    #[serde(rename = "manufacturerReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct DeviceDefinition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub udi_device_identifier: Option<Vec<DeviceDefinitionUdiDeviceIdentifier>>,
    pub manufacturer: Option<DeviceDefinitionManufacturer>,
    pub device_name: Option<Vec<DeviceDefinitionDeviceName>>,
    pub model_number: Option<String>,
    pub r#type: Option<CodeableConcept>,
    pub specialization: Option<Vec<DeviceDefinitionSpecialization>>,
    pub version: Option<Vec<String>>,
    pub safety: Option<Vec<CodeableConcept>>,
    pub shelf_life_storage: Option<Vec<ProductShelfLife>>,
    pub physical_characteristics: Option<ProdCharacteristic>,
    pub language_code: Option<Vec<CodeableConcept>>,
    pub capability: Option<Vec<DeviceDefinitionCapability>>,
    pub property: Option<Vec<DeviceDefinitionProperty>>,
    pub owner: Option<Reference>,
    pub contact: Option<Vec<ContactPoint>>,
    pub url: Option<Uri>,
    pub online_information: Option<Uri>,
    pub note: Option<Vec<Annotation>>,
    pub quantity: Option<Quantity>,
    pub parent_device: Option<Reference>,
    pub material: Option<Vec<DeviceDefinitionMaterial>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct DeviceDefinitionProperty {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: CodeableConcept,
    pub value_quantity: Option<Vec<Quantity>>,
    pub value_code: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct DeviceDefinitionDeviceName {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub r#type: Code,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct DeviceMetric {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub r#type: CodeableConcept,
    pub unit: Option<CodeableConcept>,
    pub source: Option<Reference>,
    pub parent: Option<Reference>,
    pub operational_status: Option<Code>,
    pub color: Option<Code>,
    pub category: Code,
    pub measurement_period: Option<Timing>,
    pub calibration: Option<Vec<DeviceMetricCalibration>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct DeviceMetricCalibration {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Option<Code>,
    pub state: Option<Code>,
    pub time: Option<Instant>,
}


/// Choice of types for the value[x] field in DeviceRequestParameter
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DeviceRequestParameterValue {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Quantity type.
    #[serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[serde(rename = "valueRange")]
    Range(Range),
    /// Variant accepting the Boolean type.
    #[serde(rename = "valueBoolean")]
    Boolean(Boolean),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct DeviceRequestParameter {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub value: Option<DeviceRequestParameterValue>,
}

/// Choice of types for the code[x] field in DeviceRequest
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DeviceRequestCode {
    /// Variant accepting the Reference type.
    #[serde(rename = "codeReference")]
    Reference(Reference),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "codeCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

/// Choice of types for the occurrence[x] field in DeviceRequest
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DeviceRequestOccurrence {
    /// Variant accepting the DateTime type.
    #[serde(rename = "occurrenceDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "occurrencePeriod")]
    Period(Period),
    /// Variant accepting the Timing type.
    #[serde(rename = "occurrenceTiming")]
    Timing(Timing),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct DeviceRequest {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub instantiates_canonical: Option<Vec<Canonical>>,
    pub instantiates_uri: Option<Vec<Uri>>,
    pub based_on: Option<Vec<Reference>>,
    pub prior_request: Option<Vec<Reference>>,
    pub group_identifier: Option<Identifier>,
    pub status: Option<Code>,
    pub intent: Code,
    pub priority: Option<Code>,
    pub code: Option<DeviceRequestCode>,
    pub parameter: Option<Vec<DeviceRequestParameter>>,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    pub occurrence: Option<DeviceRequestOccurrence>,
    pub authored_on: Option<DateTime>,
    pub requester: Option<Reference>,
    pub performer_type: Option<CodeableConcept>,
    pub performer: Option<Reference>,
    pub reason_code: Option<Vec<CodeableConcept>>,
    pub reason_reference: Option<Vec<Reference>>,
    pub insurance: Option<Vec<Reference>>,
    pub supporting_info: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    pub relevant_history: Option<Vec<Reference>>,
}


/// Choice of types for the timing[x] field in DeviceUseStatement
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DeviceUseStatementTiming {
    /// Variant accepting the Timing type.
    #[serde(rename = "timingTiming")]
    Timing(Timing),
    /// Variant accepting the Period type.
    #[serde(rename = "timingPeriod")]
    Period(Period),
    /// Variant accepting the DateTime type.
    #[serde(rename = "timingDateTime")]
    DateTime(DateTime),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct DeviceUseStatement {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub based_on: Option<Vec<Reference>>,
    pub status: Code,
    pub subject: Reference,
    pub derived_from: Option<Vec<Reference>>,
    pub timing: Option<DeviceUseStatementTiming>,
    pub recorded_on: Option<DateTime>,
    pub source: Option<Reference>,
    pub device: Reference,
    pub reason_code: Option<Vec<CodeableConcept>>,
    pub reason_reference: Option<Vec<Reference>>,
    pub body_site: Option<CodeableConcept>,
    pub note: Option<Vec<Annotation>>,
}


/// Choice of types for the effective[x] field in DiagnosticReport
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticReportEffective {
    /// Variant accepting the DateTime type.
    #[serde(rename = "effectiveDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "effectivePeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct DiagnosticReport {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub based_on: Option<Vec<Reference>>,
    pub status: Code,
    pub category: Option<Vec<CodeableConcept>>,
    pub code: CodeableConcept,
    pub subject: Option<Reference>,
    pub encounter: Option<Reference>,
    pub effective: Option<DiagnosticReportEffective>,
    pub issued: Option<Instant>,
    pub performer: Option<Vec<Reference>>,
    pub results_interpreter: Option<Vec<Reference>>,
    pub specimen: Option<Vec<Reference>>,
    pub result: Option<Vec<Reference>>,
    pub imaging_study: Option<Vec<Reference>>,
    pub media: Option<Vec<DiagnosticReportMedia>>,
    pub conclusion: Option<String>,
    pub conclusion_code: Option<Vec<CodeableConcept>>,
    pub presented_form: Option<Vec<Attachment>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct DiagnosticReportMedia {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub comment: Option<String>,
    pub link: Reference,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct DocumentManifest {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub master_identifier: Option<Identifier>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
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

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct DocumentManifestRelated {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    pub r#ref: Option<Reference>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct DocumentReferenceRelatesTo {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub target: Reference,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct DocumentReferenceContent {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub attachment: Attachment,
    pub format: Option<Coding>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct DocumentReferenceContext {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub encounter: Option<Vec<Reference>>,
    pub event: Option<Vec<CodeableConcept>>,
    pub period: Option<Period>,
    pub facility_type: Option<CodeableConcept>,
    pub practice_setting: Option<CodeableConcept>,
    pub source_patient_info: Option<Reference>,
    pub related: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct DocumentReference {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub master_identifier: Option<Identifier>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub doc_status: Option<Code>,
    pub r#type: Option<CodeableConcept>,
    pub category: Option<Vec<CodeableConcept>>,
    pub subject: Option<Reference>,
    pub date: Option<Instant>,
    pub author: Option<Vec<Reference>>,
    pub authenticator: Option<Reference>,
    pub custodian: Option<Reference>,
    pub relates_to: Option<Vec<DocumentReferenceRelatesTo>>,
    pub description: Option<String>,
    pub security_label: Option<Vec<CodeableConcept>>,
    pub content: Option<Vec<DocumentReferenceContent>>,
    pub context: Option<DocumentReferenceContext>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct EffectEvidenceSynthesisSampleSize {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    pub number_of_studies: Option<Integer>,
    pub number_of_participants: Option<Integer>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct EffectEvidenceSynthesisEffectEstimate {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    pub r#type: Option<CodeableConcept>,
    pub variant_state: Option<CodeableConcept>,
    pub value: Option<Decimal>,
    pub unit_of_measure: Option<CodeableConcept>,
    pub precision_estimate: Option<Vec<EffectEvidenceSynthesisEffectEstimatePrecisionEstimate>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct EffectEvidenceSynthesisCertaintyCertaintySubcomponent {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Option<CodeableConcept>,
    pub rating: Option<Vec<CodeableConcept>>,
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct EffectEvidenceSynthesisResultsByExposure {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    pub exposure_state: Option<Code>,
    pub variant_state: Option<CodeableConcept>,
    pub risk_evidence_synthesis: Reference,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct EffectEvidenceSynthesisEffectEstimatePrecisionEstimate {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Option<CodeableConcept>,
    pub level: Option<Decimal>,
    pub from: Option<Decimal>,
    pub to: Option<Decimal>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct EffectEvidenceSynthesisCertainty {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub rating: Option<Vec<CodeableConcept>>,
    pub note: Option<Vec<Annotation>>,
    pub certainty_subcomponent: Option<Vec<EffectEvidenceSynthesisCertaintyCertaintySubcomponent>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct EffectEvidenceSynthesis {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub status: Code,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    pub note: Option<Vec<Annotation>>,
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub copyright: Option<Markdown>,
    pub approval_date: Option<Date>,
    pub last_review_date: Option<Date>,
    pub effective_period: Option<Period>,
    pub topic: Option<Vec<CodeableConcept>>,
    pub author: Option<Vec<ContactDetail>>,
    pub editor: Option<Vec<ContactDetail>>,
    pub reviewer: Option<Vec<ContactDetail>>,
    pub endorser: Option<Vec<ContactDetail>>,
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    pub synthesis_type: Option<CodeableConcept>,
    pub study_type: Option<CodeableConcept>,
    pub population: Reference,
    pub exposure: Reference,
    pub exposure_alternative: Reference,
    pub outcome: Reference,
    pub sample_size: Option<EffectEvidenceSynthesisSampleSize>,
    pub results_by_exposure: Option<Vec<EffectEvidenceSynthesisResultsByExposure>>,
    pub effect_estimate: Option<Vec<EffectEvidenceSynthesisEffectEstimate>>,
    pub certainty: Option<Vec<EffectEvidenceSynthesisCertainty>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct EncounterDiagnosis {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub condition: Reference,
    pub r#use: Option<CodeableConcept>,
    pub rank: Option<PositiveInt>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct EncounterHospitalization {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub pre_admission_identifier: Option<Identifier>,
    pub origin: Option<Reference>,
    pub admit_source: Option<CodeableConcept>,
    pub re_admission: Option<CodeableConcept>,
    pub diet_preference: Option<Vec<CodeableConcept>>,
    pub special_courtesy: Option<Vec<CodeableConcept>>,
    pub special_arrangement: Option<Vec<CodeableConcept>>,
    pub destination: Option<Reference>,
    pub discharge_disposition: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct EncounterLocation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub location: Reference,
    pub status: Option<Code>,
    pub physical_type: Option<CodeableConcept>,
    pub period: Option<Period>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Encounter {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub status_history: Option<Vec<EncounterStatusHistory>>,
    pub class: Coding,
    pub class_history: Option<Vec<EncounterClassHistory>>,
    pub r#type: Option<Vec<CodeableConcept>>,
    pub service_type: Option<CodeableConcept>,
    pub priority: Option<CodeableConcept>,
    pub subject: Option<Reference>,
    pub episode_of_care: Option<Vec<Reference>>,
    pub based_on: Option<Vec<Reference>>,
    pub participant: Option<Vec<EncounterParticipant>>,
    pub appointment: Option<Vec<Reference>>,
    pub period: Option<Period>,
    pub length: Option<Duration>,
    pub reason_code: Option<Vec<CodeableConcept>>,
    pub reason_reference: Option<Vec<Reference>>,
    pub diagnosis: Option<Vec<EncounterDiagnosis>>,
    pub account: Option<Vec<Reference>>,
    pub hospitalization: Option<EncounterHospitalization>,
    pub location: Option<Vec<EncounterLocation>>,
    pub service_provider: Option<Reference>,
    pub part_of: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct EncounterStatusHistory {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub status: Code,
    pub period: Period,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct EncounterParticipant {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Option<Vec<CodeableConcept>>,
    pub period: Option<Period>,
    pub individual: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct EncounterClassHistory {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub class: Coding,
    pub period: Period,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Endpoint {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub connection_type: Coding,
    pub name: Option<String>,
    pub managing_organization: Option<Reference>,
    pub contact: Option<Vec<ContactPoint>>,
    pub period: Option<Period>,
    pub payload_type: Option<Vec<CodeableConcept>>,
    pub payload_mime_type: Option<Vec<Code>>,
    pub address: Url,
    pub header: Option<Vec<String>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct EnrollmentRequest {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Option<Code>,
    pub created: Option<DateTime>,
    pub insurer: Option<Reference>,
    pub provider: Option<Reference>,
    pub candidate: Option<Reference>,
    pub coverage: Option<Reference>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct EnrollmentResponse {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Option<Code>,
    pub request: Option<Reference>,
    pub outcome: Option<Code>,
    pub disposition: Option<String>,
    pub created: Option<DateTime>,
    pub organization: Option<Reference>,
    pub request_provider: Option<Reference>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct EpisodeOfCareDiagnosis {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub condition: Reference,
    pub role: Option<CodeableConcept>,
    pub rank: Option<PositiveInt>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct EpisodeOfCareStatusHistory {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub status: Code,
    pub period: Period,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct EpisodeOfCare {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub status_history: Option<Vec<EpisodeOfCareStatusHistory>>,
    pub r#type: Option<Vec<CodeableConcept>>,
    pub diagnosis: Option<Vec<EpisodeOfCareDiagnosis>>,
    pub patient: Reference,
    pub managing_organization: Option<Reference>,
    pub period: Option<Period>,
    pub referral_request: Option<Vec<Reference>>,
    pub care_manager: Option<Reference>,
    pub team: Option<Vec<Reference>>,
    pub account: Option<Vec<Reference>>,
}


/// Choice of types for the subject[x] field in EventDefinition
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum EventDefinitionSubject {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "subjectReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct EventDefinition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub subject: Option<EventDefinitionSubject>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub usage: Option<String>,
    pub copyright: Option<Markdown>,
    pub approval_date: Option<Date>,
    pub last_review_date: Option<Date>,
    pub effective_period: Option<Period>,
    pub topic: Option<Vec<CodeableConcept>>,
    pub author: Option<Vec<ContactDetail>>,
    pub editor: Option<Vec<ContactDetail>>,
    pub reviewer: Option<Vec<ContactDetail>>,
    pub endorser: Option<Vec<ContactDetail>>,
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    pub trigger: Option<Vec<TriggerDefinition>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Evidence {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub short_title: Option<String>,
    pub subtitle: Option<String>,
    pub status: Code,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    pub note: Option<Vec<Annotation>>,
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub copyright: Option<Markdown>,
    pub approval_date: Option<Date>,
    pub last_review_date: Option<Date>,
    pub effective_period: Option<Period>,
    pub topic: Option<Vec<CodeableConcept>>,
    pub author: Option<Vec<ContactDetail>>,
    pub editor: Option<Vec<ContactDetail>>,
    pub reviewer: Option<Vec<ContactDetail>>,
    pub endorser: Option<Vec<ContactDetail>>,
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    pub exposure_background: Reference,
    pub exposure_variant: Option<Vec<Reference>>,
    pub outcome: Option<Vec<Reference>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct EvidenceVariable {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub short_title: Option<String>,
    pub subtitle: Option<String>,
    pub status: Code,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    pub note: Option<Vec<Annotation>>,
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub copyright: Option<Markdown>,
    pub approval_date: Option<Date>,
    pub last_review_date: Option<Date>,
    pub effective_period: Option<Period>,
    pub topic: Option<Vec<CodeableConcept>>,
    pub author: Option<Vec<ContactDetail>>,
    pub editor: Option<Vec<ContactDetail>>,
    pub reviewer: Option<Vec<ContactDetail>>,
    pub endorser: Option<Vec<ContactDetail>>,
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    pub r#type: Option<Code>,
    pub characteristic: Option<Vec<EvidenceVariableCharacteristic>>,
}

/// Choice of types for the definition[x] field in EvidenceVariableCharacteristic
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum EvidenceVariableCharacteristicDefinition {
    /// Variant accepting the Reference type.
    #[serde(rename = "definitionReference")]
    Reference(Reference),
    /// Variant accepting the Canonical type.
    #[serde(rename = "definitionCanonical")]
    Canonical(Canonical),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "definitionCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Expression type.
    #[serde(rename = "definitionExpression")]
    Expression(Expression),
    /// Variant accepting the DataRequirement type.
    #[serde(rename = "definitionDataRequirement")]
    DataRequirement(DataRequirement),
    /// Variant accepting the TriggerDefinition type.
    #[serde(rename = "definitionTriggerDefinition")]
    TriggerDefinition(TriggerDefinition),
}

/// Choice of types for the participantEffective[x] field in EvidenceVariableCharacteristic
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum EvidenceVariableCharacteristicParticipantEffective {
    /// Variant accepting the DateTime type.
    #[serde(rename = "participantEffectiveDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "participantEffectivePeriod")]
    Period(Period),
    /// Variant accepting the Duration type.
    #[serde(rename = "participantEffectiveDuration")]
    Duration(Duration),
    /// Variant accepting the Timing type.
    #[serde(rename = "participantEffectiveTiming")]
    Timing(Timing),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct EvidenceVariableCharacteristic {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    pub definition: Option<EvidenceVariableCharacteristicDefinition>,
    pub usage_context: Option<Vec<UsageContext>>,
    pub exclude: Option<Boolean>,
    pub participant_effective: Option<EvidenceVariableCharacteristicParticipantEffective>,
    pub time_from_start: Option<Duration>,
    pub group_measure: Option<Code>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ExampleScenarioInstanceVersion {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub version_id: String,
    pub description: Markdown,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ExampleScenarioProcessStep {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub process: Option<Vec<ExampleScenarioProcess>>,
    pub pause: Option<Boolean>,
    pub operation: Option<ExampleScenarioProcessStepOperation>,
    pub alternative: Option<Vec<ExampleScenarioProcessStepAlternative>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ExampleScenarioActor {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub actor_id: String,
    pub r#type: Code,
    pub name: Option<String>,
    pub description: Option<Markdown>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ExampleScenarioInstance {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub resource_id: String,
    pub resource_type: Code,
    pub name: Option<String>,
    pub description: Option<Markdown>,
    pub version: Option<Vec<ExampleScenarioInstanceVersion>>,
    pub contained_instance: Option<Vec<ExampleScenarioInstanceContainedInstance>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ExampleScenarioInstanceContainedInstance {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub resource_id: String,
    pub version_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ExampleScenario {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
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
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub copyright: Option<Markdown>,
    pub purpose: Option<Markdown>,
    pub actor: Option<Vec<ExampleScenarioActor>>,
    pub instance: Option<Vec<ExampleScenarioInstance>>,
    pub process: Option<Vec<ExampleScenarioProcess>>,
    pub workflow: Option<Vec<Canonical>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ExampleScenarioProcess {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub title: String,
    pub description: Option<Markdown>,
    pub pre_conditions: Option<Markdown>,
    pub post_conditions: Option<Markdown>,
    pub step: Option<Vec<ExampleScenarioProcessStep>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ExampleScenarioProcessStepAlternative {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub title: String,
    pub description: Option<Markdown>,
    pub step: Option<Vec<ExampleScenarioProcessStep>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ExampleScenarioProcessStepOperation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub number: String,
    pub r#type: Option<String>,
    pub name: Option<String>,
    pub initiator: Option<String>,
    pub receiver: Option<String>,
    pub description: Option<Markdown>,
    pub initiator_active: Option<Boolean>,
    pub receiver_active: Option<Boolean>,
    pub request: Option<ExampleScenarioInstanceContainedInstance>,
    pub response: Option<ExampleScenarioInstanceContainedInstance>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ExplanationOfBenefit {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub r#type: CodeableConcept,
    pub sub_type: Option<CodeableConcept>,
    pub r#use: Code,
    pub patient: Reference,
    pub billable_period: Option<Period>,
    pub created: DateTime,
    pub enterer: Option<Reference>,
    pub insurer: Reference,
    pub provider: Reference,
    pub priority: Option<CodeableConcept>,
    pub funds_reserve_requested: Option<CodeableConcept>,
    pub funds_reserve: Option<CodeableConcept>,
    pub related: Option<Vec<ExplanationOfBenefitRelated>>,
    pub prescription: Option<Reference>,
    pub original_prescription: Option<Reference>,
    pub payee: Option<ExplanationOfBenefitPayee>,
    pub referral: Option<Reference>,
    pub facility: Option<Reference>,
    pub claim: Option<Reference>,
    pub claim_response: Option<Reference>,
    pub outcome: Code,
    pub disposition: Option<String>,
    pub pre_auth_ref: Option<Vec<String>>,
    pub pre_auth_ref_period: Option<Vec<Period>>,
    pub care_team: Option<Vec<ExplanationOfBenefitCareTeam>>,
    pub supporting_info: Option<Vec<ExplanationOfBenefitSupportingInfo>>,
    pub diagnosis: Option<Vec<ExplanationOfBenefitDiagnosis>>,
    pub procedure: Option<Vec<ExplanationOfBenefitProcedure>>,
    pub precedence: Option<PositiveInt>,
    pub insurance: Option<Vec<ExplanationOfBenefitInsurance>>,
    pub accident: Option<ExplanationOfBenefitAccident>,
    pub item: Option<Vec<ExplanationOfBenefitItem>>,
    pub add_item: Option<Vec<ExplanationOfBenefitAddItem>>,
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
    pub total: Option<Vec<ExplanationOfBenefitTotal>>,
    pub payment: Option<ExplanationOfBenefitPayment>,
    pub form_code: Option<CodeableConcept>,
    pub form: Option<Attachment>,
    pub process_note: Option<Vec<ExplanationOfBenefitProcessNote>>,
    pub benefit_period: Option<Period>,
    pub benefit_balance: Option<Vec<ExplanationOfBenefitBenefitBalance>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ExplanationOfBenefitAddItemDetailSubDetail {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub product_or_service: CodeableConcept,
    pub modifier: Option<Vec<CodeableConcept>>,
    pub quantity: Option<Quantity>,
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub net: Option<Money>,
    pub note_number: Option<Vec<PositiveInt>>,
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ExplanationOfBenefitBenefitBalance {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
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

/// Choice of types for the allowed[x] field in ExplanationOfBenefitBenefitBalanceFinancial
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitBenefitBalanceFinancialAllowed {
    /// Variant accepting the UnsignedInt type.
    #[serde(rename = "allowedUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the String type.
    #[serde(rename = "allowedString")]
    String(String),
    /// Variant accepting the Money type.
    #[serde(rename = "allowedMoney")]
    Money(Money),
}

/// Choice of types for the used[x] field in ExplanationOfBenefitBenefitBalanceFinancial
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitBenefitBalanceFinancialUsed {
    /// Variant accepting the UnsignedInt type.
    #[serde(rename = "usedUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Money type.
    #[serde(rename = "usedMoney")]
    Money(Money),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ExplanationOfBenefitBenefitBalanceFinancial {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: CodeableConcept,
    pub allowed: Option<ExplanationOfBenefitBenefitBalanceFinancialAllowed>,
    pub used: Option<ExplanationOfBenefitBenefitBalanceFinancialUsed>,
}

/// Choice of types for the timing[x] field in ExplanationOfBenefitSupportingInfo
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitSupportingInfoTiming {
    /// Variant accepting the Date type.
    #[serde(rename = "timingDate")]
    Date(Date),
    /// Variant accepting the Period type.
    #[serde(rename = "timingPeriod")]
    Period(Period),
}

/// Choice of types for the value[x] field in ExplanationOfBenefitSupportingInfo
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitSupportingInfoValue {
    /// Variant accepting the Boolean type.
    #[serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the String type.
    #[serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Quantity type.
    #[serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Attachment type.
    #[serde(rename = "valueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Reference type.
    #[serde(rename = "valueReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ExplanationOfBenefitSupportingInfo {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub category: CodeableConcept,
    pub code: Option<CodeableConcept>,
    pub timing: Option<ExplanationOfBenefitSupportingInfoTiming>,
    pub value: Option<ExplanationOfBenefitSupportingInfoValue>,
    pub reason: Option<Coding>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ExplanationOfBenefitInsurance {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub focal: Boolean,
    pub coverage: Reference,
    pub pre_auth_ref: Option<Vec<String>>,
}

/// Choice of types for the location[x] field in ExplanationOfBenefitAccident
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitAccidentLocation {
    /// Variant accepting the Address type.
    #[serde(rename = "locationAddress")]
    Address(Address),
    /// Variant accepting the Reference type.
    #[serde(rename = "locationReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ExplanationOfBenefitAccident {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub date: Option<Date>,
    pub r#type: Option<CodeableConcept>,
    pub location: Option<ExplanationOfBenefitAccidentLocation>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ExplanationOfBenefitItemAdjudication {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: CodeableConcept,
    pub reason: Option<CodeableConcept>,
    pub amount: Option<Money>,
    pub value: Option<Decimal>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ExplanationOfBenefitTotal {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: CodeableConcept,
    pub amount: Money,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ExplanationOfBenefitItemDetail {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub revenue: Option<CodeableConcept>,
    pub category: Option<CodeableConcept>,
    pub product_or_service: CodeableConcept,
    pub modifier: Option<Vec<CodeableConcept>>,
    pub program_code: Option<Vec<CodeableConcept>>,
    pub quantity: Option<Quantity>,
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub net: Option<Money>,
    pub udi: Option<Vec<Reference>>,
    pub note_number: Option<Vec<PositiveInt>>,
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
    pub sub_detail: Option<Vec<ExplanationOfBenefitItemDetailSubDetail>>,
}

/// Choice of types for the diagnosis[x] field in ExplanationOfBenefitDiagnosis
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitDiagnosisDiagnosis {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "diagnosisCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "diagnosisReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ExplanationOfBenefitDiagnosis {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub diagnosis: Option<ExplanationOfBenefitDiagnosisDiagnosis>,
    pub r#type: Option<Vec<CodeableConcept>>,
    pub on_admission: Option<CodeableConcept>,
    pub package_code: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ExplanationOfBenefitPayee {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Option<CodeableConcept>,
    pub party: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ExplanationOfBenefitAddItemDetail {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub product_or_service: CodeableConcept,
    pub modifier: Option<Vec<CodeableConcept>>,
    pub quantity: Option<Quantity>,
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub net: Option<Money>,
    pub note_number: Option<Vec<PositiveInt>>,
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
    pub sub_detail: Option<Vec<ExplanationOfBenefitAddItemDetailSubDetail>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ExplanationOfBenefitProcessNote {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub number: Option<PositiveInt>,
    pub r#type: Option<Code>,
    pub text: Option<String>,
    pub language: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ExplanationOfBenefitRelated {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub claim: Option<Reference>,
    pub relationship: Option<CodeableConcept>,
    pub reference: Option<Identifier>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ExplanationOfBenefitCareTeam {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub provider: Reference,
    pub responsible: Option<Boolean>,
    pub role: Option<CodeableConcept>,
    pub qualification: Option<CodeableConcept>,
}

/// Choice of types for the procedure[x] field in ExplanationOfBenefitProcedure
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitProcedureProcedure {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "procedureCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "procedureReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ExplanationOfBenefitProcedure {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub r#type: Option<Vec<CodeableConcept>>,
    pub date: Option<DateTime>,
    pub procedure: Option<ExplanationOfBenefitProcedureProcedure>,
    pub udi: Option<Vec<Reference>>,
}

/// Choice of types for the serviced[x] field in ExplanationOfBenefitItem
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitItemServiced {
    /// Variant accepting the Date type.
    #[serde(rename = "servicedDate")]
    Date(Date),
    /// Variant accepting the Period type.
    #[serde(rename = "servicedPeriod")]
    Period(Period),
}

/// Choice of types for the location[x] field in ExplanationOfBenefitItem
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitItemLocation {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "locationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Address type.
    #[serde(rename = "locationAddress")]
    Address(Address),
    /// Variant accepting the Reference type.
    #[serde(rename = "locationReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ExplanationOfBenefitItem {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub care_team_sequence: Option<Vec<PositiveInt>>,
    pub diagnosis_sequence: Option<Vec<PositiveInt>>,
    pub procedure_sequence: Option<Vec<PositiveInt>>,
    pub information_sequence: Option<Vec<PositiveInt>>,
    pub revenue: Option<CodeableConcept>,
    pub category: Option<CodeableConcept>,
    pub product_or_service: CodeableConcept,
    pub modifier: Option<Vec<CodeableConcept>>,
    pub program_code: Option<Vec<CodeableConcept>>,
    pub serviced: Option<ExplanationOfBenefitItemServiced>,
    pub location: Option<ExplanationOfBenefitItemLocation>,
    pub quantity: Option<Quantity>,
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub net: Option<Money>,
    pub udi: Option<Vec<Reference>>,
    pub body_site: Option<CodeableConcept>,
    pub sub_site: Option<Vec<CodeableConcept>>,
    pub encounter: Option<Vec<Reference>>,
    pub note_number: Option<Vec<PositiveInt>>,
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
    pub detail: Option<Vec<ExplanationOfBenefitItemDetail>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ExplanationOfBenefitItemDetailSubDetail {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: PositiveInt,
    pub revenue: Option<CodeableConcept>,
    pub category: Option<CodeableConcept>,
    pub product_or_service: CodeableConcept,
    pub modifier: Option<Vec<CodeableConcept>>,
    pub program_code: Option<Vec<CodeableConcept>>,
    pub quantity: Option<Quantity>,
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub net: Option<Money>,
    pub udi: Option<Vec<Reference>>,
    pub note_number: Option<Vec<PositiveInt>>,
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
}

/// Choice of types for the serviced[x] field in ExplanationOfBenefitAddItem
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitAddItemServiced {
    /// Variant accepting the Date type.
    #[serde(rename = "servicedDate")]
    Date(Date),
    /// Variant accepting the Period type.
    #[serde(rename = "servicedPeriod")]
    Period(Period),
}

/// Choice of types for the location[x] field in ExplanationOfBenefitAddItem
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ExplanationOfBenefitAddItemLocation {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "locationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Address type.
    #[serde(rename = "locationAddress")]
    Address(Address),
    /// Variant accepting the Reference type.
    #[serde(rename = "locationReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ExplanationOfBenefitAddItem {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub item_sequence: Option<Vec<PositiveInt>>,
    pub detail_sequence: Option<Vec<PositiveInt>>,
    pub sub_detail_sequence: Option<Vec<PositiveInt>>,
    pub provider: Option<Vec<Reference>>,
    pub product_or_service: CodeableConcept,
    pub modifier: Option<Vec<CodeableConcept>>,
    pub program_code: Option<Vec<CodeableConcept>>,
    pub serviced: Option<ExplanationOfBenefitAddItemServiced>,
    pub location: Option<ExplanationOfBenefitAddItemLocation>,
    pub quantity: Option<Quantity>,
    pub unit_price: Option<Money>,
    pub factor: Option<Decimal>,
    pub net: Option<Money>,
    pub body_site: Option<CodeableConcept>,
    pub sub_site: Option<Vec<CodeableConcept>>,
    pub note_number: Option<Vec<PositiveInt>>,
    pub adjudication: Option<Vec<ExplanationOfBenefitItemAdjudication>>,
    pub detail: Option<Vec<ExplanationOfBenefitAddItemDetail>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ExplanationOfBenefitPayment {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Option<CodeableConcept>,
    pub adjustment: Option<Money>,
    pub adjustment_reason: Option<CodeableConcept>,
    pub date: Option<Date>,
    pub amount: Option<Money>,
    pub identifier: Option<Identifier>,
}


/// Choice of types for the born[x] field in FamilyMemberHistory
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FamilyMemberHistoryBorn {
    /// Variant accepting the Period type.
    #[serde(rename = "bornPeriod")]
    Period(Period),
    /// Variant accepting the Date type.
    #[serde(rename = "bornDate")]
    Date(Date),
    /// Variant accepting the String type.
    #[serde(rename = "bornString")]
    String(String),
}

/// Choice of types for the age[x] field in FamilyMemberHistory
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FamilyMemberHistoryAge {
    /// Variant accepting the Age type.
    #[serde(rename = "ageAge")]
    Age(Age),
    /// Variant accepting the Range type.
    #[serde(rename = "ageRange")]
    Range(Range),
    /// Variant accepting the String type.
    #[serde(rename = "ageString")]
    String(String),
}

/// Choice of types for the deceased[x] field in FamilyMemberHistory
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FamilyMemberHistoryDeceased {
    /// Variant accepting the Boolean type.
    #[serde(rename = "deceasedBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Age type.
    #[serde(rename = "deceasedAge")]
    Age(Age),
    /// Variant accepting the Range type.
    #[serde(rename = "deceasedRange")]
    Range(Range),
    /// Variant accepting the Date type.
    #[serde(rename = "deceasedDate")]
    Date(Date),
    /// Variant accepting the String type.
    #[serde(rename = "deceasedString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct FamilyMemberHistory {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub instantiates_canonical: Option<Vec<Canonical>>,
    pub instantiates_uri: Option<Vec<Uri>>,
    pub status: Code,
    pub data_absent_reason: Option<CodeableConcept>,
    pub patient: Reference,
    pub date: Option<DateTime>,
    pub name: Option<String>,
    pub relationship: CodeableConcept,
    pub sex: Option<CodeableConcept>,
    pub born: Option<FamilyMemberHistoryBorn>,
    pub age: Option<FamilyMemberHistoryAge>,
    pub estimated_age: Option<Boolean>,
    pub deceased: Option<FamilyMemberHistoryDeceased>,
    pub reason_code: Option<Vec<CodeableConcept>>,
    pub reason_reference: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    pub condition: Option<Vec<FamilyMemberHistoryCondition>>,
}

/// Choice of types for the onset[x] field in FamilyMemberHistoryCondition
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum FamilyMemberHistoryConditionOnset {
    /// Variant accepting the Age type.
    #[serde(rename = "onsetAge")]
    Age(Age),
    /// Variant accepting the Range type.
    #[serde(rename = "onsetRange")]
    Range(Range),
    /// Variant accepting the Period type.
    #[serde(rename = "onsetPeriod")]
    Period(Period),
    /// Variant accepting the String type.
    #[serde(rename = "onsetString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct FamilyMemberHistoryCondition {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    pub outcome: Option<CodeableConcept>,
    pub contributed_to_death: Option<Boolean>,
    pub onset: Option<FamilyMemberHistoryConditionOnset>,
    pub note: Option<Vec<Annotation>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Flag {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
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


/// Choice of types for the start[x] field in Goal
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum GoalStart {
    /// Variant accepting the Date type.
    #[serde(rename = "startDate")]
    Date(Date),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "startCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Goal {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub lifecycle_status: Code,
    pub achievement_status: Option<CodeableConcept>,
    pub category: Option<Vec<CodeableConcept>>,
    pub priority: Option<CodeableConcept>,
    pub description: CodeableConcept,
    pub subject: Reference,
    pub start: Option<GoalStart>,
    pub target: Option<Vec<GoalTarget>>,
    pub status_date: Option<Date>,
    pub status_reason: Option<String>,
    pub expressed_by: Option<Reference>,
    pub addresses: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    pub outcome_code: Option<Vec<CodeableConcept>>,
    pub outcome_reference: Option<Vec<Reference>>,
}

/// Choice of types for the detail[x] field in GoalTarget
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum GoalTargetDetail {
    /// Variant accepting the Quantity type.
    #[serde(rename = "detailQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[serde(rename = "detailRange")]
    Range(Range),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "detailCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the String type.
    #[serde(rename = "detailString")]
    String(String),
    /// Variant accepting the Boolean type.
    #[serde(rename = "detailBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Integer type.
    #[serde(rename = "detailInteger")]
    Integer(Integer),
    /// Variant accepting the Ratio type.
    #[serde(rename = "detailRatio")]
    Ratio(Ratio),
}

/// Choice of types for the due[x] field in GoalTarget
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum GoalTargetDue {
    /// Variant accepting the Date type.
    #[serde(rename = "dueDate")]
    Date(Date),
    /// Variant accepting the Duration type.
    #[serde(rename = "dueDuration")]
    Duration(Duration),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct GoalTarget {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub measure: Option<CodeableConcept>,
    pub detail: Option<GoalTargetDetail>,
    pub due: Option<GoalTargetDue>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct GraphDefinition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
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
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub start: Code,
    pub profile: Option<Canonical>,
    pub link: Option<Vec<GraphDefinitionLink>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct GraphDefinitionLinkTargetCompartment {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#use: Code,
    pub code: Code,
    pub rule: Code,
    pub expression: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct GraphDefinitionLinkTarget {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Code,
    pub params: Option<String>,
    pub profile: Option<Canonical>,
    pub compartment: Option<Vec<GraphDefinitionLinkTargetCompartment>>,
    pub link: Option<Vec<GraphDefinitionLink>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct GraphDefinitionLink {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub path: Option<String>,
    pub slice_name: Option<String>,
    pub min: Option<Integer>,
    pub max: Option<String>,
    pub description: Option<String>,
    pub target: Option<Vec<GraphDefinitionLinkTarget>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Group {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub active: Option<Boolean>,
    pub r#type: Code,
    pub actual: Boolean,
    pub code: Option<CodeableConcept>,
    pub name: Option<String>,
    pub quantity: Option<UnsignedInt>,
    pub managing_entity: Option<Reference>,
    pub characteristic: Option<Vec<GroupCharacteristic>>,
    pub member: Option<Vec<GroupMember>>,
}

/// Choice of types for the value[x] field in GroupCharacteristic
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum GroupCharacteristicValue {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Boolean type.
    #[serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Quantity type.
    #[serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[serde(rename = "valueRange")]
    Range(Range),
    /// Variant accepting the Reference type.
    #[serde(rename = "valueReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct GroupCharacteristic {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    pub value: Option<GroupCharacteristicValue>,
    pub exclude: Boolean,
    pub period: Option<Period>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct GroupMember {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub entity: Reference,
    pub period: Option<Period>,
    pub inactive: Option<Boolean>,
}


/// Choice of types for the module[x] field in GuidanceResponse
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum GuidanceResponseModule {
    /// Variant accepting the Uri type.
    #[serde(rename = "moduleUri")]
    Uri(Uri),
    /// Variant accepting the Canonical type.
    #[serde(rename = "moduleCanonical")]
    Canonical(Canonical),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "moduleCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct GuidanceResponse {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub request_identifier: Option<Identifier>,
    pub identifier: Option<Vec<Identifier>>,
    pub module: Option<GuidanceResponseModule>,
    pub status: Code,
    pub subject: Option<Reference>,
    pub encounter: Option<Reference>,
    pub occurrence_date_time: Option<DateTime>,
    pub performer: Option<Reference>,
    pub reason_code: Option<Vec<CodeableConcept>>,
    pub reason_reference: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    pub evaluation_message: Option<Vec<Reference>>,
    pub output_parameters: Option<Reference>,
    pub result: Option<Reference>,
    pub data_requirement: Option<Vec<DataRequirement>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct HealthcareServiceNotAvailable {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: String,
    pub during: Option<Period>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct HealthcareService {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub active: Option<Boolean>,
    pub provided_by: Option<Reference>,
    pub category: Option<Vec<CodeableConcept>>,
    pub r#type: Option<Vec<CodeableConcept>>,
    pub specialty: Option<Vec<CodeableConcept>>,
    pub location: Option<Vec<Reference>>,
    pub name: Option<String>,
    pub comment: Option<String>,
    pub extra_details: Option<Markdown>,
    pub photo: Option<Attachment>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub coverage_area: Option<Vec<Reference>>,
    pub service_provision_code: Option<Vec<CodeableConcept>>,
    pub eligibility: Option<Vec<HealthcareServiceEligibility>>,
    pub program: Option<Vec<CodeableConcept>>,
    pub characteristic: Option<Vec<CodeableConcept>>,
    pub communication: Option<Vec<CodeableConcept>>,
    pub referral_method: Option<Vec<CodeableConcept>>,
    pub appointment_required: Option<Boolean>,
    pub available_time: Option<Vec<HealthcareServiceAvailableTime>>,
    pub not_available: Option<Vec<HealthcareServiceNotAvailable>>,
    pub availability_exceptions: Option<String>,
    pub endpoint: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct HealthcareServiceEligibility {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub comment: Option<Markdown>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct HealthcareServiceAvailableTime {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub days_of_week: Option<Vec<Code>>,
    pub all_day: Option<Boolean>,
    pub available_start_time: Option<Time>,
    pub available_end_time: Option<Time>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ImagingStudySeriesInstance {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub uid: Id,
    pub sop_class: Coding,
    pub number: Option<UnsignedInt>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ImagingStudySeries {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub uid: Id,
    pub number: Option<UnsignedInt>,
    pub modality: Coding,
    pub description: Option<String>,
    pub number_of_instances: Option<UnsignedInt>,
    pub endpoint: Option<Vec<Reference>>,
    pub body_site: Option<Coding>,
    pub laterality: Option<Coding>,
    pub specimen: Option<Vec<Reference>>,
    pub started: Option<DateTime>,
    pub performer: Option<Vec<ImagingStudySeriesPerformer>>,
    pub instance: Option<Vec<ImagingStudySeriesInstance>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ImagingStudySeriesPerformer {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ImagingStudy {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub modality: Option<Vec<Coding>>,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    pub started: Option<DateTime>,
    pub based_on: Option<Vec<Reference>>,
    pub referrer: Option<Reference>,
    pub interpreter: Option<Vec<Reference>>,
    pub endpoint: Option<Vec<Reference>>,
    pub number_of_series: Option<UnsignedInt>,
    pub number_of_instances: Option<UnsignedInt>,
    pub procedure_reference: Option<Reference>,
    pub procedure_code: Option<Vec<CodeableConcept>>,
    pub location: Option<Reference>,
    pub reason_code: Option<Vec<CodeableConcept>>,
    pub reason_reference: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    pub description: Option<String>,
    pub series: Option<Vec<ImagingStudySeries>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ImmunizationReaction {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub date: Option<DateTime>,
    pub detail: Option<Reference>,
    pub reported: Option<Boolean>,
}

/// Choice of types for the doseNumber[x] field in ImmunizationProtocolApplied
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ImmunizationProtocolAppliedDoseNumber {
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "doseNumberPositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[serde(rename = "doseNumberString")]
    String(String),
}

/// Choice of types for the seriesDoses[x] field in ImmunizationProtocolApplied
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ImmunizationProtocolAppliedSeriesDoses {
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "seriesDosesPositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[serde(rename = "seriesDosesString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ImmunizationProtocolApplied {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub series: Option<String>,
    pub authority: Option<Reference>,
    pub target_disease: Option<Vec<CodeableConcept>>,
    pub dose_number: Option<ImmunizationProtocolAppliedDoseNumber>,
    pub series_doses: Option<ImmunizationProtocolAppliedSeriesDoses>,
}

/// Choice of types for the occurrence[x] field in Immunization
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ImmunizationOccurrence {
    /// Variant accepting the DateTime type.
    #[serde(rename = "occurrenceDateTime")]
    DateTime(DateTime),
    /// Variant accepting the String type.
    #[serde(rename = "occurrenceString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Immunization {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub status_reason: Option<CodeableConcept>,
    pub vaccine_code: CodeableConcept,
    pub patient: Reference,
    pub encounter: Option<Reference>,
    pub occurrence: Option<ImmunizationOccurrence>,
    pub recorded: Option<DateTime>,
    pub primary_source: Option<Boolean>,
    pub report_origin: Option<CodeableConcept>,
    pub location: Option<Reference>,
    pub manufacturer: Option<Reference>,
    pub lot_number: Option<String>,
    pub expiration_date: Option<Date>,
    pub site: Option<CodeableConcept>,
    pub route: Option<CodeableConcept>,
    pub dose_quantity: Option<Quantity>,
    pub performer: Option<Vec<ImmunizationPerformer>>,
    pub note: Option<Vec<Annotation>>,
    pub reason_code: Option<Vec<CodeableConcept>>,
    pub reason_reference: Option<Vec<Reference>>,
    pub is_subpotent: Option<Boolean>,
    pub subpotent_reason: Option<Vec<CodeableConcept>>,
    pub education: Option<Vec<ImmunizationEducation>>,
    pub program_eligibility: Option<Vec<CodeableConcept>>,
    pub funding_source: Option<CodeableConcept>,
    pub reaction: Option<Vec<ImmunizationReaction>>,
    pub protocol_applied: Option<Vec<ImmunizationProtocolApplied>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ImmunizationEducation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub document_type: Option<String>,
    pub reference: Option<Uri>,
    pub publication_date: Option<DateTime>,
    pub presentation_date: Option<DateTime>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ImmunizationPerformer {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
}


/// Choice of types for the doseNumber[x] field in ImmunizationEvaluation
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ImmunizationEvaluationDoseNumber {
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "doseNumberPositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[serde(rename = "doseNumberString")]
    String(String),
}

/// Choice of types for the seriesDoses[x] field in ImmunizationEvaluation
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ImmunizationEvaluationSeriesDoses {
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "seriesDosesPositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[serde(rename = "seriesDosesString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ImmunizationEvaluation {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub patient: Reference,
    pub date: Option<DateTime>,
    pub authority: Option<Reference>,
    pub target_disease: CodeableConcept,
    pub immunization_event: Reference,
    pub dose_status: CodeableConcept,
    pub dose_status_reason: Option<Vec<CodeableConcept>>,
    pub description: Option<String>,
    pub series: Option<String>,
    pub dose_number: Option<ImmunizationEvaluationDoseNumber>,
    pub series_doses: Option<ImmunizationEvaluationSeriesDoses>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ImmunizationRecommendationRecommendationDateCriterion {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    pub value: DateTime,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ImmunizationRecommendation {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub patient: Reference,
    pub date: DateTime,
    pub authority: Option<Reference>,
    pub recommendation: Option<Vec<ImmunizationRecommendationRecommendation>>,
}

/// Choice of types for the doseNumber[x] field in ImmunizationRecommendationRecommendation
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ImmunizationRecommendationRecommendationDoseNumber {
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "doseNumberPositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[serde(rename = "doseNumberString")]
    String(String),
}

/// Choice of types for the seriesDoses[x] field in ImmunizationRecommendationRecommendation
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ImmunizationRecommendationRecommendationSeriesDoses {
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "seriesDosesPositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[serde(rename = "seriesDosesString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ImmunizationRecommendationRecommendation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub vaccine_code: Option<Vec<CodeableConcept>>,
    pub target_disease: Option<CodeableConcept>,
    pub contraindicated_vaccine_code: Option<Vec<CodeableConcept>>,
    pub forecast_status: CodeableConcept,
    pub forecast_reason: Option<Vec<CodeableConcept>>,
    pub date_criterion: Option<Vec<ImmunizationRecommendationRecommendationDateCriterion>>,
    pub description: Option<String>,
    pub series: Option<String>,
    pub dose_number: Option<ImmunizationRecommendationRecommendationDoseNumber>,
    pub series_doses: Option<ImmunizationRecommendationRecommendationSeriesDoses>,
    pub supporting_immunization: Option<Vec<Reference>>,
    pub supporting_patient_information: Option<Vec<Reference>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ImplementationGuideDefinition {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub grouping: Option<Vec<ImplementationGuideDefinitionGrouping>>,
    pub resource: Option<Vec<ImplementationGuideDefinitionResource>>,
    pub page: Option<ImplementationGuideDefinitionPage>,
    pub parameter: Option<Vec<ImplementationGuideDefinitionParameter>>,
    pub template: Option<Vec<ImplementationGuideDefinitionTemplate>>,
}

/// Choice of types for the example[x] field in ImplementationGuideDefinitionResource
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ImplementationGuideDefinitionResourceExample {
    /// Variant accepting the Boolean type.
    #[serde(rename = "exampleBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Canonical type.
    #[serde(rename = "exampleCanonical")]
    Canonical(Canonical),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ImplementationGuideDefinitionResource {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub reference: Reference,
    pub fhir_version: Option<Vec<Code>>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub example: Option<ImplementationGuideDefinitionResourceExample>,
    pub grouping_id: Option<Id>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ImplementationGuideManifestPage {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub title: Option<String>,
    pub anchor: Option<Vec<String>>,
}

/// Choice of types for the name[x] field in ImplementationGuideDefinitionPage
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ImplementationGuideDefinitionPageName {
    /// Variant accepting the Url type.
    #[serde(rename = "nameUrl")]
    Url(Url),
    /// Variant accepting the Reference type.
    #[serde(rename = "nameReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ImplementationGuideDefinitionPage {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Option<ImplementationGuideDefinitionPageName>,
    pub title: String,
    pub generation: Code,
    pub page: Option<Vec<ImplementationGuideDefinitionPage>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ImplementationGuideDefinitionGrouping {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ImplementationGuideDependsOn {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub uri: Canonical,
    pub package_id: Option<Id>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ImplementationGuideDefinitionParameter {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ImplementationGuideManifest {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub rendering: Option<Url>,
    pub resource: Option<Vec<ImplementationGuideManifestResource>>,
    pub page: Option<Vec<ImplementationGuideManifestPage>>,
    pub image: Option<Vec<String>>,
    pub other: Option<Vec<String>>,
}

/// Choice of types for the example[x] field in ImplementationGuideManifestResource
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ImplementationGuideManifestResourceExample {
    /// Variant accepting the Boolean type.
    #[serde(rename = "exampleBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Canonical type.
    #[serde(rename = "exampleCanonical")]
    Canonical(Canonical),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ImplementationGuideManifestResource {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub reference: Reference,
    pub example: Option<ImplementationGuideManifestResourceExample>,
    pub relative_path: Option<Url>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ImplementationGuideDefinitionTemplate {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub source: String,
    pub scope: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ImplementationGuideGlobal {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Code,
    pub profile: Canonical,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ImplementationGuide {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
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
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub copyright: Option<Markdown>,
    pub package_id: Id,
    pub license: Option<Code>,
    pub fhir_version: Option<Vec<Code>>,
    pub depends_on: Option<Vec<ImplementationGuideDependsOn>>,
    pub global: Option<Vec<ImplementationGuideGlobal>>,
    pub definition: Option<ImplementationGuideDefinition>,
    pub manifest: Option<ImplementationGuideManifest>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct InsurancePlanPlanGeneralCost {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Option<CodeableConcept>,
    pub group_size: Option<PositiveInt>,
    pub cost: Option<Money>,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct InsurancePlan {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Option<Code>,
    pub r#type: Option<Vec<CodeableConcept>>,
    pub name: Option<String>,
    pub alias: Option<Vec<String>>,
    pub period: Option<Period>,
    pub owned_by: Option<Reference>,
    pub administered_by: Option<Reference>,
    pub coverage_area: Option<Vec<Reference>>,
    pub contact: Option<Vec<InsurancePlanContact>>,
    pub endpoint: Option<Vec<Reference>>,
    pub network: Option<Vec<Reference>>,
    pub coverage: Option<Vec<InsurancePlanCoverage>>,
    pub plan: Option<Vec<InsurancePlanPlan>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct InsurancePlanCoverage {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: CodeableConcept,
    pub network: Option<Vec<Reference>>,
    pub benefit: Option<Vec<InsurancePlanCoverageBenefit>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct InsurancePlanContact {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub purpose: Option<CodeableConcept>,
    pub name: Option<HumanName>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub address: Option<Address>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct InsurancePlanCoverageBenefit {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: CodeableConcept,
    pub requirement: Option<String>,
    pub limit: Option<Vec<InsurancePlanCoverageBenefitLimit>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct InsurancePlanPlanSpecificCostBenefit {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: CodeableConcept,
    pub cost: Option<Vec<InsurancePlanPlanSpecificCostBenefitCost>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct InsurancePlanPlanSpecificCostBenefitCost {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: CodeableConcept,
    pub applicability: Option<CodeableConcept>,
    pub qualifiers: Option<Vec<CodeableConcept>>,
    pub value: Option<Quantity>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct InsurancePlanPlanSpecificCost {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: CodeableConcept,
    pub benefit: Option<Vec<InsurancePlanPlanSpecificCostBenefit>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct InsurancePlanPlan {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub r#type: Option<CodeableConcept>,
    pub coverage_area: Option<Vec<Reference>>,
    pub network: Option<Vec<Reference>>,
    pub general_cost: Option<Vec<InsurancePlanPlanGeneralCost>>,
    pub specific_cost: Option<Vec<InsurancePlanPlanSpecificCost>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct InsurancePlanCoverageBenefitLimit {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub value: Option<Quantity>,
    pub code: Option<CodeableConcept>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct InvoiceParticipant {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub role: Option<CodeableConcept>,
    pub actor: Reference,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct InvoiceLineItemPriceComponent {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Code,
    pub code: Option<CodeableConcept>,
    pub factor: Option<Decimal>,
    pub amount: Option<Money>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Invoice {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub cancelled_reason: Option<String>,
    pub r#type: Option<CodeableConcept>,
    pub subject: Option<Reference>,
    pub recipient: Option<Reference>,
    pub date: Option<DateTime>,
    pub participant: Option<Vec<InvoiceParticipant>>,
    pub issuer: Option<Reference>,
    pub account: Option<Reference>,
    pub line_item: Option<Vec<InvoiceLineItem>>,
    pub total_price_component: Option<Vec<InvoiceLineItemPriceComponent>>,
    pub total_net: Option<Money>,
    pub total_gross: Option<Money>,
    pub payment_terms: Option<Markdown>,
    pub note: Option<Vec<Annotation>>,
}

/// Choice of types for the chargeItem[x] field in InvoiceLineItem
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum InvoiceLineItemChargeItem {
    /// Variant accepting the Reference type.
    #[serde(rename = "chargeItemReference")]
    Reference(Reference),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "chargeItemCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct InvoiceLineItem {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence: Option<PositiveInt>,
    pub charge_item: Option<InvoiceLineItemChargeItem>,
    pub price_component: Option<Vec<InvoiceLineItemPriceComponent>>,
}


/// Choice of types for the subject[x] field in Library
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum LibrarySubject {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "subjectReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Library {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub r#type: CodeableConcept,
    pub subject: Option<LibrarySubject>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub usage: Option<String>,
    pub copyright: Option<Markdown>,
    pub approval_date: Option<Date>,
    pub last_review_date: Option<Date>,
    pub effective_period: Option<Period>,
    pub topic: Option<Vec<CodeableConcept>>,
    pub author: Option<Vec<ContactDetail>>,
    pub editor: Option<Vec<ContactDetail>>,
    pub reviewer: Option<Vec<ContactDetail>>,
    pub endorser: Option<Vec<ContactDetail>>,
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    pub parameter: Option<Vec<ParameterDefinition>>,
    pub data_requirement: Option<Vec<DataRequirement>>,
    pub content: Option<Vec<Attachment>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Linkage {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub active: Option<Boolean>,
    pub author: Option<Reference>,
    pub item: Option<Vec<LinkageItem>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct LinkageItem {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Code,
    pub resource: Reference,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ListEntry {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub flag: Option<CodeableConcept>,
    pub deleted: Option<Boolean>,
    pub date: Option<DateTime>,
    pub item: Reference,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct List {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
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
    pub ordered_by: Option<CodeableConcept>,
    pub note: Option<Vec<Annotation>>,
    pub entry: Option<Vec<ListEntry>>,
    pub empty_reason: Option<CodeableConcept>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Location {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Option<Code>,
    pub operational_status: Option<Coding>,
    pub name: Option<String>,
    pub alias: Option<Vec<String>>,
    pub description: Option<String>,
    pub mode: Option<Code>,
    pub r#type: Option<Vec<CodeableConcept>>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub address: Option<Address>,
    pub physical_type: Option<CodeableConcept>,
    pub position: Option<LocationPosition>,
    pub managing_organization: Option<Reference>,
    pub part_of: Option<Reference>,
    pub hours_of_operation: Option<Vec<LocationHoursOfOperation>>,
    pub availability_exceptions: Option<String>,
    pub endpoint: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct LocationHoursOfOperation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub days_of_week: Option<Vec<Code>>,
    pub all_day: Option<Boolean>,
    pub opening_time: Option<Time>,
    pub closing_time: Option<Time>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct LocationPosition {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub longitude: Decimal,
    pub latitude: Decimal,
    pub altitude: Option<Decimal>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MeasureGroup {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub description: Option<String>,
    pub population: Option<Vec<MeasureGroupPopulation>>,
    pub stratifier: Option<Vec<MeasureGroupStratifier>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MeasureGroupStratifier {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub description: Option<String>,
    pub criteria: Option<Expression>,
    pub component: Option<Vec<MeasureGroupStratifierComponent>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MeasureSupplementalData {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub usage: Option<Vec<CodeableConcept>>,
    pub description: Option<String>,
    pub criteria: Expression,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MeasureGroupStratifierComponent {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub description: Option<String>,
    pub criteria: Expression,
}

/// Choice of types for the subject[x] field in Measure
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MeasureSubject {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "subjectReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Measure {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub subject: Option<MeasureSubject>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub usage: Option<String>,
    pub copyright: Option<Markdown>,
    pub approval_date: Option<Date>,
    pub last_review_date: Option<Date>,
    pub effective_period: Option<Period>,
    pub topic: Option<Vec<CodeableConcept>>,
    pub author: Option<Vec<ContactDetail>>,
    pub editor: Option<Vec<ContactDetail>>,
    pub reviewer: Option<Vec<ContactDetail>>,
    pub endorser: Option<Vec<ContactDetail>>,
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    pub library: Option<Vec<Canonical>>,
    pub disclaimer: Option<Markdown>,
    pub scoring: Option<CodeableConcept>,
    pub composite_scoring: Option<CodeableConcept>,
    pub r#type: Option<Vec<CodeableConcept>>,
    pub risk_adjustment: Option<String>,
    pub rate_aggregation: Option<String>,
    pub rationale: Option<Markdown>,
    pub clinical_recommendation_statement: Option<Markdown>,
    pub improvement_notation: Option<CodeableConcept>,
    pub definition: Option<Vec<Markdown>>,
    pub guidance: Option<Markdown>,
    pub group: Option<Vec<MeasureGroup>>,
    pub supplemental_data: Option<Vec<MeasureSupplementalData>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MeasureGroupPopulation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub description: Option<String>,
    pub criteria: Expression,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MeasureReportGroup {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub population: Option<Vec<MeasureReportGroupPopulation>>,
    pub measure_score: Option<Quantity>,
    pub stratifier: Option<Vec<MeasureReportGroupStratifier>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MeasureReportGroupPopulation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub count: Option<Integer>,
    pub subject_results: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MeasureReportGroupStratifierStratum {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub value: Option<CodeableConcept>,
    pub component: Option<Vec<MeasureReportGroupStratifierStratumComponent>>,
    pub population: Option<Vec<MeasureReportGroupStratifierStratumPopulation>>,
    pub measure_score: Option<Quantity>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MeasureReportGroupStratifierStratumComponent {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    pub value: CodeableConcept,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MeasureReportGroupStratifierStratumPopulation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub count: Option<Integer>,
    pub subject_results: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MeasureReport {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub r#type: Code,
    pub measure: Canonical,
    pub subject: Option<Reference>,
    pub date: Option<DateTime>,
    pub reporter: Option<Reference>,
    pub period: Period,
    pub improvement_notation: Option<CodeableConcept>,
    pub group: Option<Vec<MeasureReportGroup>>,
    pub evaluated_resource: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MeasureReportGroupStratifier {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<Vec<CodeableConcept>>,
    pub stratum: Option<Vec<MeasureReportGroupStratifierStratum>>,
}


/// Choice of types for the created[x] field in Media
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MediaCreated {
    /// Variant accepting the DateTime type.
    #[serde(rename = "createdDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "createdPeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Media {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub based_on: Option<Vec<Reference>>,
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    pub r#type: Option<CodeableConcept>,
    pub modality: Option<CodeableConcept>,
    pub view: Option<CodeableConcept>,
    pub subject: Option<Reference>,
    pub encounter: Option<Reference>,
    pub created: Option<MediaCreated>,
    pub issued: Option<Instant>,
    pub operator: Option<Reference>,
    pub reason_code: Option<Vec<CodeableConcept>>,
    pub body_site: Option<CodeableConcept>,
    pub device_name: Option<String>,
    pub device: Option<Reference>,
    pub height: Option<PositiveInt>,
    pub width: Option<PositiveInt>,
    pub frames: Option<PositiveInt>,
    pub duration: Option<Decimal>,
    pub content: Attachment,
    pub note: Option<Vec<Annotation>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicationBatch {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub lot_number: Option<String>,
    pub expiration_date: Option<DateTime>,
}

/// Choice of types for the item[x] field in MedicationIngredient
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MedicationIngredientItem {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "itemCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "itemReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicationIngredient {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub item: Option<MedicationIngredientItem>,
    pub is_active: Option<Boolean>,
    pub strength: Option<Ratio>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Medication {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
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


/// Choice of types for the rate[x] field in MedicationAdministrationDosage
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MedicationAdministrationDosageRate {
    /// Variant accepting the Ratio type.
    #[serde(rename = "rateRatio")]
    Ratio(Ratio),
    /// Variant accepting the Quantity type.
    #[serde(rename = "rateQuantity")]
    Quantity(Quantity),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicationAdministrationDosage {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub text: Option<String>,
    pub site: Option<CodeableConcept>,
    pub route: Option<CodeableConcept>,
    pub method: Option<CodeableConcept>,
    pub dose: Option<Quantity>,
    pub rate: Option<MedicationAdministrationDosageRate>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicationAdministrationPerformer {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
}

/// Choice of types for the medication[x] field in MedicationAdministration
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MedicationAdministrationMedication {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "medicationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "medicationReference")]
    Reference(Reference),
}

/// Choice of types for the effective[x] field in MedicationAdministration
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MedicationAdministrationEffective {
    /// Variant accepting the DateTime type.
    #[serde(rename = "effectiveDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "effectivePeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicationAdministration {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub instantiates: Option<Vec<Uri>>,
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    pub status_reason: Option<Vec<CodeableConcept>>,
    pub category: Option<CodeableConcept>,
    pub medication: Option<MedicationAdministrationMedication>,
    pub subject: Reference,
    pub context: Option<Reference>,
    pub supporting_information: Option<Vec<Reference>>,
    pub effective: Option<MedicationAdministrationEffective>,
    pub performer: Option<Vec<MedicationAdministrationPerformer>>,
    pub reason_code: Option<Vec<CodeableConcept>>,
    pub reason_reference: Option<Vec<Reference>>,
    pub request: Option<Reference>,
    pub device: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    pub dosage: Option<MedicationAdministrationDosage>,
    pub event_history: Option<Vec<Reference>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicationDispenseSubstitution {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub was_substituted: Boolean,
    pub r#type: Option<CodeableConcept>,
    pub reason: Option<Vec<CodeableConcept>>,
    pub responsible_party: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicationDispensePerformer {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
}

/// Choice of types for the statusReason[x] field in MedicationDispense
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MedicationDispenseStatusReason {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "statusReasonCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "statusReasonReference")]
    Reference(Reference),
}

/// Choice of types for the medication[x] field in MedicationDispense
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MedicationDispenseMedication {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "medicationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "medicationReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicationDispense {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    pub status_reason: Option<MedicationDispenseStatusReason>,
    pub category: Option<CodeableConcept>,
    pub medication: Option<MedicationDispenseMedication>,
    pub subject: Option<Reference>,
    pub context: Option<Reference>,
    pub supporting_information: Option<Vec<Reference>>,
    pub performer: Option<Vec<MedicationDispensePerformer>>,
    pub location: Option<Reference>,
    pub authorizing_prescription: Option<Vec<Reference>>,
    pub r#type: Option<CodeableConcept>,
    pub quantity: Option<Quantity>,
    pub days_supply: Option<Quantity>,
    pub when_prepared: Option<DateTime>,
    pub when_handed_over: Option<DateTime>,
    pub destination: Option<Reference>,
    pub receiver: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    pub dosage_instruction: Option<Vec<Dosage>>,
    pub substitution: Option<MedicationDispenseSubstitution>,
    pub detected_issue: Option<Vec<Reference>>,
    pub event_history: Option<Vec<Reference>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicationKnowledgeMonograph {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Option<CodeableConcept>,
    pub source: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicationKnowledge {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub status: Option<Code>,
    pub manufacturer: Option<Reference>,
    pub dose_form: Option<CodeableConcept>,
    pub amount: Option<Quantity>,
    pub synonym: Option<Vec<String>>,
    pub related_medication_knowledge: Option<Vec<MedicationKnowledgeRelatedMedicationKnowledge>>,
    pub associated_medication: Option<Vec<Reference>>,
    pub product_type: Option<Vec<CodeableConcept>>,
    pub monograph: Option<Vec<MedicationKnowledgeMonograph>>,
    pub ingredient: Option<Vec<MedicationKnowledgeIngredient>>,
    pub preparation_instruction: Option<Markdown>,
    pub intended_route: Option<Vec<CodeableConcept>>,
    pub cost: Option<Vec<MedicationKnowledgeCost>>,
    pub monitoring_program: Option<Vec<MedicationKnowledgeMonitoringProgram>>,
    pub administration_guidelines: Option<Vec<MedicationKnowledgeAdministrationGuidelines>>,
    pub medicine_classification: Option<Vec<MedicationKnowledgeMedicineClassification>>,
    pub packaging: Option<MedicationKnowledgePackaging>,
    pub drug_characteristic: Option<Vec<MedicationKnowledgeDrugCharacteristic>>,
    pub contraindication: Option<Vec<Reference>>,
    pub regulatory: Option<Vec<MedicationKnowledgeRegulatory>>,
    pub kinetics: Option<Vec<MedicationKnowledgeKinetics>>,
}

/// Choice of types for the item[x] field in MedicationKnowledgeIngredient
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MedicationKnowledgeIngredientItem {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "itemCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "itemReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicationKnowledgeIngredient {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub item: Option<MedicationKnowledgeIngredientItem>,
    pub is_active: Option<Boolean>,
    pub strength: Option<Ratio>,
}

/// Choice of types for the characteristic[x] field in MedicationKnowledgeAdministrationGuidelinesPatientCharacteristics
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MedicationKnowledgeAdministrationGuidelinesPatientCharacteristicsCharacteristic {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "characteristicCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Quantity type.
    #[serde(rename = "characteristicQuantity")]
    Quantity(Quantity),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicationKnowledgeAdministrationGuidelinesPatientCharacteristics {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub characteristic: Option<MedicationKnowledgeAdministrationGuidelinesPatientCharacteristicsCharacteristic>,
    pub value: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicationKnowledgeAdministrationGuidelinesDosage {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: CodeableConcept,
    pub dosage: Option<Vec<Dosage>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicationKnowledgeMonitoringProgram {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Option<CodeableConcept>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicationKnowledgeRegulatorySchedule {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub schedule: CodeableConcept,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicationKnowledgeRegulatoryMaxDispense {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub quantity: Quantity,
    pub period: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicationKnowledgeKinetics {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub area_under_curve: Option<Vec<Quantity>>,
    pub lethal_dose50: Option<Vec<Quantity>>,
    pub half_life_period: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicationKnowledgeRegulatorySubstitution {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: CodeableConcept,
    pub allowed: Boolean,
}

/// Choice of types for the value[x] field in MedicationKnowledgeDrugCharacteristic
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MedicationKnowledgeDrugCharacteristicValue {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the String type.
    #[serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Quantity type.
    #[serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Base64Binary type.
    #[serde(rename = "valueBase64Binary")]
    Base64Binary(Base64Binary),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicationKnowledgeDrugCharacteristic {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Option<CodeableConcept>,
    pub value: Option<MedicationKnowledgeDrugCharacteristicValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicationKnowledgeRelatedMedicationKnowledge {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: CodeableConcept,
    pub reference: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicationKnowledgeMedicineClassification {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: CodeableConcept,
    pub classification: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicationKnowledgeCost {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: CodeableConcept,
    pub source: Option<String>,
    pub cost: Money,
}

/// Choice of types for the indication[x] field in MedicationKnowledgeAdministrationGuidelines
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MedicationKnowledgeAdministrationGuidelinesIndication {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "indicationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "indicationReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicationKnowledgeAdministrationGuidelines {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub dosage: Option<Vec<MedicationKnowledgeAdministrationGuidelinesDosage>>,
    pub indication: Option<MedicationKnowledgeAdministrationGuidelinesIndication>,
    pub patient_characteristics: Option<Vec<MedicationKnowledgeAdministrationGuidelinesPatientCharacteristics>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicationKnowledgePackaging {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Option<CodeableConcept>,
    pub quantity: Option<Quantity>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicationKnowledgeRegulatory {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub regulatory_authority: Reference,
    pub substitution: Option<Vec<MedicationKnowledgeRegulatorySubstitution>>,
    pub schedule: Option<Vec<MedicationKnowledgeRegulatorySchedule>>,
    pub max_dispense: Option<MedicationKnowledgeRegulatoryMaxDispense>,
}


/// Choice of types for the reported[x] field in MedicationRequest
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MedicationRequestReported {
    /// Variant accepting the Boolean type.
    #[serde(rename = "reportedBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Reference type.
    #[serde(rename = "reportedReference")]
    Reference(Reference),
}

/// Choice of types for the medication[x] field in MedicationRequest
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MedicationRequestMedication {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "medicationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "medicationReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicationRequest {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub status_reason: Option<CodeableConcept>,
    pub intent: Code,
    pub category: Option<Vec<CodeableConcept>>,
    pub priority: Option<Code>,
    pub do_not_perform: Option<Boolean>,
    pub reported: Option<MedicationRequestReported>,
    pub medication: Option<MedicationRequestMedication>,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    pub supporting_information: Option<Vec<Reference>>,
    pub authored_on: Option<DateTime>,
    pub requester: Option<Reference>,
    pub performer: Option<Reference>,
    pub performer_type: Option<CodeableConcept>,
    pub recorder: Option<Reference>,
    pub reason_code: Option<Vec<CodeableConcept>>,
    pub reason_reference: Option<Vec<Reference>>,
    pub instantiates_canonical: Option<Vec<Canonical>>,
    pub instantiates_uri: Option<Vec<Uri>>,
    pub based_on: Option<Vec<Reference>>,
    pub group_identifier: Option<Identifier>,
    pub course_of_therapy_type: Option<CodeableConcept>,
    pub insurance: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    pub dosage_instruction: Option<Vec<Dosage>>,
    pub dispense_request: Option<MedicationRequestDispenseRequest>,
    pub substitution: Option<MedicationRequestSubstitution>,
    pub prior_prescription: Option<Reference>,
    pub detected_issue: Option<Vec<Reference>>,
    pub event_history: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicationRequestDispenseRequest {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub initial_fill: Option<MedicationRequestDispenseRequestInitialFill>,
    pub dispense_interval: Option<Duration>,
    pub validity_period: Option<Period>,
    pub number_of_repeats_allowed: Option<UnsignedInt>,
    pub quantity: Option<Quantity>,
    pub expected_supply_duration: Option<Duration>,
    pub performer: Option<Reference>,
}

/// Choice of types for the allowed[x] field in MedicationRequestSubstitution
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MedicationRequestSubstitutionAllowed {
    /// Variant accepting the Boolean type.
    #[serde(rename = "allowedBoolean")]
    Boolean(Boolean),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "allowedCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicationRequestSubstitution {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub allowed: Option<MedicationRequestSubstitutionAllowed>,
    pub reason: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicationRequestDispenseRequestInitialFill {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub quantity: Option<Quantity>,
    pub duration: Option<Duration>,
}


/// Choice of types for the medication[x] field in MedicationStatement
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MedicationStatementMedication {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "medicationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "medicationReference")]
    Reference(Reference),
}

/// Choice of types for the effective[x] field in MedicationStatement
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MedicationStatementEffective {
    /// Variant accepting the DateTime type.
    #[serde(rename = "effectiveDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "effectivePeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicationStatement {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub based_on: Option<Vec<Reference>>,
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    pub status_reason: Option<Vec<CodeableConcept>>,
    pub category: Option<CodeableConcept>,
    pub medication: Option<MedicationStatementMedication>,
    pub subject: Reference,
    pub context: Option<Reference>,
    pub effective: Option<MedicationStatementEffective>,
    pub date_asserted: Option<DateTime>,
    pub information_source: Option<Reference>,
    pub derived_from: Option<Vec<Reference>>,
    pub reason_code: Option<Vec<CodeableConcept>>,
    pub reason_reference: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    pub dosage: Option<Vec<Dosage>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicinalProductNameCountryLanguage {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub country: CodeableConcept,
    pub jurisdiction: Option<CodeableConcept>,
    pub language: CodeableConcept,
}

/// Choice of types for the indication[x] field in MedicinalProductSpecialDesignation
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MedicinalProductSpecialDesignationIndication {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "indicationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "indicationReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicinalProductSpecialDesignation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub r#type: Option<CodeableConcept>,
    pub intended_use: Option<CodeableConcept>,
    pub indication: Option<MedicinalProductSpecialDesignationIndication>,
    pub status: Option<CodeableConcept>,
    pub date: Option<DateTime>,
    pub species: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicinalProductNameNamePart {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub part: String,
    pub r#type: Coding,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicinalProductManufacturingBusinessOperation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub operation_type: Option<CodeableConcept>,
    pub authorisation_reference_number: Option<Identifier>,
    pub effective_date: Option<DateTime>,
    pub confidentiality_indicator: Option<CodeableConcept>,
    pub manufacturer: Option<Vec<Reference>>,
    pub regulator: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicinalProduct {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub r#type: Option<CodeableConcept>,
    pub domain: Option<Coding>,
    pub combined_pharmaceutical_dose_form: Option<CodeableConcept>,
    pub legal_status_of_supply: Option<CodeableConcept>,
    pub additional_monitoring_indicator: Option<CodeableConcept>,
    pub special_measures: Option<Vec<String>>,
    pub paediatric_use_indicator: Option<CodeableConcept>,
    pub product_classification: Option<Vec<CodeableConcept>>,
    pub marketing_status: Option<Vec<MarketingStatus>>,
    pub pharmaceutical_product: Option<Vec<Reference>>,
    pub packaged_medicinal_product: Option<Vec<Reference>>,
    pub attached_document: Option<Vec<Reference>>,
    pub master_file: Option<Vec<Reference>>,
    pub contact: Option<Vec<Reference>>,
    pub clinical_trial: Option<Vec<Reference>>,
    pub name: Option<Vec<MedicinalProductName>>,
    pub cross_reference: Option<Vec<Identifier>>,
    pub manufacturing_business_operation: Option<Vec<MedicinalProductManufacturingBusinessOperation>>,
    pub special_designation: Option<Vec<MedicinalProductSpecialDesignation>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicinalProductName {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub product_name: String,
    pub name_part: Option<Vec<MedicinalProductNameNamePart>>,
    pub country_language: Option<Vec<MedicinalProductNameCountryLanguage>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicinalProductAuthorizationJurisdictionalAuthorization {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub country: Option<CodeableConcept>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub legal_status_of_supply: Option<CodeableConcept>,
    pub validity_period: Option<Period>,
}

/// Choice of types for the date[x] field in MedicinalProductAuthorizationProcedure
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MedicinalProductAuthorizationProcedureDate {
    /// Variant accepting the Period type.
    #[serde(rename = "datePeriod")]
    Period(Period),
    /// Variant accepting the DateTime type.
    #[serde(rename = "dateDateTime")]
    DateTime(DateTime),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicinalProductAuthorizationProcedure {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    pub r#type: CodeableConcept,
    pub date: Option<MedicinalProductAuthorizationProcedureDate>,
    pub application: Option<Vec<MedicinalProductAuthorizationProcedure>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicinalProductAuthorization {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub subject: Option<Reference>,
    pub country: Option<Vec<CodeableConcept>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub status: Option<CodeableConcept>,
    pub status_date: Option<DateTime>,
    pub restore_date: Option<DateTime>,
    pub validity_period: Option<Period>,
    pub data_exclusivity_period: Option<Period>,
    pub date_of_first_authorization: Option<DateTime>,
    pub international_birth_date: Option<DateTime>,
    pub legal_basis: Option<CodeableConcept>,
    pub jurisdictional_authorization: Option<Vec<MedicinalProductAuthorizationJurisdictionalAuthorization>>,
    pub holder: Option<Reference>,
    pub regulator: Option<Reference>,
    pub procedure: Option<MedicinalProductAuthorizationProcedure>,
}


/// Choice of types for the medication[x] field in MedicinalProductContraindicationOtherTherapy
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MedicinalProductContraindicationOtherTherapyMedication {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "medicationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "medicationReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicinalProductContraindicationOtherTherapy {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub therapy_relationship_type: CodeableConcept,
    pub medication: Option<MedicinalProductContraindicationOtherTherapyMedication>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicinalProductContraindication {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub subject: Option<Vec<Reference>>,
    pub disease: Option<CodeableConcept>,
    pub disease_status: Option<CodeableConcept>,
    pub comorbidity: Option<Vec<CodeableConcept>>,
    pub therapeutic_indication: Option<Vec<Reference>>,
    pub other_therapy: Option<Vec<MedicinalProductContraindicationOtherTherapy>>,
    pub population: Option<Vec<Population>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicinalProductIndication {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub subject: Option<Vec<Reference>>,
    pub disease_symptom_procedure: Option<CodeableConcept>,
    pub disease_status: Option<CodeableConcept>,
    pub comorbidity: Option<Vec<CodeableConcept>>,
    pub intended_effect: Option<CodeableConcept>,
    pub duration: Option<Quantity>,
    pub other_therapy: Option<Vec<MedicinalProductIndicationOtherTherapy>>,
    pub undesirable_effect: Option<Vec<Reference>>,
    pub population: Option<Vec<Population>>,
}

/// Choice of types for the medication[x] field in MedicinalProductIndicationOtherTherapy
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MedicinalProductIndicationOtherTherapyMedication {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "medicationCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "medicationReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicinalProductIndicationOtherTherapy {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub therapy_relationship_type: CodeableConcept,
    pub medication: Option<MedicinalProductIndicationOtherTherapyMedication>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicinalProductIngredient {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    pub role: CodeableConcept,
    pub allergenic_indicator: Option<Boolean>,
    pub manufacturer: Option<Vec<Reference>>,
    pub specified_substance: Option<Vec<MedicinalProductIngredientSpecifiedSubstance>>,
    pub substance: Option<MedicinalProductIngredientSubstance>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicinalProductIngredientSubstance {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    pub strength: Option<Vec<MedicinalProductIngredientSpecifiedSubstanceStrength>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicinalProductIngredientSpecifiedSubstanceStrength {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub presentation: Ratio,
    pub presentation_low_limit: Option<Ratio>,
    pub concentration: Option<Ratio>,
    pub concentration_low_limit: Option<Ratio>,
    pub measurement_point: Option<String>,
    pub country: Option<Vec<CodeableConcept>>,
    pub reference_strength: Option<Vec<MedicinalProductIngredientSpecifiedSubstanceStrengthReferenceStrength>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicinalProductIngredientSpecifiedSubstance {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    pub group: CodeableConcept,
    pub confidentiality: Option<CodeableConcept>,
    pub strength: Option<Vec<MedicinalProductIngredientSpecifiedSubstanceStrength>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicinalProductIngredientSpecifiedSubstanceStrengthReferenceStrength {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub substance: Option<CodeableConcept>,
    pub strength: Ratio,
    pub strength_low_limit: Option<Ratio>,
    pub measurement_point: Option<String>,
    pub country: Option<Vec<CodeableConcept>>,
}


/// Choice of types for the item[x] field in MedicinalProductInteractionInteractant
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MedicinalProductInteractionInteractantItem {
    /// Variant accepting the Reference type.
    #[serde(rename = "itemReference")]
    Reference(Reference),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "itemCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicinalProductInteractionInteractant {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub item: Option<MedicinalProductInteractionInteractantItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicinalProductInteraction {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub subject: Option<Vec<Reference>>,
    pub description: Option<String>,
    pub interactant: Option<Vec<MedicinalProductInteractionInteractant>>,
    pub r#type: Option<CodeableConcept>,
    pub effect: Option<CodeableConcept>,
    pub incidence: Option<CodeableConcept>,
    pub management: Option<CodeableConcept>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicinalProductManufactured {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub manufactured_dose_form: CodeableConcept,
    pub unit_of_presentation: Option<CodeableConcept>,
    pub quantity: Quantity,
    pub manufacturer: Option<Vec<Reference>>,
    pub ingredient: Option<Vec<Reference>>,
    pub physical_characteristics: Option<ProdCharacteristic>,
    pub other_characteristics: Option<Vec<CodeableConcept>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicinalProductPackagedPackageItem {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub r#type: CodeableConcept,
    pub quantity: Quantity,
    pub material: Option<Vec<CodeableConcept>>,
    pub alternate_material: Option<Vec<CodeableConcept>>,
    pub device: Option<Vec<Reference>>,
    pub manufactured_item: Option<Vec<Reference>>,
    pub package_item: Option<Vec<MedicinalProductPackagedPackageItem>>,
    pub physical_characteristics: Option<ProdCharacteristic>,
    pub other_characteristics: Option<Vec<CodeableConcept>>,
    pub shelf_life_storage: Option<Vec<ProductShelfLife>>,
    pub manufacturer: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicinalProductPackaged {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub subject: Option<Vec<Reference>>,
    pub description: Option<String>,
    pub legal_status_of_supply: Option<CodeableConcept>,
    pub marketing_status: Option<Vec<MarketingStatus>>,
    pub marketing_authorization: Option<Reference>,
    pub manufacturer: Option<Vec<Reference>>,
    pub batch_identifier: Option<Vec<MedicinalProductPackagedBatchIdentifier>>,
    pub package_item: Option<Vec<MedicinalProductPackagedPackageItem>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicinalProductPackagedBatchIdentifier {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub outer_packaging: Identifier,
    pub immediate_packaging: Option<Identifier>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicinalProductPharmaceuticalRouteOfAdministrationTargetSpeciesWithdrawalPeriod {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub tissue: CodeableConcept,
    pub value: Quantity,
    pub supporting_information: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicinalProductPharmaceutical {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub administrable_dose_form: CodeableConcept,
    pub unit_of_presentation: Option<CodeableConcept>,
    pub ingredient: Option<Vec<Reference>>,
    pub device: Option<Vec<Reference>>,
    pub characteristics: Option<Vec<MedicinalProductPharmaceuticalCharacteristics>>,
    pub route_of_administration: Option<Vec<MedicinalProductPharmaceuticalRouteOfAdministration>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicinalProductPharmaceuticalCharacteristics {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    pub status: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicinalProductPharmaceuticalRouteOfAdministration {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    pub first_dose: Option<Quantity>,
    pub max_single_dose: Option<Quantity>,
    pub max_dose_per_day: Option<Quantity>,
    pub max_dose_per_treatment_period: Option<Ratio>,
    pub max_treatment_period: Option<Duration>,
    pub target_species: Option<Vec<MedicinalProductPharmaceuticalRouteOfAdministrationTargetSpecies>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicinalProductPharmaceuticalRouteOfAdministrationTargetSpecies {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    pub withdrawal_period: Option<Vec<MedicinalProductPharmaceuticalRouteOfAdministrationTargetSpeciesWithdrawalPeriod>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MedicinalProductUndesirableEffect {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub subject: Option<Vec<Reference>>,
    pub symptom_condition_effect: Option<CodeableConcept>,
    pub classification: Option<CodeableConcept>,
    pub frequency_of_occurrence: Option<CodeableConcept>,
    pub population: Option<Vec<Population>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MessageDefinitionAllowedResponse {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub message: Canonical,
    pub situation: Option<Markdown>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MessageDefinitionFocus {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub profile: Option<Canonical>,
    pub min: UnsignedInt,
    pub max: Option<String>,
}

/// Choice of types for the event[x] field in MessageDefinition
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MessageDefinitionEvent {
    /// Variant accepting the Coding type.
    #[serde(rename = "eventCoding")]
    Coding(Coding),
    /// Variant accepting the Uri type.
    #[serde(rename = "eventUri")]
    Uri(Uri),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MessageDefinition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
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
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    pub base: Option<Canonical>,
    pub parent: Option<Vec<Canonical>>,
    pub event: Option<MessageDefinitionEvent>,
    pub category: Option<Code>,
    pub focus: Option<Vec<MessageDefinitionFocus>>,
    pub response_required: Option<Code>,
    pub allowed_response: Option<Vec<MessageDefinitionAllowedResponse>>,
    pub graph: Option<Vec<Canonical>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MessageHeaderDestination {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Option<String>,
    pub target: Option<Reference>,
    pub endpoint: Url,
    pub receiver: Option<Reference>,
}

/// Choice of types for the event[x] field in MessageHeader
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MessageHeaderEvent {
    /// Variant accepting the Coding type.
    #[serde(rename = "eventCoding")]
    Coding(Coding),
    /// Variant accepting the Uri type.
    #[serde(rename = "eventUri")]
    Uri(Uri),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MessageHeader {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub event: Option<MessageHeaderEvent>,
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

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MessageHeaderResponse {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Id,
    pub code: Code,
    pub details: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MessageHeaderSource {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Option<String>,
    pub software: Option<String>,
    pub version: Option<String>,
    pub contact: Option<ContactPoint>,
    pub endpoint: Url,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MolecularSequenceVariant {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub start: Option<Integer>,
    pub end: Option<Integer>,
    pub observed_allele: Option<String>,
    pub reference_allele: Option<String>,
    pub cigar: Option<String>,
    pub variant_pointer: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MolecularSequenceStructureVariantInner {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub start: Option<Integer>,
    pub end: Option<Integer>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MolecularSequenceQualityRoc {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub score: Option<Vec<Integer>>,
    pub num_t_p: Option<Vec<Integer>>,
    pub num_f_p: Option<Vec<Integer>>,
    pub num_f_n: Option<Vec<Integer>>,
    pub precision: Option<Vec<Decimal>>,
    pub sensitivity: Option<Vec<Decimal>>,
    pub f_measure: Option<Vec<Decimal>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MolecularSequenceStructureVariantOuter {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub start: Option<Integer>,
    pub end: Option<Integer>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MolecularSequenceReferenceSeq {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub chromosome: Option<CodeableConcept>,
    pub genome_build: Option<String>,
    pub orientation: Option<Code>,
    pub reference_seq_id: Option<CodeableConcept>,
    pub reference_seq_pointer: Option<Reference>,
    pub reference_seq_string: Option<String>,
    pub strand: Option<Code>,
    pub window_start: Option<Integer>,
    pub window_end: Option<Integer>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MolecularSequenceQuality {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Code,
    pub standard_sequence: Option<CodeableConcept>,
    pub start: Option<Integer>,
    pub end: Option<Integer>,
    pub score: Option<Quantity>,
    pub method: Option<CodeableConcept>,
    pub truth_t_p: Option<Decimal>,
    pub query_t_p: Option<Decimal>,
    pub truth_f_n: Option<Decimal>,
    pub query_f_p: Option<Decimal>,
    pub gt_f_p: Option<Decimal>,
    pub precision: Option<Decimal>,
    pub recall: Option<Decimal>,
    pub f_score: Option<Decimal>,
    pub roc: Option<MolecularSequenceQualityRoc>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MolecularSequenceStructureVariant {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub variant_type: Option<CodeableConcept>,
    pub exact: Option<Boolean>,
    pub length: Option<Integer>,
    pub outer: Option<MolecularSequenceStructureVariantOuter>,
    pub inner: Option<MolecularSequenceStructureVariantInner>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MolecularSequenceRepository {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Code,
    pub url: Option<Uri>,
    pub name: Option<String>,
    pub dataset_id: Option<String>,
    pub variantset_id: Option<String>,
    pub readset_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct MolecularSequence {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub r#type: Option<Code>,
    pub coordinate_system: Integer,
    pub patient: Option<Reference>,
    pub specimen: Option<Reference>,
    pub device: Option<Reference>,
    pub performer: Option<Reference>,
    pub quantity: Option<Quantity>,
    pub reference_seq: Option<MolecularSequenceReferenceSeq>,
    pub variant: Option<Vec<MolecularSequenceVariant>>,
    pub observed_seq: Option<String>,
    pub quality: Option<Vec<MolecularSequenceQuality>>,
    pub read_coverage: Option<Integer>,
    pub repository: Option<Vec<MolecularSequenceRepository>>,
    pub pointer: Option<Vec<Reference>>,
    pub structure_variant: Option<Vec<MolecularSequenceStructureVariant>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct NamingSystemUniqueId {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Code,
    pub value: String,
    pub preferred: Option<Boolean>,
    pub comment: Option<String>,
    pub period: Option<Period>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct NamingSystem {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub status: Code,
    pub kind: Code,
    pub date: DateTime,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub responsible: Option<String>,
    pub r#type: Option<CodeableConcept>,
    pub description: Option<Markdown>,
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub usage: Option<String>,
    pub unique_id: Option<Vec<NamingSystemUniqueId>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct NutritionOrderSupplement {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Option<CodeableConcept>,
    pub product_name: Option<String>,
    pub schedule: Option<Vec<Timing>>,
    pub quantity: Option<Quantity>,
    pub instruction: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct NutritionOrderOralDietTexture {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub modifier: Option<CodeableConcept>,
    pub food_type: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct NutritionOrderEnteralFormula {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub base_formula_type: Option<CodeableConcept>,
    pub base_formula_product_name: Option<String>,
    pub additive_type: Option<CodeableConcept>,
    pub additive_product_name: Option<String>,
    pub caloric_density: Option<Quantity>,
    pub routeof_administration: Option<CodeableConcept>,
    pub administration: Option<Vec<NutritionOrderEnteralFormulaAdministration>>,
    pub max_volume_to_deliver: Option<Quantity>,
    pub administration_instruction: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct NutritionOrder {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub instantiates_canonical: Option<Vec<Canonical>>,
    pub instantiates_uri: Option<Vec<Uri>>,
    pub instantiates: Option<Vec<Uri>>,
    pub status: Code,
    pub intent: Code,
    pub patient: Reference,
    pub encounter: Option<Reference>,
    pub date_time: DateTime,
    pub orderer: Option<Reference>,
    pub allergy_intolerance: Option<Vec<Reference>>,
    pub food_preference_modifier: Option<Vec<CodeableConcept>>,
    pub exclude_food_modifier: Option<Vec<CodeableConcept>>,
    pub oral_diet: Option<NutritionOrderOralDiet>,
    pub supplement: Option<Vec<NutritionOrderSupplement>>,
    pub enteral_formula: Option<NutritionOrderEnteralFormula>,
    pub note: Option<Vec<Annotation>>,
}

/// Choice of types for the rate[x] field in NutritionOrderEnteralFormulaAdministration
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum NutritionOrderEnteralFormulaAdministrationRate {
    /// Variant accepting the Quantity type.
    #[serde(rename = "rateQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Ratio type.
    #[serde(rename = "rateRatio")]
    Ratio(Ratio),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct NutritionOrderEnteralFormulaAdministration {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub schedule: Option<Timing>,
    pub quantity: Option<Quantity>,
    pub rate: Option<NutritionOrderEnteralFormulaAdministrationRate>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct NutritionOrderOralDietNutrient {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub modifier: Option<CodeableConcept>,
    pub amount: Option<Quantity>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct NutritionOrderOralDiet {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Option<Vec<CodeableConcept>>,
    pub schedule: Option<Vec<Timing>>,
    pub nutrient: Option<Vec<NutritionOrderOralDietNutrient>>,
    pub texture: Option<Vec<NutritionOrderOralDietTexture>>,
    pub fluid_consistency_type: Option<Vec<CodeableConcept>>,
    pub instruction: Option<String>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ObservationReferenceRange {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub low: Option<Quantity>,
    pub high: Option<Quantity>,
    pub r#type: Option<CodeableConcept>,
    pub applies_to: Option<Vec<CodeableConcept>>,
    pub age: Option<Range>,
    pub text: Option<String>,
}

/// Choice of types for the value[x] field in ObservationComponent
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ObservationComponentValue {
    /// Variant accepting the Quantity type.
    #[serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the String type.
    #[serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Boolean type.
    #[serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Integer type.
    #[serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Range type.
    #[serde(rename = "valueRange")]
    Range(Range),
    /// Variant accepting the Ratio type.
    #[serde(rename = "valueRatio")]
    Ratio(Ratio),
    /// Variant accepting the SampledData type.
    #[serde(rename = "valueSampledData")]
    SampledData(SampledData),
    /// Variant accepting the Time type.
    #[serde(rename = "valueTime")]
    Time(Time),
    /// Variant accepting the DateTime type.
    #[serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "valuePeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ObservationComponent {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: CodeableConcept,
    pub value: Option<ObservationComponentValue>,
    pub data_absent_reason: Option<CodeableConcept>,
    pub interpretation: Option<Vec<CodeableConcept>>,
    pub reference_range: Option<Vec<ObservationReferenceRange>>,
}

/// Choice of types for the effective[x] field in Observation
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ObservationEffective {
    /// Variant accepting the DateTime type.
    #[serde(rename = "effectiveDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "effectivePeriod")]
    Period(Period),
    /// Variant accepting the Timing type.
    #[serde(rename = "effectiveTiming")]
    Timing(Timing),
    /// Variant accepting the Instant type.
    #[serde(rename = "effectiveInstant")]
    Instant(Instant),
}

/// Choice of types for the value[x] field in Observation
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ObservationValue {
    /// Variant accepting the Quantity type.
    #[serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the String type.
    #[serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Boolean type.
    #[serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Integer type.
    #[serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Range type.
    #[serde(rename = "valueRange")]
    Range(Range),
    /// Variant accepting the Ratio type.
    #[serde(rename = "valueRatio")]
    Ratio(Ratio),
    /// Variant accepting the SampledData type.
    #[serde(rename = "valueSampledData")]
    SampledData(SampledData),
    /// Variant accepting the Time type.
    #[serde(rename = "valueTime")]
    Time(Time),
    /// Variant accepting the DateTime type.
    #[serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "valuePeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Observation {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub based_on: Option<Vec<Reference>>,
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    pub category: Option<Vec<CodeableConcept>>,
    pub code: CodeableConcept,
    pub subject: Option<Reference>,
    pub focus: Option<Vec<Reference>>,
    pub encounter: Option<Reference>,
    pub effective: Option<ObservationEffective>,
    pub issued: Option<Instant>,
    pub performer: Option<Vec<Reference>>,
    pub value: Option<ObservationValue>,
    pub data_absent_reason: Option<CodeableConcept>,
    pub interpretation: Option<Vec<CodeableConcept>>,
    pub note: Option<Vec<Annotation>>,
    pub body_site: Option<CodeableConcept>,
    pub method: Option<CodeableConcept>,
    pub specimen: Option<Reference>,
    pub device: Option<Reference>,
    pub reference_range: Option<Vec<ObservationReferenceRange>>,
    pub has_member: Option<Vec<Reference>>,
    pub derived_from: Option<Vec<Reference>>,
    pub component: Option<Vec<ObservationComponent>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ObservationDefinitionQualifiedInterval {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: Option<Code>,
    pub range: Option<Range>,
    pub context: Option<CodeableConcept>,
    pub applies_to: Option<Vec<CodeableConcept>>,
    pub gender: Option<Code>,
    pub age: Option<Range>,
    pub gestational_age: Option<Range>,
    pub condition: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ObservationDefinition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: Option<Vec<CodeableConcept>>,
    pub code: CodeableConcept,
    pub identifier: Option<Vec<Identifier>>,
    pub permitted_data_type: Option<Vec<Code>>,
    pub multiple_results_allowed: Option<Boolean>,
    pub method: Option<CodeableConcept>,
    pub preferred_report_name: Option<String>,
    pub quantitative_details: Option<ObservationDefinitionQuantitativeDetails>,
    pub qualified_interval: Option<Vec<ObservationDefinitionQualifiedInterval>>,
    pub valid_coded_value_set: Option<Reference>,
    pub normal_coded_value_set: Option<Reference>,
    pub abnormal_coded_value_set: Option<Reference>,
    pub critical_coded_value_set: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ObservationDefinitionQuantitativeDetails {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub customary_unit: Option<CodeableConcept>,
    pub unit: Option<CodeableConcept>,
    pub conversion_factor: Option<Decimal>,
    pub decimal_precision: Option<Integer>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct OperationDefinitionParameterReferencedFrom {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub source: String,
    pub source_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct OperationDefinition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
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
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub affects_state: Option<Boolean>,
    pub code: Code,
    pub comment: Option<Markdown>,
    pub base: Option<Canonical>,
    pub resource: Option<Vec<Code>>,
    pub system: Boolean,
    pub r#type: Boolean,
    pub instance: Boolean,
    pub input_profile: Option<Canonical>,
    pub output_profile: Option<Canonical>,
    pub parameter: Option<Vec<OperationDefinitionParameter>>,
    pub overload: Option<Vec<OperationDefinitionOverload>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct OperationDefinitionParameter {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Code,
    pub r#use: Code,
    pub min: Integer,
    pub max: String,
    pub documentation: Option<String>,
    pub r#type: Option<Code>,
    pub target_profile: Option<Vec<Canonical>>,
    pub search_type: Option<Code>,
    pub binding: Option<OperationDefinitionParameterBinding>,
    pub referenced_from: Option<Vec<OperationDefinitionParameterReferencedFrom>>,
    pub part: Option<Vec<OperationDefinitionParameter>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct OperationDefinitionParameterBinding {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub strength: Code,
    pub value_set: Canonical,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct OperationDefinitionOverload {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub parameter_name: Option<Vec<String>>,
    pub comment: Option<String>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct OperationOutcomeIssue {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub severity: Code,
    pub code: Code,
    pub details: Option<CodeableConcept>,
    pub diagnostics: Option<String>,
    pub location: Option<Vec<String>>,
    pub expression: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct OperationOutcome {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub issue: Option<Vec<OperationOutcomeIssue>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct OrganizationContact {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub purpose: Option<CodeableConcept>,
    pub name: Option<HumanName>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub address: Option<Address>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Organization {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub active: Option<Boolean>,
    pub r#type: Option<Vec<CodeableConcept>>,
    pub name: Option<String>,
    pub alias: Option<Vec<String>>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub address: Option<Vec<Address>>,
    pub part_of: Option<Reference>,
    pub contact: Option<Vec<OrganizationContact>>,
    pub endpoint: Option<Vec<Reference>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct OrganizationAffiliation {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub active: Option<Boolean>,
    pub period: Option<Period>,
    pub organization: Option<Reference>,
    pub participating_organization: Option<Reference>,
    pub network: Option<Vec<Reference>>,
    pub code: Option<Vec<CodeableConcept>>,
    pub specialty: Option<Vec<CodeableConcept>>,
    pub location: Option<Vec<Reference>>,
    pub healthcare_service: Option<Vec<Reference>>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub endpoint: Option<Vec<Reference>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Parameters {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub parameter: Option<Vec<ParametersParameter>>,
}

/// Choice of types for the value[x] field in ParametersParameter
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ParametersParameterValue {
    /// Variant accepting the Base64Binary type.
    #[serde(rename = "valueBase64Binary")]
    Base64Binary(Base64Binary),
    /// Variant accepting the Boolean type.
    #[serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Canonical type.
    #[serde(rename = "valueCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Code type.
    #[serde(rename = "valueCode")]
    Code(Code),
    /// Variant accepting the Date type.
    #[serde(rename = "valueDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Decimal type.
    #[serde(rename = "valueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Id type.
    #[serde(rename = "valueId")]
    Id(Id),
    /// Variant accepting the Instant type.
    #[serde(rename = "valueInstant")]
    Instant(Instant),
    /// Variant accepting the Integer type.
    #[serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Markdown type.
    #[serde(rename = "valueMarkdown")]
    Markdown(Markdown),
    /// Variant accepting the Oid type.
    #[serde(rename = "valueOid")]
    Oid(Oid),
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "valuePositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Time type.
    #[serde(rename = "valueTime")]
    Time(Time),
    /// Variant accepting the UnsignedInt type.
    #[serde(rename = "valueUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Uri type.
    #[serde(rename = "valueUri")]
    Uri(Uri),
    /// Variant accepting the Url type.
    #[serde(rename = "valueUrl")]
    Url(Url),
    /// Variant accepting the Uuid type.
    #[serde(rename = "valueUuid")]
    Uuid(Uuid),
    /// Variant accepting the Address type.
    #[serde(rename = "valueAddress")]
    Address(Address),
    /// Variant accepting the Age type.
    #[serde(rename = "valueAge")]
    Age(Age),
    /// Variant accepting the Annotation type.
    #[serde(rename = "valueAnnotation")]
    Annotation(Annotation),
    /// Variant accepting the Attachment type.
    #[serde(rename = "valueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Coding type.
    #[serde(rename = "valueCoding")]
    Coding(Coding),
    /// Variant accepting the ContactPoint type.
    #[serde(rename = "valueContactPoint")]
    ContactPoint(ContactPoint),
    /// Variant accepting the Count type.
    #[serde(rename = "valueCount")]
    Count(Count),
    /// Variant accepting the Distance type.
    #[serde(rename = "valueDistance")]
    Distance(Distance),
    /// Variant accepting the Duration type.
    #[serde(rename = "valueDuration")]
    Duration(Duration),
    /// Variant accepting the HumanName type.
    #[serde(rename = "valueHumanName")]
    HumanName(HumanName),
    /// Variant accepting the Identifier type.
    #[serde(rename = "valueIdentifier")]
    Identifier(Identifier),
    /// Variant accepting the Money type.
    #[serde(rename = "valueMoney")]
    Money(Money),
    /// Variant accepting the Period type.
    #[serde(rename = "valuePeriod")]
    Period(Period),
    /// Variant accepting the Quantity type.
    #[serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[serde(rename = "valueRange")]
    Range(Range),
    /// Variant accepting the Ratio type.
    #[serde(rename = "valueRatio")]
    Ratio(Ratio),
    /// Variant accepting the Reference type.
    #[serde(rename = "valueReference")]
    Reference(Reference),
    /// Variant accepting the SampledData type.
    #[serde(rename = "valueSampledData")]
    SampledData(SampledData),
    /// Variant accepting the Signature type.
    #[serde(rename = "valueSignature")]
    Signature(Signature),
    /// Variant accepting the Timing type.
    #[serde(rename = "valueTiming")]
    Timing(Timing),
    /// Variant accepting the ContactDetail type.
    #[serde(rename = "valueContactDetail")]
    ContactDetail(ContactDetail),
    /// Variant accepting the Contributor type.
    #[serde(rename = "valueContributor")]
    Contributor(Contributor),
    /// Variant accepting the DataRequirement type.
    #[serde(rename = "valueDataRequirement")]
    DataRequirement(DataRequirement),
    /// Variant accepting the Expression type.
    #[serde(rename = "valueExpression")]
    Expression(Expression),
    /// Variant accepting the ParameterDefinition type.
    #[serde(rename = "valueParameterDefinition")]
    ParameterDefinition(ParameterDefinition),
    /// Variant accepting the RelatedArtifact type.
    #[serde(rename = "valueRelatedArtifact")]
    RelatedArtifact(RelatedArtifact),
    /// Variant accepting the TriggerDefinition type.
    #[serde(rename = "valueTriggerDefinition")]
    TriggerDefinition(TriggerDefinition),
    /// Variant accepting the UsageContext type.
    #[serde(rename = "valueUsageContext")]
    UsageContext(UsageContext),
    /// Variant accepting the Dosage type.
    #[serde(rename = "valueDosage")]
    Dosage(Dosage),
    /// Variant accepting the Meta type.
    #[serde(rename = "valueMeta")]
    Meta(Meta),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ParametersParameter {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub value: Option<ParametersParameterValue>,
    pub resource: Option<Resource>,
    pub part: Option<Vec<ParametersParameter>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct PatientCommunication {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub language: CodeableConcept,
    pub preferred: Option<Boolean>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct PatientContact {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub relationship: Option<Vec<CodeableConcept>>,
    pub name: Option<HumanName>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub address: Option<Address>,
    pub gender: Option<Code>,
    pub organization: Option<Reference>,
    pub period: Option<Period>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct PatientLink {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub other: Reference,
    pub r#type: Code,
}

/// Choice of types for the deceased[x] field in Patient
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PatientDeceased {
    /// Variant accepting the Boolean type.
    #[serde(rename = "deceasedBoolean")]
    Boolean(Boolean),
    /// Variant accepting the DateTime type.
    #[serde(rename = "deceasedDateTime")]
    DateTime(DateTime),
}

/// Choice of types for the multipleBirth[x] field in Patient
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PatientMultipleBirth {
    /// Variant accepting the Boolean type.
    #[serde(rename = "multipleBirthBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Integer type.
    #[serde(rename = "multipleBirthInteger")]
    Integer(Integer),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Patient {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub active: Option<Boolean>,
    pub name: Option<Vec<HumanName>>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub gender: Option<Code>,
    pub birth_date: Option<Date>,
    pub deceased: Option<PatientDeceased>,
    pub address: Option<Vec<Address>>,
    pub marital_status: Option<CodeableConcept>,
    pub multiple_birth: Option<PatientMultipleBirth>,
    pub photo: Option<Vec<Attachment>>,
    pub contact: Option<Vec<PatientContact>>,
    pub communication: Option<Vec<PatientCommunication>>,
    pub general_practitioner: Option<Vec<Reference>>,
    pub managing_organization: Option<Reference>,
    pub link: Option<Vec<PatientLink>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct PaymentNotice {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub request: Option<Reference>,
    pub response: Option<Reference>,
    pub created: DateTime,
    pub provider: Option<Reference>,
    pub payment: Reference,
    pub payment_date: Option<Date>,
    pub payee: Option<Reference>,
    pub recipient: Reference,
    pub amount: Money,
    pub payment_status: Option<CodeableConcept>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct PaymentReconciliationDetail {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    pub predecessor: Option<Identifier>,
    pub r#type: CodeableConcept,
    pub request: Option<Reference>,
    pub submitter: Option<Reference>,
    pub response: Option<Reference>,
    pub date: Option<Date>,
    pub responsible: Option<Reference>,
    pub payee: Option<Reference>,
    pub amount: Option<Money>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct PaymentReconciliation {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub period: Option<Period>,
    pub created: DateTime,
    pub payment_issuer: Option<Reference>,
    pub request: Option<Reference>,
    pub requestor: Option<Reference>,
    pub outcome: Option<Code>,
    pub disposition: Option<String>,
    pub payment_date: Date,
    pub payment_amount: Money,
    pub payment_identifier: Option<Identifier>,
    pub detail: Option<Vec<PaymentReconciliationDetail>>,
    pub form_code: Option<CodeableConcept>,
    pub process_note: Option<Vec<PaymentReconciliationProcessNote>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct PaymentReconciliationProcessNote {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Option<Code>,
    pub text: Option<String>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct PersonLink {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub target: Reference,
    pub assurance: Option<Code>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Person {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub name: Option<Vec<HumanName>>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub gender: Option<Code>,
    pub birth_date: Option<Date>,
    pub address: Option<Vec<Address>>,
    pub photo: Option<Attachment>,
    pub managing_organization: Option<Reference>,
    pub active: Option<Boolean>,
    pub link: Option<Vec<PersonLink>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct PlanDefinitionActionDynamicValue {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub path: Option<String>,
    pub expression: Option<Expression>,
}

/// Choice of types for the subject[x] field in PlanDefinitionAction
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PlanDefinitionActionSubject {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "subjectReference")]
    Reference(Reference),
}

/// Choice of types for the timing[x] field in PlanDefinitionAction
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PlanDefinitionActionTiming {
    /// Variant accepting the DateTime type.
    #[serde(rename = "timingDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Age type.
    #[serde(rename = "timingAge")]
    Age(Age),
    /// Variant accepting the Period type.
    #[serde(rename = "timingPeriod")]
    Period(Period),
    /// Variant accepting the Duration type.
    #[serde(rename = "timingDuration")]
    Duration(Duration),
    /// Variant accepting the Range type.
    #[serde(rename = "timingRange")]
    Range(Range),
    /// Variant accepting the Timing type.
    #[serde(rename = "timingTiming")]
    Timing(Timing),
}

/// Choice of types for the definition[x] field in PlanDefinitionAction
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PlanDefinitionActionDefinition {
    /// Variant accepting the Canonical type.
    #[serde(rename = "definitionCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Uri type.
    #[serde(rename = "definitionUri")]
    Uri(Uri),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct PlanDefinitionAction {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub prefix: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub text_equivalent: Option<String>,
    pub priority: Option<Code>,
    pub code: Option<Vec<CodeableConcept>>,
    pub reason: Option<Vec<CodeableConcept>>,
    pub documentation: Option<Vec<RelatedArtifact>>,
    pub goal_id: Option<Vec<Id>>,
    pub subject: Option<PlanDefinitionActionSubject>,
    pub trigger: Option<Vec<TriggerDefinition>>,
    pub condition: Option<Vec<PlanDefinitionActionCondition>>,
    pub input: Option<Vec<DataRequirement>>,
    pub output: Option<Vec<DataRequirement>>,
    pub related_action: Option<Vec<PlanDefinitionActionRelatedAction>>,
    pub timing: Option<PlanDefinitionActionTiming>,
    pub participant: Option<Vec<PlanDefinitionActionParticipant>>,
    pub r#type: Option<CodeableConcept>,
    pub grouping_behavior: Option<Code>,
    pub selection_behavior: Option<Code>,
    pub required_behavior: Option<Code>,
    pub precheck_behavior: Option<Code>,
    pub cardinality_behavior: Option<Code>,
    pub definition: Option<PlanDefinitionActionDefinition>,
    pub transform: Option<Canonical>,
    pub dynamic_value: Option<Vec<PlanDefinitionActionDynamicValue>>,
    pub action: Option<Vec<PlanDefinitionAction>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct PlanDefinitionActionParticipant {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Code,
    pub role: Option<CodeableConcept>,
}

/// Choice of types for the subject[x] field in PlanDefinition
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PlanDefinitionSubject {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "subjectReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct PlanDefinition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub r#type: Option<CodeableConcept>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub subject: Option<PlanDefinitionSubject>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub usage: Option<String>,
    pub copyright: Option<Markdown>,
    pub approval_date: Option<Date>,
    pub last_review_date: Option<Date>,
    pub effective_period: Option<Period>,
    pub topic: Option<Vec<CodeableConcept>>,
    pub author: Option<Vec<ContactDetail>>,
    pub editor: Option<Vec<ContactDetail>>,
    pub reviewer: Option<Vec<ContactDetail>>,
    pub endorser: Option<Vec<ContactDetail>>,
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    pub library: Option<Vec<Canonical>>,
    pub goal: Option<Vec<PlanDefinitionGoal>>,
    pub action: Option<Vec<PlanDefinitionAction>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct PlanDefinitionActionCondition {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub kind: Code,
    pub expression: Option<Expression>,
}

/// Choice of types for the offset[x] field in PlanDefinitionActionRelatedAction
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PlanDefinitionActionRelatedActionOffset {
    /// Variant accepting the Duration type.
    #[serde(rename = "offsetDuration")]
    Duration(Duration),
    /// Variant accepting the Range type.
    #[serde(rename = "offsetRange")]
    Range(Range),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct PlanDefinitionActionRelatedAction {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub action_id: Id,
    pub relationship: Code,
    pub offset: Option<PlanDefinitionActionRelatedActionOffset>,
}

/// Choice of types for the detail[x] field in PlanDefinitionGoalTarget
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PlanDefinitionGoalTargetDetail {
    /// Variant accepting the Quantity type.
    #[serde(rename = "detailQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[serde(rename = "detailRange")]
    Range(Range),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "detailCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct PlanDefinitionGoalTarget {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub measure: Option<CodeableConcept>,
    pub detail: Option<PlanDefinitionGoalTargetDetail>,
    pub due: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct PlanDefinitionGoal {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: Option<CodeableConcept>,
    pub description: CodeableConcept,
    pub priority: Option<CodeableConcept>,
    pub start: Option<CodeableConcept>,
    pub addresses: Option<Vec<CodeableConcept>>,
    pub documentation: Option<Vec<RelatedArtifact>>,
    pub target: Option<Vec<PlanDefinitionGoalTarget>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Practitioner {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub active: Option<Boolean>,
    pub name: Option<Vec<HumanName>>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub address: Option<Vec<Address>>,
    pub gender: Option<Code>,
    pub birth_date: Option<Date>,
    pub photo: Option<Vec<Attachment>>,
    pub qualification: Option<Vec<PractitionerQualification>>,
    pub communication: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct PractitionerQualification {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub code: CodeableConcept,
    pub period: Option<Period>,
    pub issuer: Option<Reference>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct PractitionerRoleAvailableTime {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub days_of_week: Option<Vec<Code>>,
    pub all_day: Option<Boolean>,
    pub available_start_time: Option<Time>,
    pub available_end_time: Option<Time>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct PractitionerRoleNotAvailable {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: String,
    pub during: Option<Period>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct PractitionerRole {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub active: Option<Boolean>,
    pub period: Option<Period>,
    pub practitioner: Option<Reference>,
    pub organization: Option<Reference>,
    pub code: Option<Vec<CodeableConcept>>,
    pub specialty: Option<Vec<CodeableConcept>>,
    pub location: Option<Vec<Reference>>,
    pub healthcare_service: Option<Vec<Reference>>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub available_time: Option<Vec<PractitionerRoleAvailableTime>>,
    pub not_available: Option<Vec<PractitionerRoleNotAvailable>>,
    pub availability_exceptions: Option<String>,
    pub endpoint: Option<Vec<Reference>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ProcedurePerformer {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub function: Option<CodeableConcept>,
    pub actor: Reference,
    pub on_behalf_of: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ProcedureFocalDevice {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub action: Option<CodeableConcept>,
    pub manipulated: Reference,
}

/// Choice of types for the performed[x] field in Procedure
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ProcedurePerformed {
    /// Variant accepting the DateTime type.
    #[serde(rename = "performedDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "performedPeriod")]
    Period(Period),
    /// Variant accepting the String type.
    #[serde(rename = "performedString")]
    String(String),
    /// Variant accepting the Age type.
    #[serde(rename = "performedAge")]
    Age(Age),
    /// Variant accepting the Range type.
    #[serde(rename = "performedRange")]
    Range(Range),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Procedure {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub instantiates_canonical: Option<Vec<Canonical>>,
    pub instantiates_uri: Option<Vec<Uri>>,
    pub based_on: Option<Vec<Reference>>,
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    pub status_reason: Option<CodeableConcept>,
    pub category: Option<CodeableConcept>,
    pub code: Option<CodeableConcept>,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    pub performed: Option<ProcedurePerformed>,
    pub recorder: Option<Reference>,
    pub asserter: Option<Reference>,
    pub performer: Option<Vec<ProcedurePerformer>>,
    pub location: Option<Reference>,
    pub reason_code: Option<Vec<CodeableConcept>>,
    pub reason_reference: Option<Vec<Reference>>,
    pub body_site: Option<Vec<CodeableConcept>>,
    pub outcome: Option<CodeableConcept>,
    pub report: Option<Vec<Reference>>,
    pub complication: Option<Vec<CodeableConcept>>,
    pub complication_detail: Option<Vec<Reference>>,
    pub follow_up: Option<Vec<CodeableConcept>>,
    pub note: Option<Vec<Annotation>>,
    pub focal_device: Option<Vec<ProcedureFocalDevice>>,
    pub used_reference: Option<Vec<Reference>>,
    pub used_code: Option<Vec<CodeableConcept>>,
}


/// Choice of types for the occurred[x] field in Provenance
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ProvenanceOccurred {
    /// Variant accepting the Period type.
    #[serde(rename = "occurredPeriod")]
    Period(Period),
    /// Variant accepting the DateTime type.
    #[serde(rename = "occurredDateTime")]
    DateTime(DateTime),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Provenance {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub target: Option<Vec<Reference>>,
    pub occurred: Option<ProvenanceOccurred>,
    pub recorded: Instant,
    pub policy: Option<Vec<Uri>>,
    pub location: Option<Reference>,
    pub reason: Option<Vec<CodeableConcept>>,
    pub activity: Option<CodeableConcept>,
    pub agent: Option<Vec<ProvenanceAgent>>,
    pub entity: Option<Vec<ProvenanceEntity>>,
    pub signature: Option<Vec<Signature>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ProvenanceAgent {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Option<CodeableConcept>,
    pub role: Option<Vec<CodeableConcept>>,
    pub who: Reference,
    pub on_behalf_of: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ProvenanceEntity {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub role: Code,
    pub what: Reference,
    pub agent: Option<Vec<ProvenanceAgent>>,
}


/// Choice of types for the answer[x] field in QuestionnaireItemEnableWhen
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum QuestionnaireItemEnableWhenAnswer {
    /// Variant accepting the Boolean type.
    #[serde(rename = "answerBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Decimal type.
    #[serde(rename = "answerDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Integer type.
    #[serde(rename = "answerInteger")]
    Integer(Integer),
    /// Variant accepting the Date type.
    #[serde(rename = "answerDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[serde(rename = "answerDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Time type.
    #[serde(rename = "answerTime")]
    Time(Time),
    /// Variant accepting the String type.
    #[serde(rename = "answerString")]
    String(String),
    /// Variant accepting the Coding type.
    #[serde(rename = "answerCoding")]
    Coding(Coding),
    /// Variant accepting the Quantity type.
    #[serde(rename = "answerQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Reference type.
    #[serde(rename = "answerReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct QuestionnaireItemEnableWhen {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub question: String,
    pub operator: Code,
    pub answer: Option<QuestionnaireItemEnableWhenAnswer>,
}

/// Choice of types for the value[x] field in QuestionnaireItemAnswerOption
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum QuestionnaireItemAnswerOptionValue {
    /// Variant accepting the Integer type.
    #[serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Date type.
    #[serde(rename = "valueDate")]
    Date(Date),
    /// Variant accepting the Time type.
    #[serde(rename = "valueTime")]
    Time(Time),
    /// Variant accepting the String type.
    #[serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Coding type.
    #[serde(rename = "valueCoding")]
    Coding(Coding),
    /// Variant accepting the Reference type.
    #[serde(rename = "valueReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct QuestionnaireItemAnswerOption {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub value: Option<QuestionnaireItemAnswerOptionValue>,
    pub initial_selected: Option<Boolean>,
}

/// Choice of types for the value[x] field in QuestionnaireItemInitial
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum QuestionnaireItemInitialValue {
    /// Variant accepting the Boolean type.
    #[serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Decimal type.
    #[serde(rename = "valueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Integer type.
    #[serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Date type.
    #[serde(rename = "valueDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Time type.
    #[serde(rename = "valueTime")]
    Time(Time),
    /// Variant accepting the String type.
    #[serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Uri type.
    #[serde(rename = "valueUri")]
    Uri(Uri),
    /// Variant accepting the Attachment type.
    #[serde(rename = "valueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Coding type.
    #[serde(rename = "valueCoding")]
    Coding(Coding),
    /// Variant accepting the Quantity type.
    #[serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Reference type.
    #[serde(rename = "valueReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct QuestionnaireItemInitial {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub value: Option<QuestionnaireItemInitialValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Questionnaire {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub derived_from: Option<Vec<Canonical>>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub subject_type: Option<Vec<Code>>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    pub approval_date: Option<Date>,
    pub last_review_date: Option<Date>,
    pub effective_period: Option<Period>,
    pub code: Option<Vec<Coding>>,
    pub item: Option<Vec<QuestionnaireItem>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct QuestionnaireItem {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub link_id: String,
    pub definition: Option<Uri>,
    pub code: Option<Vec<Coding>>,
    pub prefix: Option<String>,
    pub text: Option<String>,
    pub r#type: Code,
    pub enable_when: Option<Vec<QuestionnaireItemEnableWhen>>,
    pub enable_behavior: Option<Code>,
    pub required: Option<Boolean>,
    pub repeats: Option<Boolean>,
    pub read_only: Option<Boolean>,
    pub max_length: Option<Integer>,
    pub answer_value_set: Option<Canonical>,
    pub answer_option: Option<Vec<QuestionnaireItemAnswerOption>>,
    pub initial: Option<Vec<QuestionnaireItemInitial>>,
    pub item: Option<Vec<QuestionnaireItem>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct QuestionnaireResponseItem {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub link_id: String,
    pub definition: Option<Uri>,
    pub text: Option<String>,
    pub answer: Option<Vec<QuestionnaireResponseItemAnswer>>,
    pub item: Option<Vec<QuestionnaireResponseItem>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct QuestionnaireResponse {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    pub based_on: Option<Vec<Reference>>,
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

/// Choice of types for the value[x] field in QuestionnaireResponseItemAnswer
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum QuestionnaireResponseItemAnswerValue {
    /// Variant accepting the Boolean type.
    #[serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Decimal type.
    #[serde(rename = "valueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Integer type.
    #[serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Date type.
    #[serde(rename = "valueDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Time type.
    #[serde(rename = "valueTime")]
    Time(Time),
    /// Variant accepting the String type.
    #[serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Uri type.
    #[serde(rename = "valueUri")]
    Uri(Uri),
    /// Variant accepting the Attachment type.
    #[serde(rename = "valueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the Coding type.
    #[serde(rename = "valueCoding")]
    Coding(Coding),
    /// Variant accepting the Quantity type.
    #[serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Reference type.
    #[serde(rename = "valueReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct QuestionnaireResponseItemAnswer {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub value: Option<QuestionnaireResponseItemAnswerValue>,
    pub item: Option<Vec<QuestionnaireResponseItem>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct RelatedPerson {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub active: Option<Boolean>,
    pub patient: Reference,
    pub relationship: Option<Vec<CodeableConcept>>,
    pub name: Option<Vec<HumanName>>,
    pub telecom: Option<Vec<ContactPoint>>,
    pub gender: Option<Code>,
    pub birth_date: Option<Date>,
    pub address: Option<Vec<Address>>,
    pub photo: Option<Vec<Attachment>>,
    pub period: Option<Period>,
    pub communication: Option<Vec<RelatedPersonCommunication>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct RelatedPersonCommunication {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub language: CodeableConcept,
    pub preferred: Option<Boolean>,
}


/// Choice of types for the timing[x] field in RequestGroupAction
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RequestGroupActionTiming {
    /// Variant accepting the DateTime type.
    #[serde(rename = "timingDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Age type.
    #[serde(rename = "timingAge")]
    Age(Age),
    /// Variant accepting the Period type.
    #[serde(rename = "timingPeriod")]
    Period(Period),
    /// Variant accepting the Duration type.
    #[serde(rename = "timingDuration")]
    Duration(Duration),
    /// Variant accepting the Range type.
    #[serde(rename = "timingRange")]
    Range(Range),
    /// Variant accepting the Timing type.
    #[serde(rename = "timingTiming")]
    Timing(Timing),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct RequestGroupAction {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub prefix: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub text_equivalent: Option<String>,
    pub priority: Option<Code>,
    pub code: Option<Vec<CodeableConcept>>,
    pub documentation: Option<Vec<RelatedArtifact>>,
    pub condition: Option<Vec<RequestGroupActionCondition>>,
    pub related_action: Option<Vec<RequestGroupActionRelatedAction>>,
    pub timing: Option<RequestGroupActionTiming>,
    pub participant: Option<Vec<Reference>>,
    pub r#type: Option<CodeableConcept>,
    pub grouping_behavior: Option<Code>,
    pub selection_behavior: Option<Code>,
    pub required_behavior: Option<Code>,
    pub precheck_behavior: Option<Code>,
    pub cardinality_behavior: Option<Code>,
    pub resource: Option<Reference>,
    pub action: Option<Vec<RequestGroupAction>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct RequestGroup {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub instantiates_canonical: Option<Vec<Canonical>>,
    pub instantiates_uri: Option<Vec<Uri>>,
    pub based_on: Option<Vec<Reference>>,
    pub replaces: Option<Vec<Reference>>,
    pub group_identifier: Option<Identifier>,
    pub status: Code,
    pub intent: Code,
    pub priority: Option<Code>,
    pub code: Option<CodeableConcept>,
    pub subject: Option<Reference>,
    pub encounter: Option<Reference>,
    pub authored_on: Option<DateTime>,
    pub author: Option<Reference>,
    pub reason_code: Option<Vec<CodeableConcept>>,
    pub reason_reference: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    pub action: Option<Vec<RequestGroupAction>>,
}

/// Choice of types for the offset[x] field in RequestGroupActionRelatedAction
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RequestGroupActionRelatedActionOffset {
    /// Variant accepting the Duration type.
    #[serde(rename = "offsetDuration")]
    Duration(Duration),
    /// Variant accepting the Range type.
    #[serde(rename = "offsetRange")]
    Range(Range),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct RequestGroupActionRelatedAction {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub action_id: Id,
    pub relationship: Code,
    pub offset: Option<RequestGroupActionRelatedActionOffset>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct RequestGroupActionCondition {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub kind: Code,
    pub expression: Option<Expression>,
}


/// Choice of types for the subject[x] field in ResearchDefinition
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ResearchDefinitionSubject {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "subjectReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ResearchDefinition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub short_title: Option<String>,
    pub subtitle: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub subject: Option<ResearchDefinitionSubject>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    pub comment: Option<Vec<String>>,
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub usage: Option<String>,
    pub copyright: Option<Markdown>,
    pub approval_date: Option<Date>,
    pub last_review_date: Option<Date>,
    pub effective_period: Option<Period>,
    pub topic: Option<Vec<CodeableConcept>>,
    pub author: Option<Vec<ContactDetail>>,
    pub editor: Option<Vec<ContactDetail>>,
    pub reviewer: Option<Vec<ContactDetail>>,
    pub endorser: Option<Vec<ContactDetail>>,
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    pub library: Option<Vec<Canonical>>,
    pub population: Reference,
    pub exposure: Option<Reference>,
    pub exposure_alternative: Option<Reference>,
    pub outcome: Option<Reference>,
}


/// Choice of types for the subject[x] field in ResearchElementDefinition
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ResearchElementDefinitionSubject {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "subjectCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "subjectReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ResearchElementDefinition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub short_title: Option<String>,
    pub subtitle: Option<String>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub subject: Option<ResearchElementDefinitionSubject>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    pub comment: Option<Vec<String>>,
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub usage: Option<String>,
    pub copyright: Option<Markdown>,
    pub approval_date: Option<Date>,
    pub last_review_date: Option<Date>,
    pub effective_period: Option<Period>,
    pub topic: Option<Vec<CodeableConcept>>,
    pub author: Option<Vec<ContactDetail>>,
    pub editor: Option<Vec<ContactDetail>>,
    pub reviewer: Option<Vec<ContactDetail>>,
    pub endorser: Option<Vec<ContactDetail>>,
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    pub library: Option<Vec<Canonical>>,
    pub r#type: Code,
    pub variable_type: Option<Code>,
    pub characteristic: Option<Vec<ResearchElementDefinitionCharacteristic>>,
}

/// Choice of types for the definition[x] field in ResearchElementDefinitionCharacteristic
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ResearchElementDefinitionCharacteristicDefinition {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "definitionCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Canonical type.
    #[serde(rename = "definitionCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Expression type.
    #[serde(rename = "definitionExpression")]
    Expression(Expression),
    /// Variant accepting the DataRequirement type.
    #[serde(rename = "definitionDataRequirement")]
    DataRequirement(DataRequirement),
}

/// Choice of types for the studyEffective[x] field in ResearchElementDefinitionCharacteristic
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ResearchElementDefinitionCharacteristicStudyEffective {
    /// Variant accepting the DateTime type.
    #[serde(rename = "studyEffectiveDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "studyEffectivePeriod")]
    Period(Period),
    /// Variant accepting the Duration type.
    #[serde(rename = "studyEffectiveDuration")]
    Duration(Duration),
    /// Variant accepting the Timing type.
    #[serde(rename = "studyEffectiveTiming")]
    Timing(Timing),
}

/// Choice of types for the participantEffective[x] field in ResearchElementDefinitionCharacteristic
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ResearchElementDefinitionCharacteristicParticipantEffective {
    /// Variant accepting the DateTime type.
    #[serde(rename = "participantEffectiveDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "participantEffectivePeriod")]
    Period(Period),
    /// Variant accepting the Duration type.
    #[serde(rename = "participantEffectiveDuration")]
    Duration(Duration),
    /// Variant accepting the Timing type.
    #[serde(rename = "participantEffectiveTiming")]
    Timing(Timing),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ResearchElementDefinitionCharacteristic {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub definition: Option<ResearchElementDefinitionCharacteristicDefinition>,
    pub usage_context: Option<Vec<UsageContext>>,
    pub exclude: Option<Boolean>,
    pub unit_of_measure: Option<CodeableConcept>,
    pub study_effective_description: Option<String>,
    pub study_effective: Option<ResearchElementDefinitionCharacteristicStudyEffective>,
    pub study_effective_time_from_start: Option<Duration>,
    pub study_effective_group_measure: Option<Code>,
    pub participant_effective_description: Option<String>,
    pub participant_effective: Option<ResearchElementDefinitionCharacteristicParticipantEffective>,
    pub participant_effective_time_from_start: Option<Duration>,
    pub participant_effective_group_measure: Option<Code>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ResearchStudyArm {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub r#type: Option<CodeableConcept>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ResearchStudy {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub title: Option<String>,
    pub protocol: Option<Vec<Reference>>,
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    pub primary_purpose_type: Option<CodeableConcept>,
    pub phase: Option<CodeableConcept>,
    pub category: Option<Vec<CodeableConcept>>,
    pub focus: Option<Vec<CodeableConcept>>,
    pub condition: Option<Vec<CodeableConcept>>,
    pub contact: Option<Vec<ContactDetail>>,
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    pub keyword: Option<Vec<CodeableConcept>>,
    pub location: Option<Vec<CodeableConcept>>,
    pub description: Option<Markdown>,
    pub enrollment: Option<Vec<Reference>>,
    pub period: Option<Period>,
    pub sponsor: Option<Reference>,
    pub principal_investigator: Option<Reference>,
    pub site: Option<Vec<Reference>>,
    pub reason_stopped: Option<CodeableConcept>,
    pub note: Option<Vec<Annotation>>,
    pub arm: Option<Vec<ResearchStudyArm>>,
    pub objective: Option<Vec<ResearchStudyObjective>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ResearchStudyObjective {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Option<String>,
    pub r#type: Option<CodeableConcept>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ResearchSubject {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub period: Option<Period>,
    pub study: Reference,
    pub individual: Reference,
    pub assigned_arm: Option<String>,
    pub actual_arm: Option<String>,
    pub consent: Option<Reference>,
}


/// Choice of types for the probability[x] field in RiskAssessmentPrediction
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RiskAssessmentPredictionProbability {
    /// Variant accepting the Decimal type.
    #[serde(rename = "probabilityDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Range type.
    #[serde(rename = "probabilityRange")]
    Range(Range),
}

/// Choice of types for the when[x] field in RiskAssessmentPrediction
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RiskAssessmentPredictionWhen {
    /// Variant accepting the Period type.
    #[serde(rename = "whenPeriod")]
    Period(Period),
    /// Variant accepting the Range type.
    #[serde(rename = "whenRange")]
    Range(Range),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct RiskAssessmentPrediction {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub outcome: Option<CodeableConcept>,
    pub probability: Option<RiskAssessmentPredictionProbability>,
    pub qualitative_risk: Option<CodeableConcept>,
    pub relative_risk: Option<Decimal>,
    pub when: Option<RiskAssessmentPredictionWhen>,
    pub rationale: Option<String>,
}

/// Choice of types for the occurrence[x] field in RiskAssessment
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RiskAssessmentOccurrence {
    /// Variant accepting the DateTime type.
    #[serde(rename = "occurrenceDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "occurrencePeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct RiskAssessment {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub based_on: Option<Reference>,
    pub parent: Option<Reference>,
    pub status: Code,
    pub method: Option<CodeableConcept>,
    pub code: Option<CodeableConcept>,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    pub occurrence: Option<RiskAssessmentOccurrence>,
    pub condition: Option<Reference>,
    pub performer: Option<Reference>,
    pub reason_code: Option<Vec<CodeableConcept>>,
    pub reason_reference: Option<Vec<Reference>>,
    pub basis: Option<Vec<Reference>>,
    pub prediction: Option<Vec<RiskAssessmentPrediction>>,
    pub mitigation: Option<String>,
    pub note: Option<Vec<Annotation>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct RiskEvidenceSynthesis {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Option<Uri>,
    pub identifier: Option<Vec<Identifier>>,
    pub version: Option<String>,
    pub name: Option<String>,
    pub title: Option<String>,
    pub status: Code,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Option<Markdown>,
    pub note: Option<Vec<Annotation>>,
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub copyright: Option<Markdown>,
    pub approval_date: Option<Date>,
    pub last_review_date: Option<Date>,
    pub effective_period: Option<Period>,
    pub topic: Option<Vec<CodeableConcept>>,
    pub author: Option<Vec<ContactDetail>>,
    pub editor: Option<Vec<ContactDetail>>,
    pub reviewer: Option<Vec<ContactDetail>>,
    pub endorser: Option<Vec<ContactDetail>>,
    pub related_artifact: Option<Vec<RelatedArtifact>>,
    pub synthesis_type: Option<CodeableConcept>,
    pub study_type: Option<CodeableConcept>,
    pub population: Reference,
    pub exposure: Option<Reference>,
    pub outcome: Reference,
    pub sample_size: Option<RiskEvidenceSynthesisSampleSize>,
    pub risk_estimate: Option<RiskEvidenceSynthesisRiskEstimate>,
    pub certainty: Option<Vec<RiskEvidenceSynthesisCertainty>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct RiskEvidenceSynthesisSampleSize {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    pub number_of_studies: Option<Integer>,
    pub number_of_participants: Option<Integer>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct RiskEvidenceSynthesisRiskEstimate {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    pub r#type: Option<CodeableConcept>,
    pub value: Option<Decimal>,
    pub unit_of_measure: Option<CodeableConcept>,
    pub denominator_count: Option<Integer>,
    pub numerator_count: Option<Integer>,
    pub precision_estimate: Option<Vec<RiskEvidenceSynthesisRiskEstimatePrecisionEstimate>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct RiskEvidenceSynthesisRiskEstimatePrecisionEstimate {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Option<CodeableConcept>,
    pub level: Option<Decimal>,
    pub from: Option<Decimal>,
    pub to: Option<Decimal>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct RiskEvidenceSynthesisCertaintyCertaintySubcomponent {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Option<CodeableConcept>,
    pub rating: Option<Vec<CodeableConcept>>,
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct RiskEvidenceSynthesisCertainty {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub rating: Option<Vec<CodeableConcept>>,
    pub note: Option<Vec<Annotation>>,
    pub certainty_subcomponent: Option<Vec<RiskEvidenceSynthesisCertaintyCertaintySubcomponent>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Schedule {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub active: Option<Boolean>,
    pub service_category: Option<Vec<CodeableConcept>>,
    pub service_type: Option<Vec<CodeableConcept>>,
    pub specialty: Option<Vec<CodeableConcept>>,
    pub actor: Option<Vec<Reference>>,
    pub planning_horizon: Option<Period>,
    pub comment: Option<String>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SearchParameterComponent {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub definition: Canonical,
    pub expression: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SearchParameter {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Uri,
    pub version: Option<String>,
    pub name: String,
    pub derived_from: Option<Canonical>,
    pub status: Code,
    pub experimental: Option<Boolean>,
    pub date: Option<DateTime>,
    pub publisher: Option<String>,
    pub contact: Option<Vec<ContactDetail>>,
    pub description: Markdown,
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub code: Code,
    pub base: Option<Vec<Code>>,
    pub r#type: Code,
    pub expression: Option<String>,
    pub xpath: Option<String>,
    pub xpath_usage: Option<Code>,
    pub target: Option<Vec<Code>>,
    pub multiple_or: Option<Boolean>,
    pub multiple_and: Option<Boolean>,
    pub comparator: Option<Vec<Code>>,
    pub modifier: Option<Vec<Code>>,
    pub chain: Option<Vec<String>>,
    pub component: Option<Vec<SearchParameterComponent>>,
}


/// Choice of types for the quantity[x] field in ServiceRequest
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ServiceRequestQuantity {
    /// Variant accepting the Quantity type.
    #[serde(rename = "quantityQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Ratio type.
    #[serde(rename = "quantityRatio")]
    Ratio(Ratio),
    /// Variant accepting the Range type.
    #[serde(rename = "quantityRange")]
    Range(Range),
}

/// Choice of types for the occurrence[x] field in ServiceRequest
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ServiceRequestOccurrence {
    /// Variant accepting the DateTime type.
    #[serde(rename = "occurrenceDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "occurrencePeriod")]
    Period(Period),
    /// Variant accepting the Timing type.
    #[serde(rename = "occurrenceTiming")]
    Timing(Timing),
}

/// Choice of types for the asNeeded[x] field in ServiceRequest
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ServiceRequestAsNeeded {
    /// Variant accepting the Boolean type.
    #[serde(rename = "asNeededBoolean")]
    Boolean(Boolean),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "asNeededCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ServiceRequest {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub instantiates_canonical: Option<Vec<Canonical>>,
    pub instantiates_uri: Option<Vec<Uri>>,
    pub based_on: Option<Vec<Reference>>,
    pub replaces: Option<Vec<Reference>>,
    pub requisition: Option<Identifier>,
    pub status: Code,
    pub intent: Code,
    pub category: Option<Vec<CodeableConcept>>,
    pub priority: Option<Code>,
    pub do_not_perform: Option<Boolean>,
    pub code: Option<CodeableConcept>,
    pub order_detail: Option<Vec<CodeableConcept>>,
    pub quantity: Option<ServiceRequestQuantity>,
    pub subject: Reference,
    pub encounter: Option<Reference>,
    pub occurrence: Option<ServiceRequestOccurrence>,
    pub as_needed: Option<ServiceRequestAsNeeded>,
    pub authored_on: Option<DateTime>,
    pub requester: Option<Reference>,
    pub performer_type: Option<CodeableConcept>,
    pub performer: Option<Vec<Reference>>,
    pub location_code: Option<Vec<CodeableConcept>>,
    pub location_reference: Option<Vec<Reference>>,
    pub reason_code: Option<Vec<CodeableConcept>>,
    pub reason_reference: Option<Vec<Reference>>,
    pub insurance: Option<Vec<Reference>>,
    pub supporting_info: Option<Vec<Reference>>,
    pub specimen: Option<Vec<Reference>>,
    pub body_site: Option<Vec<CodeableConcept>>,
    pub note: Option<Vec<Annotation>>,
    pub patient_instruction: Option<String>,
    pub relevant_history: Option<Vec<Reference>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Slot {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub service_category: Option<Vec<CodeableConcept>>,
    pub service_type: Option<Vec<CodeableConcept>>,
    pub specialty: Option<Vec<CodeableConcept>>,
    pub appointment_type: Option<CodeableConcept>,
    pub schedule: Reference,
    pub status: Code,
    pub start: Instant,
    pub end: Instant,
    pub overbooked: Option<Boolean>,
    pub comment: Option<String>,
}


/// Choice of types for the additive[x] field in SpecimenContainer
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SpecimenContainerAdditive {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "additiveCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "additiveReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SpecimenContainer {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub description: Option<String>,
    pub r#type: Option<CodeableConcept>,
    pub capacity: Option<Quantity>,
    pub specimen_quantity: Option<Quantity>,
    pub additive: Option<SpecimenContainerAdditive>,
}

/// Choice of types for the time[x] field in SpecimenProcessing
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SpecimenProcessingTime {
    /// Variant accepting the DateTime type.
    #[serde(rename = "timeDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "timePeriod")]
    Period(Period),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SpecimenProcessing {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: Option<String>,
    pub procedure: Option<CodeableConcept>,
    pub additive: Option<Vec<Reference>>,
    pub time: Option<SpecimenProcessingTime>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Specimen {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub accession_identifier: Option<Identifier>,
    pub status: Option<Code>,
    pub r#type: Option<CodeableConcept>,
    pub subject: Option<Reference>,
    pub received_time: Option<DateTime>,
    pub parent: Option<Vec<Reference>>,
    pub request: Option<Vec<Reference>>,
    pub collection: Option<SpecimenCollection>,
    pub processing: Option<Vec<SpecimenProcessing>>,
    pub container: Option<Vec<SpecimenContainer>>,
    pub condition: Option<Vec<CodeableConcept>>,
    pub note: Option<Vec<Annotation>>,
}

/// Choice of types for the collected[x] field in SpecimenCollection
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SpecimenCollectionCollected {
    /// Variant accepting the DateTime type.
    #[serde(rename = "collectedDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "collectedPeriod")]
    Period(Period),
}

/// Choice of types for the fastingStatus[x] field in SpecimenCollection
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SpecimenCollectionFastingStatus {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "fastingStatusCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Duration type.
    #[serde(rename = "fastingStatusDuration")]
    Duration(Duration),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SpecimenCollection {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub collector: Option<Reference>,
    pub collected: Option<SpecimenCollectionCollected>,
    pub duration: Option<Duration>,
    pub quantity: Option<Quantity>,
    pub method: Option<CodeableConcept>,
    pub body_site: Option<CodeableConcept>,
    pub fasting_status: Option<SpecimenCollectionFastingStatus>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SpecimenDefinitionTypeTestedHandling {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub temperature_qualifier: Option<CodeableConcept>,
    pub temperature_range: Option<Range>,
    pub max_duration: Option<Duration>,
    pub instruction: Option<String>,
}

/// Choice of types for the minimumVolume[x] field in SpecimenDefinitionTypeTestedContainer
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SpecimenDefinitionTypeTestedContainerMinimumVolume {
    /// Variant accepting the Quantity type.
    #[serde(rename = "minimumVolumeQuantity")]
    Quantity(Quantity),
    /// Variant accepting the String type.
    #[serde(rename = "minimumVolumeString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SpecimenDefinitionTypeTestedContainer {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub material: Option<CodeableConcept>,
    pub r#type: Option<CodeableConcept>,
    pub cap: Option<CodeableConcept>,
    pub description: Option<String>,
    pub capacity: Option<Quantity>,
    pub minimum_volume: Option<SpecimenDefinitionTypeTestedContainerMinimumVolume>,
    pub additive: Option<Vec<SpecimenDefinitionTypeTestedContainerAdditive>>,
    pub preparation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SpecimenDefinitionTypeTested {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub is_derived: Option<Boolean>,
    pub r#type: Option<CodeableConcept>,
    pub preference: Code,
    pub container: Option<SpecimenDefinitionTypeTestedContainer>,
    pub requirement: Option<String>,
    pub retention_time: Option<Duration>,
    pub rejection_criterion: Option<Vec<CodeableConcept>>,
    pub handling: Option<Vec<SpecimenDefinitionTypeTestedHandling>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SpecimenDefinition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    pub type_collected: Option<CodeableConcept>,
    pub patient_preparation: Option<Vec<CodeableConcept>>,
    pub time_aspect: Option<String>,
    pub collection: Option<Vec<CodeableConcept>>,
    pub type_tested: Option<Vec<SpecimenDefinitionTypeTested>>,
}

/// Choice of types for the additive[x] field in SpecimenDefinitionTypeTestedContainerAdditive
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SpecimenDefinitionTypeTestedContainerAdditiveAdditive {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "additiveCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "additiveReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SpecimenDefinitionTypeTestedContainerAdditive {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub additive: Option<SpecimenDefinitionTypeTestedContainerAdditiveAdditive>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct StructureDefinitionContext {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Code,
    pub expression: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct StructureDefinitionDifferential {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub element: Option<Vec<ElementDefinition>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct StructureDefinitionMapping {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identity: Id,
    pub uri: Option<Uri>,
    pub name: Option<String>,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct StructureDefinition {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
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
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    pub keyword: Option<Vec<Coding>>,
    pub fhir_version: Option<Code>,
    pub mapping: Option<Vec<StructureDefinitionMapping>>,
    pub kind: Code,
    pub r#abstract: Boolean,
    pub context: Option<Vec<StructureDefinitionContext>>,
    pub context_invariant: Option<Vec<String>>,
    pub r#type: Uri,
    pub base_definition: Option<Canonical>,
    pub derivation: Option<Code>,
    pub snapshot: Option<StructureDefinitionSnapshot>,
    pub differential: Option<StructureDefinitionDifferential>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct StructureDefinitionSnapshot {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub element: Option<Vec<ElementDefinition>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct StructureMapGroupRuleDependent {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Id,
    pub variable: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct StructureMapGroupInput {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Id,
    pub r#type: Option<String>,
    pub mode: Code,
    pub documentation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct StructureMapStructure {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Canonical,
    pub mode: Code,
    pub alias: Option<String>,
    pub documentation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct StructureMapGroupRule {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Id,
    pub source: Option<Vec<StructureMapGroupRuleSource>>,
    pub target: Option<Vec<StructureMapGroupRuleTarget>>,
    pub rule: Option<Vec<StructureMapGroupRule>>,
    pub dependent: Option<Vec<StructureMapGroupRuleDependent>>,
    pub documentation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct StructureMapGroupRuleTarget {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub context: Option<Id>,
    pub context_type: Option<Code>,
    pub element: Option<String>,
    pub variable: Option<Id>,
    pub list_mode: Option<Vec<Code>>,
    pub list_rule_id: Option<Id>,
    pub transform: Option<Code>,
    pub parameter: Option<Vec<StructureMapGroupRuleTargetParameter>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct StructureMapGroup {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Id,
    pub extends: Option<Id>,
    pub type_mode: Code,
    pub documentation: Option<String>,
    pub input: Option<Vec<StructureMapGroupInput>>,
    pub rule: Option<Vec<StructureMapGroupRule>>,
}

/// Choice of types for the defaultValue[x] field in StructureMapGroupRuleSource
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum StructureMapGroupRuleSourceDefaultValue {
    /// Variant accepting the Base64Binary type.
    #[serde(rename = "defaultValueBase64Binary")]
    Base64Binary(Base64Binary),
    /// Variant accepting the Boolean type.
    #[serde(rename = "defaultValueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Canonical type.
    #[serde(rename = "defaultValueCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Code type.
    #[serde(rename = "defaultValueCode")]
    Code(Code),
    /// Variant accepting the Date type.
    #[serde(rename = "defaultValueDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[serde(rename = "defaultValueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Decimal type.
    #[serde(rename = "defaultValueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Id type.
    #[serde(rename = "defaultValueId")]
    Id(Id),
    /// Variant accepting the Instant type.
    #[serde(rename = "defaultValueInstant")]
    Instant(Instant),
    /// Variant accepting the Integer type.
    #[serde(rename = "defaultValueInteger")]
    Integer(Integer),
    /// Variant accepting the Markdown type.
    #[serde(rename = "defaultValueMarkdown")]
    Markdown(Markdown),
    /// Variant accepting the Oid type.
    #[serde(rename = "defaultValueOid")]
    Oid(Oid),
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "defaultValuePositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[serde(rename = "defaultValueString")]
    String(String),
    /// Variant accepting the Time type.
    #[serde(rename = "defaultValueTime")]
    Time(Time),
    /// Variant accepting the UnsignedInt type.
    #[serde(rename = "defaultValueUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Uri type.
    #[serde(rename = "defaultValueUri")]
    Uri(Uri),
    /// Variant accepting the Url type.
    #[serde(rename = "defaultValueUrl")]
    Url(Url),
    /// Variant accepting the Uuid type.
    #[serde(rename = "defaultValueUuid")]
    Uuid(Uuid),
    /// Variant accepting the Address type.
    #[serde(rename = "defaultValueAddress")]
    Address(Address),
    /// Variant accepting the Age type.
    #[serde(rename = "defaultValueAge")]
    Age(Age),
    /// Variant accepting the Annotation type.
    #[serde(rename = "defaultValueAnnotation")]
    Annotation(Annotation),
    /// Variant accepting the Attachment type.
    #[serde(rename = "defaultValueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "defaultValueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Coding type.
    #[serde(rename = "defaultValueCoding")]
    Coding(Coding),
    /// Variant accepting the ContactPoint type.
    #[serde(rename = "defaultValueContactPoint")]
    ContactPoint(ContactPoint),
    /// Variant accepting the Count type.
    #[serde(rename = "defaultValueCount")]
    Count(Count),
    /// Variant accepting the Distance type.
    #[serde(rename = "defaultValueDistance")]
    Distance(Distance),
    /// Variant accepting the Duration type.
    #[serde(rename = "defaultValueDuration")]
    Duration(Duration),
    /// Variant accepting the HumanName type.
    #[serde(rename = "defaultValueHumanName")]
    HumanName(HumanName),
    /// Variant accepting the Identifier type.
    #[serde(rename = "defaultValueIdentifier")]
    Identifier(Identifier),
    /// Variant accepting the Money type.
    #[serde(rename = "defaultValueMoney")]
    Money(Money),
    /// Variant accepting the Period type.
    #[serde(rename = "defaultValuePeriod")]
    Period(Period),
    /// Variant accepting the Quantity type.
    #[serde(rename = "defaultValueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[serde(rename = "defaultValueRange")]
    Range(Range),
    /// Variant accepting the Ratio type.
    #[serde(rename = "defaultValueRatio")]
    Ratio(Ratio),
    /// Variant accepting the Reference type.
    #[serde(rename = "defaultValueReference")]
    Reference(Reference),
    /// Variant accepting the SampledData type.
    #[serde(rename = "defaultValueSampledData")]
    SampledData(SampledData),
    /// Variant accepting the Signature type.
    #[serde(rename = "defaultValueSignature")]
    Signature(Signature),
    /// Variant accepting the Timing type.
    #[serde(rename = "defaultValueTiming")]
    Timing(Timing),
    /// Variant accepting the ContactDetail type.
    #[serde(rename = "defaultValueContactDetail")]
    ContactDetail(ContactDetail),
    /// Variant accepting the Contributor type.
    #[serde(rename = "defaultValueContributor")]
    Contributor(Contributor),
    /// Variant accepting the DataRequirement type.
    #[serde(rename = "defaultValueDataRequirement")]
    DataRequirement(DataRequirement),
    /// Variant accepting the Expression type.
    #[serde(rename = "defaultValueExpression")]
    Expression(Expression),
    /// Variant accepting the ParameterDefinition type.
    #[serde(rename = "defaultValueParameterDefinition")]
    ParameterDefinition(ParameterDefinition),
    /// Variant accepting the RelatedArtifact type.
    #[serde(rename = "defaultValueRelatedArtifact")]
    RelatedArtifact(RelatedArtifact),
    /// Variant accepting the TriggerDefinition type.
    #[serde(rename = "defaultValueTriggerDefinition")]
    TriggerDefinition(TriggerDefinition),
    /// Variant accepting the UsageContext type.
    #[serde(rename = "defaultValueUsageContext")]
    UsageContext(UsageContext),
    /// Variant accepting the Dosage type.
    #[serde(rename = "defaultValueDosage")]
    Dosage(Dosage),
    /// Variant accepting the Meta type.
    #[serde(rename = "defaultValueMeta")]
    Meta(Meta),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct StructureMapGroupRuleSource {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub context: Id,
    pub min: Option<Integer>,
    pub max: Option<String>,
    pub r#type: Option<String>,
    pub default_value: Option<StructureMapGroupRuleSourceDefaultValue>,
    pub element: Option<String>,
    pub list_mode: Option<Code>,
    pub variable: Option<Id>,
    pub condition: Option<String>,
    pub check: Option<String>,
    pub log_message: Option<String>,
}

/// Choice of types for the value[x] field in StructureMapGroupRuleTargetParameter
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum StructureMapGroupRuleTargetParameterValue {
    /// Variant accepting the Id type.
    #[serde(rename = "valueId")]
    Id(Id),
    /// Variant accepting the String type.
    #[serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Boolean type.
    #[serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Integer type.
    #[serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Decimal type.
    #[serde(rename = "valueDecimal")]
    Decimal(Decimal),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct StructureMapGroupRuleTargetParameter {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub value: Option<StructureMapGroupRuleTargetParameterValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct StructureMap {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
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
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    pub structure: Option<Vec<StructureMapStructure>>,
    pub import: Option<Vec<Canonical>>,
    pub group: Option<Vec<StructureMapGroup>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Subscription {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub status: Code,
    pub contact: Option<Vec<ContactPoint>>,
    pub end: Option<Instant>,
    pub reason: String,
    pub criteria: String,
    pub error: Option<String>,
    pub channel: SubscriptionChannel,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubscriptionChannel {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Code,
    pub endpoint: Option<Url>,
    pub payload: Option<Code>,
    pub header: Option<Vec<String>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstanceInstance {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    pub expiry: Option<DateTime>,
    pub quantity: Option<Quantity>,
}

/// Choice of types for the substance[x] field in SubstanceIngredient
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SubstanceIngredientSubstance {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "substanceCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "substanceReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstanceIngredient {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub quantity: Option<Ratio>,
    pub substance: Option<SubstanceIngredientSubstance>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Substance {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Option<Code>,
    pub category: Option<Vec<CodeableConcept>>,
    pub code: CodeableConcept,
    pub description: Option<String>,
    pub instance: Option<Vec<SubstanceInstance>>,
    pub ingredient: Option<Vec<SubstanceIngredient>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstanceNucleicAcidSubunit {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub subunit: Option<Integer>,
    pub sequence: Option<String>,
    pub length: Option<Integer>,
    pub sequence_attachment: Option<Attachment>,
    pub five_prime: Option<CodeableConcept>,
    pub three_prime: Option<CodeableConcept>,
    pub linkage: Option<Vec<SubstanceNucleicAcidSubunitLinkage>>,
    pub sugar: Option<Vec<SubstanceNucleicAcidSubunitSugar>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstanceNucleicAcidSubunitSugar {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    pub name: Option<String>,
    pub residue_site: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstanceNucleicAcid {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence_type: Option<CodeableConcept>,
    pub number_of_subunits: Option<Integer>,
    pub area_of_hybridisation: Option<String>,
    pub oligo_nucleotide_type: Option<CodeableConcept>,
    pub subunit: Option<Vec<SubstanceNucleicAcidSubunit>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstanceNucleicAcidSubunitLinkage {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub connectivity: Option<String>,
    pub identifier: Option<Identifier>,
    pub name: Option<String>,
    pub residue_site: Option<String>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstancePolymerRepeat {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub number_of_units: Option<Integer>,
    pub average_molecular_formula: Option<String>,
    pub repeat_unit_amount_type: Option<CodeableConcept>,
    pub repeat_unit: Option<Vec<SubstancePolymerRepeatRepeatUnit>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstancePolymerRepeatRepeatUnitDegreeOfPolymerisation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub degree: Option<CodeableConcept>,
    pub amount: Option<SubstanceAmount>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstancePolymerRepeatRepeatUnitStructuralRepresentation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Option<CodeableConcept>,
    pub representation: Option<String>,
    pub attachment: Option<Attachment>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstancePolymer {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub class: Option<CodeableConcept>,
    pub geometry: Option<CodeableConcept>,
    pub copolymer_connectivity: Option<Vec<CodeableConcept>>,
    pub modification: Option<Vec<String>>,
    pub monomer_set: Option<Vec<SubstancePolymerMonomerSet>>,
    pub repeat: Option<Vec<SubstancePolymerRepeat>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstancePolymerRepeatRepeatUnit {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub orientation_of_polymerisation: Option<CodeableConcept>,
    pub repeat_unit: Option<String>,
    pub amount: Option<SubstanceAmount>,
    pub degree_of_polymerisation: Option<Vec<SubstancePolymerRepeatRepeatUnitDegreeOfPolymerisation>>,
    pub structural_representation: Option<Vec<SubstancePolymerRepeatRepeatUnitStructuralRepresentation>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstancePolymerMonomerSetStartingMaterial {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub material: Option<CodeableConcept>,
    pub r#type: Option<CodeableConcept>,
    pub is_defining: Option<Boolean>,
    pub amount: Option<SubstanceAmount>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstancePolymerMonomerSet {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub ratio_type: Option<CodeableConcept>,
    pub starting_material: Option<Vec<SubstancePolymerMonomerSetStartingMaterial>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstanceProtein {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub sequence_type: Option<CodeableConcept>,
    pub number_of_subunits: Option<Integer>,
    pub disulfide_linkage: Option<Vec<String>>,
    pub subunit: Option<Vec<SubstanceProteinSubunit>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstanceProteinSubunit {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub subunit: Option<Integer>,
    pub sequence: Option<String>,
    pub length: Option<Integer>,
    pub sequence_attachment: Option<Attachment>,
    pub n_terminal_modification_id: Option<Identifier>,
    pub n_terminal_modification: Option<String>,
    pub c_terminal_modification_id: Option<Identifier>,
    pub c_terminal_modification: Option<String>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstanceReferenceInformationClassification {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub domain: Option<CodeableConcept>,
    pub classification: Option<CodeableConcept>,
    pub subtype: Option<Vec<CodeableConcept>>,
    pub source: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstanceReferenceInformation {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub comment: Option<String>,
    pub gene: Option<Vec<SubstanceReferenceInformationGene>>,
    pub gene_element: Option<Vec<SubstanceReferenceInformationGeneElement>>,
    pub classification: Option<Vec<SubstanceReferenceInformationClassification>>,
    pub target: Option<Vec<SubstanceReferenceInformationTarget>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstanceReferenceInformationGene {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub gene_sequence_origin: Option<CodeableConcept>,
    pub gene: Option<CodeableConcept>,
    pub source: Option<Vec<Reference>>,
}

/// Choice of types for the amount[x] field in SubstanceReferenceInformationTarget
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SubstanceReferenceInformationTargetAmount {
    /// Variant accepting the Quantity type.
    #[serde(rename = "amountQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[serde(rename = "amountRange")]
    Range(Range),
    /// Variant accepting the String type.
    #[serde(rename = "amountString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstanceReferenceInformationTarget {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub target: Option<Identifier>,
    pub r#type: Option<CodeableConcept>,
    pub interaction: Option<CodeableConcept>,
    pub organism: Option<CodeableConcept>,
    pub organism_type: Option<CodeableConcept>,
    pub amount: Option<SubstanceReferenceInformationTargetAmount>,
    pub amount_type: Option<CodeableConcept>,
    pub source: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstanceReferenceInformationGeneElement {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Option<CodeableConcept>,
    pub element: Option<Identifier>,
    pub source: Option<Vec<Reference>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstanceSourceMaterialOrganismAuthor {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub author_type: Option<CodeableConcept>,
    pub author_description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstanceSourceMaterial {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub source_material_class: Option<CodeableConcept>,
    pub source_material_type: Option<CodeableConcept>,
    pub source_material_state: Option<CodeableConcept>,
    pub organism_id: Option<Identifier>,
    pub organism_name: Option<String>,
    pub parent_substance_id: Option<Vec<Identifier>>,
    pub parent_substance_name: Option<Vec<String>>,
    pub country_of_origin: Option<Vec<CodeableConcept>>,
    pub geographical_location: Option<Vec<String>>,
    pub development_stage: Option<CodeableConcept>,
    pub fraction_description: Option<Vec<SubstanceSourceMaterialFractionDescription>>,
    pub organism: Option<SubstanceSourceMaterialOrganism>,
    pub part_description: Option<Vec<SubstanceSourceMaterialPartDescription>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstanceSourceMaterialOrganismHybrid {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub maternal_organism_id: Option<String>,
    pub maternal_organism_name: Option<String>,
    pub paternal_organism_id: Option<String>,
    pub paternal_organism_name: Option<String>,
    pub hybrid_type: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstanceSourceMaterialOrganism {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub family: Option<CodeableConcept>,
    pub genus: Option<CodeableConcept>,
    pub species: Option<CodeableConcept>,
    pub intraspecific_type: Option<CodeableConcept>,
    pub intraspecific_description: Option<String>,
    pub author: Option<Vec<SubstanceSourceMaterialOrganismAuthor>>,
    pub hybrid: Option<SubstanceSourceMaterialOrganismHybrid>,
    pub organism_general: Option<SubstanceSourceMaterialOrganismOrganismGeneral>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstanceSourceMaterialOrganismOrganismGeneral {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub kingdom: Option<CodeableConcept>,
    pub phylum: Option<CodeableConcept>,
    pub class: Option<CodeableConcept>,
    pub order: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstanceSourceMaterialFractionDescription {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub fraction: Option<String>,
    pub material_type: Option<CodeableConcept>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstanceSourceMaterialPartDescription {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub part: Option<CodeableConcept>,
    pub part_location: Option<CodeableConcept>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstanceSpecification {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    pub r#type: Option<CodeableConcept>,
    pub status: Option<CodeableConcept>,
    pub domain: Option<CodeableConcept>,
    pub description: Option<String>,
    pub source: Option<Vec<Reference>>,
    pub comment: Option<String>,
    pub moiety: Option<Vec<SubstanceSpecificationMoiety>>,
    pub property: Option<Vec<SubstanceSpecificationProperty>>,
    pub reference_information: Option<Reference>,
    pub structure: Option<SubstanceSpecificationStructure>,
    pub code: Option<Vec<SubstanceSpecificationCode>>,
    pub name: Option<Vec<SubstanceSpecificationName>>,
    pub molecular_weight: Option<Vec<SubstanceSpecificationStructureIsotopeMolecularWeight>>,
    pub relationship: Option<Vec<SubstanceSpecificationRelationship>>,
    pub nucleic_acid: Option<Reference>,
    pub polymer: Option<Reference>,
    pub protein: Option<Reference>,
    pub source_material: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstanceSpecificationStructure {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub stereochemistry: Option<CodeableConcept>,
    pub optical_activity: Option<CodeableConcept>,
    pub molecular_formula: Option<String>,
    pub molecular_formula_by_moiety: Option<String>,
    pub isotope: Option<Vec<SubstanceSpecificationStructureIsotope>>,
    pub molecular_weight: Option<SubstanceSpecificationStructureIsotopeMolecularWeight>,
    pub source: Option<Vec<Reference>>,
    pub representation: Option<Vec<SubstanceSpecificationStructureRepresentation>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstanceSpecificationStructureIsotope {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    pub name: Option<CodeableConcept>,
    pub substitution: Option<CodeableConcept>,
    pub half_life: Option<Quantity>,
    pub molecular_weight: Option<SubstanceSpecificationStructureIsotopeMolecularWeight>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstanceSpecificationStructureIsotopeMolecularWeight {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub method: Option<CodeableConcept>,
    pub r#type: Option<CodeableConcept>,
    pub amount: Option<Quantity>,
}

/// Choice of types for the definingSubstance[x] field in SubstanceSpecificationProperty
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SubstanceSpecificationPropertyDefiningSubstance {
    /// Variant accepting the Reference type.
    #[serde(rename = "definingSubstanceReference")]
    Reference(Reference),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "definingSubstanceCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

/// Choice of types for the amount[x] field in SubstanceSpecificationProperty
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SubstanceSpecificationPropertyAmount {
    /// Variant accepting the Quantity type.
    #[serde(rename = "amountQuantity")]
    Quantity(Quantity),
    /// Variant accepting the String type.
    #[serde(rename = "amountString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstanceSpecificationProperty {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub category: Option<CodeableConcept>,
    pub code: Option<CodeableConcept>,
    pub parameters: Option<String>,
    pub defining_substance: Option<SubstanceSpecificationPropertyDefiningSubstance>,
    pub amount: Option<SubstanceSpecificationPropertyAmount>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstanceSpecificationNameOfficial {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub authority: Option<CodeableConcept>,
    pub status: Option<CodeableConcept>,
    pub date: Option<DateTime>,
}

/// Choice of types for the substance[x] field in SubstanceSpecificationRelationship
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SubstanceSpecificationRelationshipSubstance {
    /// Variant accepting the Reference type.
    #[serde(rename = "substanceReference")]
    Reference(Reference),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "substanceCodeableConcept")]
    CodeableConcept(CodeableConcept),
}

/// Choice of types for the amount[x] field in SubstanceSpecificationRelationship
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SubstanceSpecificationRelationshipAmount {
    /// Variant accepting the Quantity type.
    #[serde(rename = "amountQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[serde(rename = "amountRange")]
    Range(Range),
    /// Variant accepting the Ratio type.
    #[serde(rename = "amountRatio")]
    Ratio(Ratio),
    /// Variant accepting the String type.
    #[serde(rename = "amountString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstanceSpecificationRelationship {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub substance: Option<SubstanceSpecificationRelationshipSubstance>,
    pub relationship: Option<CodeableConcept>,
    pub is_defining: Option<Boolean>,
    pub amount: Option<SubstanceSpecificationRelationshipAmount>,
    pub amount_ratio_low_limit: Option<Ratio>,
    pub amount_type: Option<CodeableConcept>,
    pub source: Option<Vec<Reference>>,
}

/// Choice of types for the amount[x] field in SubstanceSpecificationMoiety
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SubstanceSpecificationMoietyAmount {
    /// Variant accepting the Quantity type.
    #[serde(rename = "amountQuantity")]
    Quantity(Quantity),
    /// Variant accepting the String type.
    #[serde(rename = "amountString")]
    String(String),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstanceSpecificationMoiety {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub role: Option<CodeableConcept>,
    pub identifier: Option<Identifier>,
    pub name: Option<String>,
    pub stereochemistry: Option<CodeableConcept>,
    pub optical_activity: Option<CodeableConcept>,
    pub molecular_formula: Option<String>,
    pub amount: Option<SubstanceSpecificationMoietyAmount>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstanceSpecificationStructureRepresentation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Option<CodeableConcept>,
    pub representation: Option<String>,
    pub attachment: Option<Attachment>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstanceSpecificationCode {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub status: Option<CodeableConcept>,
    pub status_date: Option<DateTime>,
    pub comment: Option<String>,
    pub source: Option<Vec<Reference>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SubstanceSpecificationName {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub r#type: Option<CodeableConcept>,
    pub status: Option<CodeableConcept>,
    pub preferred: Option<Boolean>,
    pub language: Option<Vec<CodeableConcept>>,
    pub domain: Option<Vec<CodeableConcept>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub synonym: Option<Vec<SubstanceSpecificationName>>,
    pub translation: Option<Vec<SubstanceSpecificationName>>,
    pub official: Option<Vec<SubstanceSpecificationNameOfficial>>,
    pub source: Option<Vec<Reference>>,
}


/// Choice of types for the occurrence[x] field in SupplyDelivery
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SupplyDeliveryOccurrence {
    /// Variant accepting the DateTime type.
    #[serde(rename = "occurrenceDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "occurrencePeriod")]
    Period(Period),
    /// Variant accepting the Timing type.
    #[serde(rename = "occurrenceTiming")]
    Timing(Timing),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SupplyDelivery {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub based_on: Option<Vec<Reference>>,
    pub part_of: Option<Vec<Reference>>,
    pub status: Option<Code>,
    pub patient: Option<Reference>,
    pub r#type: Option<CodeableConcept>,
    pub supplied_item: Option<SupplyDeliverySuppliedItem>,
    pub occurrence: Option<SupplyDeliveryOccurrence>,
    pub supplier: Option<Reference>,
    pub destination: Option<Reference>,
    pub receiver: Option<Vec<Reference>>,
}

/// Choice of types for the item[x] field in SupplyDeliverySuppliedItem
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SupplyDeliverySuppliedItemItem {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "itemCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "itemReference")]
    Reference(Reference),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SupplyDeliverySuppliedItem {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub quantity: Option<Quantity>,
    pub item: Option<SupplyDeliverySuppliedItemItem>,
}


/// Choice of types for the item[x] field in SupplyRequest
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SupplyRequestItem {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "itemCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Reference type.
    #[serde(rename = "itemReference")]
    Reference(Reference),
}

/// Choice of types for the occurrence[x] field in SupplyRequest
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SupplyRequestOccurrence {
    /// Variant accepting the DateTime type.
    #[serde(rename = "occurrenceDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Period type.
    #[serde(rename = "occurrencePeriod")]
    Period(Period),
    /// Variant accepting the Timing type.
    #[serde(rename = "occurrenceTiming")]
    Timing(Timing),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SupplyRequest {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Option<Code>,
    pub category: Option<CodeableConcept>,
    pub priority: Option<Code>,
    pub item: Option<SupplyRequestItem>,
    pub quantity: Quantity,
    pub parameter: Option<Vec<SupplyRequestParameter>>,
    pub occurrence: Option<SupplyRequestOccurrence>,
    pub authored_on: Option<DateTime>,
    pub requester: Option<Reference>,
    pub supplier: Option<Vec<Reference>>,
    pub reason_code: Option<Vec<CodeableConcept>>,
    pub reason_reference: Option<Vec<Reference>>,
    pub deliver_from: Option<Reference>,
    pub deliver_to: Option<Reference>,
}

/// Choice of types for the value[x] field in SupplyRequestParameter
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SupplyRequestParameterValue {
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Quantity type.
    #[serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[serde(rename = "valueRange")]
    Range(Range),
    /// Variant accepting the Boolean type.
    #[serde(rename = "valueBoolean")]
    Boolean(Boolean),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct SupplyRequestParameter {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<CodeableConcept>,
    pub value: Option<SupplyRequestParameterValue>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TaskRestriction {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub repetitions: Option<PositiveInt>,
    pub period: Option<Period>,
    pub recipient: Option<Vec<Reference>>,
}

/// Choice of types for the value[x] field in TaskOutput
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TaskOutputValue {
    /// Variant accepting the Base64Binary type.
    #[serde(rename = "valueBase64Binary")]
    Base64Binary(Base64Binary),
    /// Variant accepting the Boolean type.
    #[serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Canonical type.
    #[serde(rename = "valueCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Code type.
    #[serde(rename = "valueCode")]
    Code(Code),
    /// Variant accepting the Date type.
    #[serde(rename = "valueDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Decimal type.
    #[serde(rename = "valueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Id type.
    #[serde(rename = "valueId")]
    Id(Id),
    /// Variant accepting the Instant type.
    #[serde(rename = "valueInstant")]
    Instant(Instant),
    /// Variant accepting the Integer type.
    #[serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Markdown type.
    #[serde(rename = "valueMarkdown")]
    Markdown(Markdown),
    /// Variant accepting the Oid type.
    #[serde(rename = "valueOid")]
    Oid(Oid),
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "valuePositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Time type.
    #[serde(rename = "valueTime")]
    Time(Time),
    /// Variant accepting the UnsignedInt type.
    #[serde(rename = "valueUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Uri type.
    #[serde(rename = "valueUri")]
    Uri(Uri),
    /// Variant accepting the Url type.
    #[serde(rename = "valueUrl")]
    Url(Url),
    /// Variant accepting the Uuid type.
    #[serde(rename = "valueUuid")]
    Uuid(Uuid),
    /// Variant accepting the Address type.
    #[serde(rename = "valueAddress")]
    Address(Address),
    /// Variant accepting the Age type.
    #[serde(rename = "valueAge")]
    Age(Age),
    /// Variant accepting the Annotation type.
    #[serde(rename = "valueAnnotation")]
    Annotation(Annotation),
    /// Variant accepting the Attachment type.
    #[serde(rename = "valueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Coding type.
    #[serde(rename = "valueCoding")]
    Coding(Coding),
    /// Variant accepting the ContactPoint type.
    #[serde(rename = "valueContactPoint")]
    ContactPoint(ContactPoint),
    /// Variant accepting the Count type.
    #[serde(rename = "valueCount")]
    Count(Count),
    /// Variant accepting the Distance type.
    #[serde(rename = "valueDistance")]
    Distance(Distance),
    /// Variant accepting the Duration type.
    #[serde(rename = "valueDuration")]
    Duration(Duration),
    /// Variant accepting the HumanName type.
    #[serde(rename = "valueHumanName")]
    HumanName(HumanName),
    /// Variant accepting the Identifier type.
    #[serde(rename = "valueIdentifier")]
    Identifier(Identifier),
    /// Variant accepting the Money type.
    #[serde(rename = "valueMoney")]
    Money(Money),
    /// Variant accepting the Period type.
    #[serde(rename = "valuePeriod")]
    Period(Period),
    /// Variant accepting the Quantity type.
    #[serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[serde(rename = "valueRange")]
    Range(Range),
    /// Variant accepting the Ratio type.
    #[serde(rename = "valueRatio")]
    Ratio(Ratio),
    /// Variant accepting the Reference type.
    #[serde(rename = "valueReference")]
    Reference(Reference),
    /// Variant accepting the SampledData type.
    #[serde(rename = "valueSampledData")]
    SampledData(SampledData),
    /// Variant accepting the Signature type.
    #[serde(rename = "valueSignature")]
    Signature(Signature),
    /// Variant accepting the Timing type.
    #[serde(rename = "valueTiming")]
    Timing(Timing),
    /// Variant accepting the ContactDetail type.
    #[serde(rename = "valueContactDetail")]
    ContactDetail(ContactDetail),
    /// Variant accepting the Contributor type.
    #[serde(rename = "valueContributor")]
    Contributor(Contributor),
    /// Variant accepting the DataRequirement type.
    #[serde(rename = "valueDataRequirement")]
    DataRequirement(DataRequirement),
    /// Variant accepting the Expression type.
    #[serde(rename = "valueExpression")]
    Expression(Expression),
    /// Variant accepting the ParameterDefinition type.
    #[serde(rename = "valueParameterDefinition")]
    ParameterDefinition(ParameterDefinition),
    /// Variant accepting the RelatedArtifact type.
    #[serde(rename = "valueRelatedArtifact")]
    RelatedArtifact(RelatedArtifact),
    /// Variant accepting the TriggerDefinition type.
    #[serde(rename = "valueTriggerDefinition")]
    TriggerDefinition(TriggerDefinition),
    /// Variant accepting the UsageContext type.
    #[serde(rename = "valueUsageContext")]
    UsageContext(UsageContext),
    /// Variant accepting the Dosage type.
    #[serde(rename = "valueDosage")]
    Dosage(Dosage),
    /// Variant accepting the Meta type.
    #[serde(rename = "valueMeta")]
    Meta(Meta),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TaskOutput {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: CodeableConcept,
    pub value: Option<TaskOutputValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct Task {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub instantiates_canonical: Option<Canonical>,
    pub instantiates_uri: Option<Uri>,
    pub based_on: Option<Vec<Reference>>,
    pub group_identifier: Option<Identifier>,
    pub part_of: Option<Vec<Reference>>,
    pub status: Code,
    pub status_reason: Option<CodeableConcept>,
    pub business_status: Option<CodeableConcept>,
    pub intent: Code,
    pub priority: Option<Code>,
    pub code: Option<CodeableConcept>,
    pub description: Option<String>,
    pub focus: Option<Reference>,
    pub r#for: Option<Reference>,
    pub encounter: Option<Reference>,
    pub execution_period: Option<Period>,
    pub authored_on: Option<DateTime>,
    pub last_modified: Option<DateTime>,
    pub requester: Option<Reference>,
    pub performer_type: Option<Vec<CodeableConcept>>,
    pub owner: Option<Reference>,
    pub location: Option<Reference>,
    pub reason_code: Option<CodeableConcept>,
    pub reason_reference: Option<Reference>,
    pub insurance: Option<Vec<Reference>>,
    pub note: Option<Vec<Annotation>>,
    pub relevant_history: Option<Vec<Reference>>,
    pub restriction: Option<TaskRestriction>,
    pub input: Option<Vec<TaskInput>>,
    pub output: Option<Vec<TaskOutput>>,
}

/// Choice of types for the value[x] field in TaskInput
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TaskInputValue {
    /// Variant accepting the Base64Binary type.
    #[serde(rename = "valueBase64Binary")]
    Base64Binary(Base64Binary),
    /// Variant accepting the Boolean type.
    #[serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Canonical type.
    #[serde(rename = "valueCanonical")]
    Canonical(Canonical),
    /// Variant accepting the Code type.
    #[serde(rename = "valueCode")]
    Code(Code),
    /// Variant accepting the Date type.
    #[serde(rename = "valueDate")]
    Date(Date),
    /// Variant accepting the DateTime type.
    #[serde(rename = "valueDateTime")]
    DateTime(DateTime),
    /// Variant accepting the Decimal type.
    #[serde(rename = "valueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Id type.
    #[serde(rename = "valueId")]
    Id(Id),
    /// Variant accepting the Instant type.
    #[serde(rename = "valueInstant")]
    Instant(Instant),
    /// Variant accepting the Integer type.
    #[serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Markdown type.
    #[serde(rename = "valueMarkdown")]
    Markdown(Markdown),
    /// Variant accepting the Oid type.
    #[serde(rename = "valueOid")]
    Oid(Oid),
    /// Variant accepting the PositiveInt type.
    #[serde(rename = "valuePositiveInt")]
    PositiveInt(PositiveInt),
    /// Variant accepting the String type.
    #[serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Time type.
    #[serde(rename = "valueTime")]
    Time(Time),
    /// Variant accepting the UnsignedInt type.
    #[serde(rename = "valueUnsignedInt")]
    UnsignedInt(UnsignedInt),
    /// Variant accepting the Uri type.
    #[serde(rename = "valueUri")]
    Uri(Uri),
    /// Variant accepting the Url type.
    #[serde(rename = "valueUrl")]
    Url(Url),
    /// Variant accepting the Uuid type.
    #[serde(rename = "valueUuid")]
    Uuid(Uuid),
    /// Variant accepting the Address type.
    #[serde(rename = "valueAddress")]
    Address(Address),
    /// Variant accepting the Age type.
    #[serde(rename = "valueAge")]
    Age(Age),
    /// Variant accepting the Annotation type.
    #[serde(rename = "valueAnnotation")]
    Annotation(Annotation),
    /// Variant accepting the Attachment type.
    #[serde(rename = "valueAttachment")]
    Attachment(Attachment),
    /// Variant accepting the CodeableConcept type.
    #[serde(rename = "valueCodeableConcept")]
    CodeableConcept(CodeableConcept),
    /// Variant accepting the Coding type.
    #[serde(rename = "valueCoding")]
    Coding(Coding),
    /// Variant accepting the ContactPoint type.
    #[serde(rename = "valueContactPoint")]
    ContactPoint(ContactPoint),
    /// Variant accepting the Count type.
    #[serde(rename = "valueCount")]
    Count(Count),
    /// Variant accepting the Distance type.
    #[serde(rename = "valueDistance")]
    Distance(Distance),
    /// Variant accepting the Duration type.
    #[serde(rename = "valueDuration")]
    Duration(Duration),
    /// Variant accepting the HumanName type.
    #[serde(rename = "valueHumanName")]
    HumanName(HumanName),
    /// Variant accepting the Identifier type.
    #[serde(rename = "valueIdentifier")]
    Identifier(Identifier),
    /// Variant accepting the Money type.
    #[serde(rename = "valueMoney")]
    Money(Money),
    /// Variant accepting the Period type.
    #[serde(rename = "valuePeriod")]
    Period(Period),
    /// Variant accepting the Quantity type.
    #[serde(rename = "valueQuantity")]
    Quantity(Quantity),
    /// Variant accepting the Range type.
    #[serde(rename = "valueRange")]
    Range(Range),
    /// Variant accepting the Ratio type.
    #[serde(rename = "valueRatio")]
    Ratio(Ratio),
    /// Variant accepting the Reference type.
    #[serde(rename = "valueReference")]
    Reference(Reference),
    /// Variant accepting the SampledData type.
    #[serde(rename = "valueSampledData")]
    SampledData(SampledData),
    /// Variant accepting the Signature type.
    #[serde(rename = "valueSignature")]
    Signature(Signature),
    /// Variant accepting the Timing type.
    #[serde(rename = "valueTiming")]
    Timing(Timing),
    /// Variant accepting the ContactDetail type.
    #[serde(rename = "valueContactDetail")]
    ContactDetail(ContactDetail),
    /// Variant accepting the Contributor type.
    #[serde(rename = "valueContributor")]
    Contributor(Contributor),
    /// Variant accepting the DataRequirement type.
    #[serde(rename = "valueDataRequirement")]
    DataRequirement(DataRequirement),
    /// Variant accepting the Expression type.
    #[serde(rename = "valueExpression")]
    Expression(Expression),
    /// Variant accepting the ParameterDefinition type.
    #[serde(rename = "valueParameterDefinition")]
    ParameterDefinition(ParameterDefinition),
    /// Variant accepting the RelatedArtifact type.
    #[serde(rename = "valueRelatedArtifact")]
    RelatedArtifact(RelatedArtifact),
    /// Variant accepting the TriggerDefinition type.
    #[serde(rename = "valueTriggerDefinition")]
    TriggerDefinition(TriggerDefinition),
    /// Variant accepting the UsageContext type.
    #[serde(rename = "valueUsageContext")]
    UsageContext(UsageContext),
    /// Variant accepting the Dosage type.
    #[serde(rename = "valueDosage")]
    Dosage(Dosage),
    /// Variant accepting the Meta type.
    #[serde(rename = "valueMeta")]
    Meta(Meta),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TaskInput {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: CodeableConcept,
    pub value: Option<TaskInputValue>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TerminologyCapabilitiesTranslation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub needs_map: Boolean,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TerminologyCapabilitiesExpansionParameter {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Code,
    pub documentation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TerminologyCapabilitiesValidateCode {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub translations: Boolean,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TerminologyCapabilitiesImplementation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub description: String,
    pub url: Option<Url>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TerminologyCapabilitiesClosure {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub translation: Option<Boolean>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TerminologyCapabilitiesCodeSystem {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub uri: Option<Canonical>,
    pub version: Option<Vec<TerminologyCapabilitiesCodeSystemVersion>>,
    pub subsumption: Option<Boolean>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TerminologyCapabilitiesSoftware {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TerminologyCapabilitiesCodeSystemVersion {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Option<String>,
    pub is_default: Option<Boolean>,
    pub compositional: Option<Boolean>,
    pub language: Option<Vec<Code>>,
    pub filter: Option<Vec<TerminologyCapabilitiesCodeSystemVersionFilter>>,
    pub property: Option<Vec<Code>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TerminologyCapabilitiesExpansion {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub hierarchical: Option<Boolean>,
    pub paging: Option<Boolean>,
    pub incomplete: Option<Boolean>,
    pub parameter: Option<Vec<TerminologyCapabilitiesExpansionParameter>>,
    pub text_filter: Option<Markdown>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TerminologyCapabilities {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
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
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    pub kind: Code,
    pub software: Option<TerminologyCapabilitiesSoftware>,
    pub implementation: Option<TerminologyCapabilitiesImplementation>,
    pub locked_date: Option<Boolean>,
    pub code_system: Option<Vec<TerminologyCapabilitiesCodeSystem>>,
    pub expansion: Option<TerminologyCapabilitiesExpansion>,
    pub code_search: Option<Code>,
    pub validate_code: Option<TerminologyCapabilitiesValidateCode>,
    pub translation: Option<TerminologyCapabilitiesTranslation>,
    pub closure: Option<TerminologyCapabilitiesClosure>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TerminologyCapabilitiesCodeSystemVersionFilter {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub op: Option<Vec<Code>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TestReportTestAction {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub operation: Option<TestReportSetupActionOperation>,
    pub assert: Option<TestReportSetupActionAssert>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TestReportSetupActionAssert {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub result: Code,
    pub message: Option<Markdown>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TestReportTest {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub action: Option<Vec<TestReportTestAction>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TestReportSetup {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub action: Option<Vec<TestReportSetupAction>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TestReportSetupAction {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub operation: Option<TestReportSetupActionOperation>,
    pub assert: Option<TestReportSetupActionAssert>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TestReportSetupActionOperation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub result: Code,
    pub message: Option<Markdown>,
    pub detail: Option<Uri>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TestReportTeardown {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub action: Option<Vec<TestReportTeardownAction>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TestReportTeardownAction {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub operation: TestReportSetupActionOperation,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TestReport {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Identifier>,
    pub name: Option<String>,
    pub status: Code,
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

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TestReportParticipant {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Code,
    pub uri: Uri,
    pub display: Option<String>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TestScriptMetadataCapability {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub required: Boolean,
    pub validated: Boolean,
    pub description: Option<String>,
    pub origin: Option<Vec<Integer>>,
    pub destination: Option<Integer>,
    pub link: Option<Vec<Uri>>,
    pub capabilities: Canonical,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TestScriptOrigin {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub index: Integer,
    pub profile: Coding,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TestScript {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
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

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TestScriptFixture {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub autocreate: Boolean,
    pub autodelete: Boolean,
    pub resource: Option<Reference>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TestScriptMetadata {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub link: Option<Vec<TestScriptMetadataLink>>,
    pub capability: Option<Vec<TestScriptMetadataCapability>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TestScriptMetadataLink {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub url: Uri,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TestScriptTeardownAction {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub operation: TestScriptSetupActionOperation,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TestScriptSetup {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub action: Option<Vec<TestScriptSetupAction>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TestScriptSetupActionAssert {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub label: Option<String>,
    pub description: Option<String>,
    pub direction: Option<Code>,
    pub compare_to_source_id: Option<String>,
    pub compare_to_source_expression: Option<String>,
    pub compare_to_source_path: Option<String>,
    pub content_type: Option<Code>,
    pub expression: Option<String>,
    pub header_field: Option<String>,
    pub minimum_id: Option<String>,
    pub navigation_links: Option<Boolean>,
    pub operator: Option<Code>,
    pub path: Option<String>,
    pub request_method: Option<Code>,
    pub request_u_r_l: Option<String>,
    pub resource: Option<Code>,
    pub response: Option<Code>,
    pub response_code: Option<String>,
    pub source_id: Option<Id>,
    pub validate_profile_id: Option<Id>,
    pub value: Option<String>,
    pub warning_only: Boolean,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TestScriptTest {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub action: Option<Vec<TestScriptTestAction>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TestScriptTeardown {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub action: Option<Vec<TestScriptTeardownAction>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TestScriptSetupActionOperationRequestHeader {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub field: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TestScriptTestAction {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub operation: Option<TestScriptSetupActionOperation>,
    pub assert: Option<TestScriptSetupActionAssert>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TestScriptVariable {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub default_value: Option<String>,
    pub description: Option<String>,
    pub expression: Option<String>,
    pub header_field: Option<String>,
    pub hint: Option<String>,
    pub path: Option<String>,
    pub source_id: Option<Id>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TestScriptDestination {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub index: Integer,
    pub profile: Coding,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TestScriptSetupAction {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub operation: Option<TestScriptSetupActionOperation>,
    pub assert: Option<TestScriptSetupActionAssert>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct TestScriptSetupActionOperation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub r#type: Option<Coding>,
    pub resource: Option<Code>,
    pub label: Option<String>,
    pub description: Option<String>,
    pub accept: Option<Code>,
    pub content_type: Option<Code>,
    pub destination: Option<Integer>,
    pub encode_request_url: Boolean,
    pub method: Option<Code>,
    pub origin: Option<Integer>,
    pub params: Option<String>,
    pub request_header: Option<Vec<TestScriptSetupActionOperationRequestHeader>>,
    pub request_id: Option<Id>,
    pub response_id: Option<Id>,
    pub source_id: Option<Id>,
    pub target_id: Option<Id>,
    pub url: Option<String>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ValueSet {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
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
    pub use_context: Option<Vec<UsageContext>>,
    pub jurisdiction: Option<Vec<CodeableConcept>>,
    pub immutable: Option<Boolean>,
    pub purpose: Option<Markdown>,
    pub copyright: Option<Markdown>,
    pub compose: Option<ValueSetCompose>,
    pub expansion: Option<ValueSetExpansion>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ValueSetComposeIncludeFilter {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub property: Code,
    pub op: Code,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ValueSetExpansionContains {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub system: Option<Uri>,
    pub r#abstract: Option<Boolean>,
    pub inactive: Option<Boolean>,
    pub version: Option<String>,
    pub code: Option<Code>,
    pub display: Option<String>,
    pub designation: Option<Vec<ValueSetComposeIncludeConceptDesignation>>,
    pub contains: Option<Vec<ValueSetExpansionContains>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ValueSetComposeIncludeConcept {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub code: Code,
    pub display: Option<String>,
    pub designation: Option<Vec<ValueSetComposeIncludeConceptDesignation>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ValueSetComposeIncludeConceptDesignation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub language: Option<Code>,
    pub r#use: Option<Coding>,
    pub value: String,
}

/// Choice of types for the value[x] field in ValueSetExpansionParameter
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ValueSetExpansionParameterValue {
    /// Variant accepting the String type.
    #[serde(rename = "valueString")]
    String(String),
    /// Variant accepting the Boolean type.
    #[serde(rename = "valueBoolean")]
    Boolean(Boolean),
    /// Variant accepting the Integer type.
    #[serde(rename = "valueInteger")]
    Integer(Integer),
    /// Variant accepting the Decimal type.
    #[serde(rename = "valueDecimal")]
    Decimal(Decimal),
    /// Variant accepting the Uri type.
    #[serde(rename = "valueUri")]
    Uri(Uri),
    /// Variant accepting the Code type.
    #[serde(rename = "valueCode")]
    Code(Code),
    /// Variant accepting the DateTime type.
    #[serde(rename = "valueDateTime")]
    DateTime(DateTime),
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ValueSetExpansionParameter {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub name: String,
    pub value: Option<ValueSetExpansionParameterValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ValueSetExpansion {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Uri>,
    pub timestamp: DateTime,
    pub total: Option<Integer>,
    pub offset: Option<Integer>,
    pub parameter: Option<Vec<ValueSetExpansionParameter>>,
    pub contains: Option<Vec<ValueSetExpansionContains>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ValueSetCompose {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub locked_date: Option<Date>,
    pub inactive: Option<Boolean>,
    pub include: Option<Vec<ValueSetComposeInclude>>,
    pub exclude: Option<Vec<ValueSetComposeInclude>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct ValueSetComposeInclude {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub system: Option<Uri>,
    pub version: Option<String>,
    pub concept: Option<Vec<ValueSetComposeIncludeConcept>>,
    pub filter: Option<Vec<ValueSetComposeIncludeFilter>>,
    pub value_set: Option<Vec<Canonical>>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct VerificationResult {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub target: Option<Vec<Reference>>,
    pub target_location: Option<Vec<String>>,
    pub need: Option<CodeableConcept>,
    pub status: Code,
    pub status_date: Option<DateTime>,
    pub validation_type: Option<CodeableConcept>,
    pub validation_process: Option<Vec<CodeableConcept>>,
    pub frequency: Option<Timing>,
    pub last_performed: Option<DateTime>,
    pub next_scheduled: Option<Date>,
    pub failure_action: Option<CodeableConcept>,
    pub primary_source: Option<Vec<VerificationResultPrimarySource>>,
    pub attestation: Option<VerificationResultAttestation>,
    pub validator: Option<Vec<VerificationResultValidator>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct VerificationResultValidator {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub organization: Reference,
    pub identity_certificate: Option<String>,
    pub attestation_signature: Option<Signature>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct VerificationResultPrimarySource {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub who: Option<Reference>,
    pub r#type: Option<Vec<CodeableConcept>>,
    pub communication_method: Option<Vec<CodeableConcept>>,
    pub validation_status: Option<CodeableConcept>,
    pub validation_date: Option<DateTime>,
    pub can_push_updates: Option<CodeableConcept>,
    pub push_type_available: Option<Vec<CodeableConcept>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct VerificationResultAttestation {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub who: Option<Reference>,
    pub on_behalf_of: Option<Reference>,
    pub communication_method: Option<CodeableConcept>,
    pub date: Option<Date>,
    pub source_identity_certificate: Option<String>,
    pub proxy_identity_certificate: Option<String>,
    pub proxy_signature: Option<Signature>,
    pub source_signature: Option<Signature>,
}


#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct VisionPrescription {
    pub id: Option<std::string::String>,
    pub meta: Option<Meta>,
    pub implicit_rules: Option<Uri>,
    pub language: Option<Code>,
    pub text: Option<Narrative>,
    pub contained: Option<Vec<Resource>>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub identifier: Option<Vec<Identifier>>,
    pub status: Code,
    pub created: DateTime,
    pub patient: Reference,
    pub encounter: Option<Reference>,
    pub date_written: DateTime,
    pub prescriber: Reference,
    pub lens_specification: Option<Vec<VisionPrescriptionLensSpecification>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct VisionPrescriptionLensSpecification {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub product: CodeableConcept,
    pub eye: Code,
    pub sphere: Option<Decimal>,
    pub cylinder: Option<Decimal>,
    pub axis: Option<Integer>,
    pub prism: Option<Vec<VisionPrescriptionLensSpecificationPrism>>,
    pub add: Option<Decimal>,
    pub power: Option<Decimal>,
    pub back_curve: Option<Decimal>,
    pub diameter: Option<Decimal>,
    pub duration: Option<Quantity>,
    pub color: Option<String>,
    pub brand: Option<String>,
    pub note: Option<Vec<Annotation>>,
}

#[derive(Debug, Clone, PartialEq, Eq, FhirSerde, Default)]
pub struct VisionPrescriptionLensSpecificationPrism {
    pub id: Option<std::string::String>,
    pub extension: Option<Vec<Extension>>,
    pub modifier_extension: Option<Vec<Extension>>,
    pub amount: Decimal,
    pub base: Code,
}


#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(tag = "resourceType")]
pub enum Resource {
    Account(Account),
    ActivityDefinition(ActivityDefinition),
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
    Claim(Claim),
    ClaimResponse(ClaimResponse),
    ClinicalImpression(ClinicalImpression),
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
    EffectEvidenceSynthesis(EffectEvidenceSynthesis),
    Encounter(Encounter),
    Endpoint(Endpoint),
    EnrollmentRequest(EnrollmentRequest),
    EnrollmentResponse(EnrollmentResponse),
    EpisodeOfCare(EpisodeOfCare),
    EventDefinition(EventDefinition),
    Evidence(Evidence),
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
    InsurancePlan(InsurancePlan),
    Invoice(Invoice),
    Library(Library),
    Linkage(Linkage),
    List(List),
    Location(Location),
    Measure(Measure),
    MeasureReport(MeasureReport),
    Media(Media),
    Medication(Medication),
    MedicationAdministration(MedicationAdministration),
    MedicationDispense(MedicationDispense),
    MedicationKnowledge(MedicationKnowledge),
    MedicationRequest(MedicationRequest),
    MedicationStatement(MedicationStatement),
    MedicinalProduct(MedicinalProduct),
    MedicinalProductAuthorization(MedicinalProductAuthorization),
    MedicinalProductContraindication(MedicinalProductContraindication),
    MedicinalProductIndication(MedicinalProductIndication),
    MedicinalProductIngredient(MedicinalProductIngredient),
    MedicinalProductInteraction(MedicinalProductInteraction),
    MedicinalProductManufactured(MedicinalProductManufactured),
    MedicinalProductPackaged(MedicinalProductPackaged),
    MedicinalProductPharmaceutical(MedicinalProductPharmaceutical),
    MedicinalProductUndesirableEffect(MedicinalProductUndesirableEffect),
    MessageDefinition(MessageDefinition),
    MessageHeader(MessageHeader),
    MolecularSequence(MolecularSequence),
    NamingSystem(NamingSystem),
    NutritionOrder(NutritionOrder),
    Observation(Observation),
    ObservationDefinition(ObservationDefinition),
    OperationDefinition(OperationDefinition),
    OperationOutcome(OperationOutcome),
    Organization(Organization),
    OrganizationAffiliation(OrganizationAffiliation),
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
    RelatedPerson(RelatedPerson),
    RequestGroup(RequestGroup),
    ResearchDefinition(ResearchDefinition),
    ResearchElementDefinition(ResearchElementDefinition),
    ResearchStudy(ResearchStudy),
    ResearchSubject(ResearchSubject),
    RiskAssessment(RiskAssessment),
    RiskEvidenceSynthesis(RiskEvidenceSynthesis),
    Schedule(Schedule),
    SearchParameter(SearchParameter),
    ServiceRequest(ServiceRequest),
    Slot(Slot),
    Specimen(Specimen),
    SpecimenDefinition(SpecimenDefinition),
    StructureDefinition(StructureDefinition),
    StructureMap(StructureMap),
    Subscription(Subscription),
    Substance(Substance),
    SubstanceNucleicAcid(SubstanceNucleicAcid),
    SubstancePolymer(SubstancePolymer),
    SubstanceProtein(SubstanceProtein),
    SubstanceReferenceInformation(SubstanceReferenceInformation),
    SubstanceSourceMaterial(SubstanceSourceMaterial),
    SubstanceSpecification(SubstanceSpecification),
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


// --- From<T> Implementations for Element<T, Extension> ---
impl From<bool> for Element<bool, Extension> {
    fn from(value: bool) -> Self {
        Self { value: Some(value), ..Default::default() }
    }
}
impl From<std::primitive::i32> for Element<std::primitive::i32, Extension> {
    fn from(value: std::primitive::i32) -> Self {
        Self { value: Some(value), ..Default::default() }
    }
}
impl From<std::string::String> for Element<std::string::String, Extension> {
    fn from(value: std::string::String) -> Self {
        Self { value: Some(value), ..Default::default() }
    }
}
// --- End From<T> Implementations ---
