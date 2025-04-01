use fhir::r4;
use fhir_macro::FhirSerde;

#[derive(Debug, PartialEq, FhirSerde)] // Use FhirSerde derive
struct FhirSerdeTestStruct2 {
    // Regular field
    name: Option<String>,

    // Field with potential extension (_birthDate) using type alias
    // FhirSerde should handle the 'birthDate'/'_birthDate' logic based on the field name.
    #[rustfmt::skip]
        birth_date: Option<r4::Date>, // Use type alias like in Patient

    // Another potentially extended field using type alias
    // FhirSerde should handle the 'isActive'/'_isActive' logic based on the field name.
    #[rustfmt::skip]
        is_active: Option<r4::Boolean>, // Use type alias

    // A non-element field for good measure
    count: Option<i32>,
}
