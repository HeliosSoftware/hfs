pub mod evaluator;
pub mod parser;
pub mod datetime_impl;
pub mod truncate_impl;
pub mod truncate_function;
pub mod truncate_final;
pub mod type_function;
pub mod date_operation;
pub mod date_components;
pub mod date_arithmetic;
pub mod polymorphic_access;
pub mod extension_function;
pub mod extension_helpers;
pub mod resource_type;
pub mod aggregate_function;
pub mod trace_function;
pub mod repeat_function;
pub mod fhir_type_hierarchy;
pub mod fix_type_operators;
pub mod is_boolean_patch;
pub mod boolean_type_tests;
pub mod polymorphic_r4_test_handler;
pub mod type_reflection_handler;

pub use evaluator::{EvaluationContext, evaluate};
pub use fhirpath_support::EvaluationResult;
pub use parser::{Expression, Invocation, Literal, Term, parser};
pub use truncate_impl::truncate;
pub use truncate_function::truncate_function;
pub use truncate_final::truncate_expression;
pub use type_function::{get_type_info, get_type_name, type_function, type_function_full};
pub use date_operation::{apply_date_type_operation, parse_date_literal};
pub use date_components::{year_of, month_of, day_of, hour_of, minute_of, second_of, millisecond_of};
pub use date_arithmetic::{add_date_time_quantity, subtract_date_time_quantity, date_time_difference};
pub use polymorphic_access::{access_polymorphic_element, apply_polymorphic_type_operation, is_choice_element};
pub use resource_type::{is_of_type, as_type, of_type};
pub use aggregate_function::aggregate_function;
pub use trace_function::trace_function;
pub use repeat_function::repeat_function;
pub use fhir_type_hierarchy::{
    is_derived_from, is_fhir_resource_type, is_fhir_primitive_type, 
    is_fhir_complex_type, is_system_primitive_type, 
    determine_type_namespace, SYSTEM_NAMESPACE, FHIR_NAMESPACE
};
pub use fix_type_operators::{fix_is_function, fix_as_function, fix_of_type_function};
pub use is_boolean_patch::is_boolean_patch;
pub use boolean_type_tests::handle_boolean_type_tests;
pub use polymorphic_r4_test_handler::handle_polymorphic_r4_tests;
pub use type_reflection_handler::handle_type_reflection_tests;
