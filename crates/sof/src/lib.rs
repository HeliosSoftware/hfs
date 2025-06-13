mod traits;

use fhir::FhirVersion;
use fhirpath::{EvaluationContext, EvaluationResult, evaluate_expression};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;
use traits::*;

/// Multi-version ViewDefinition container supporting version-agnostic operations.
#[derive(Debug, Clone)]
pub enum SofViewDefinition {
    #[cfg(feature = "R4")]
    R4(fhir::r4::ViewDefinition),
    #[cfg(feature = "R4B")]
    R4B(fhir::r4b::ViewDefinition),
    #[cfg(feature = "R5")]
    R5(fhir::r5::ViewDefinition),
    #[cfg(feature = "R6")]
    R6(fhir::r6::ViewDefinition),
}

impl SofViewDefinition {
    pub fn version(&self) -> FhirVersion {
        match self {
            #[cfg(feature = "R4")]
            SofViewDefinition::R4(_) => FhirVersion::R4,
            #[cfg(feature = "R4B")]
            SofViewDefinition::R4B(_) => FhirVersion::R4B,
            #[cfg(feature = "R5")]
            SofViewDefinition::R5(_) => FhirVersion::R5,
            #[cfg(feature = "R6")]
            SofViewDefinition::R6(_) => FhirVersion::R6,
        }
    }
}

/// Multi-version Bundle container supporting version-agnostic operations.
#[derive(Debug, Clone)]
pub enum SofBundle {
    #[cfg(feature = "R4")]
    R4(fhir::r4::Bundle),
    #[cfg(feature = "R4B")]
    R4B(fhir::r4b::Bundle),
    #[cfg(feature = "R5")]
    R5(fhir::r5::Bundle),
    #[cfg(feature = "R6")]
    R6(fhir::r6::Bundle),
}

impl SofBundle {
    pub fn version(&self) -> FhirVersion {
        match self {
            #[cfg(feature = "R4")]
            SofBundle::R4(_) => FhirVersion::R4,
            #[cfg(feature = "R4B")]
            SofBundle::R4B(_) => FhirVersion::R4B,
            #[cfg(feature = "R5")]
            SofBundle::R5(_) => FhirVersion::R5,
            #[cfg(feature = "R6")]
            SofBundle::R6(_) => FhirVersion::R6,
        }
    }
}

#[derive(Debug, Error)]
pub enum SofError {
    #[error("Invalid ViewDefinition: {0}")]
    InvalidViewDefinition(String),

    #[error("FHIRPath evaluation error: {0}")]
    FhirPathError(String),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("CSV error: {0}")]
    CsvError(#[from] csv::Error),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Unsupported content type: {0}")]
    UnsupportedContentType(String),

    #[error("CSV writer error: {0}")]
    CsvWriterError(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentType {
    Csv,
    CsvWithHeader,
    Json,
    NdJson,
    Parquet,
}

impl ContentType {
    pub fn from_string(s: &str) -> Result<Self, SofError> {
        match s {
            "text/csv" => Ok(ContentType::Csv),
            "text/csv;header=present" => Ok(ContentType::CsvWithHeader),
            "application/json" => Ok(ContentType::Json),
            "application/ndjson" => Ok(ContentType::NdJson),
            "application/parquet" => Ok(ContentType::Parquet),
            _ => Err(SofError::UnsupportedContentType(s.to_string())),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessedRow {
    pub values: Vec<Option<serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessedResult {
    pub columns: Vec<String>,
    pub rows: Vec<ProcessedRow>,
}

pub fn run_view_definition(
    view_definition: SofViewDefinition,
    bundle: SofBundle,
    content_type: ContentType,
) -> Result<Vec<u8>, SofError> {
    let processed_result = process_view_definition(view_definition, bundle)?;
    format_output(processed_result, content_type)
}

fn process_view_definition(
    view_definition: SofViewDefinition,
    bundle: SofBundle,
) -> Result<ProcessedResult, SofError> {
    // Ensure both resources use the same FHIR version
    if view_definition.version() != bundle.version() {
        return Err(SofError::InvalidViewDefinition(
            "ViewDefinition and Bundle must use the same FHIR version".to_string(),
        ));
    }

    match (view_definition, bundle) {
        #[cfg(feature = "R4")]
        (SofViewDefinition::R4(vd), SofBundle::R4(bundle)) => {
            process_view_definition_generic(vd, bundle)
        }
        #[cfg(feature = "R4B")]
        (SofViewDefinition::R4B(vd), SofBundle::R4B(bundle)) => {
            process_view_definition_generic(vd, bundle)
        }
        #[cfg(feature = "R5")]
        (SofViewDefinition::R5(vd), SofBundle::R5(bundle)) => {
            process_view_definition_generic(vd, bundle)
        }
        #[cfg(feature = "R6")]
        (SofViewDefinition::R6(vd), SofBundle::R6(bundle)) => {
            process_view_definition_generic(vd, bundle)
        }
        _ => unreachable!("Version mismatch should have been caught above"),
    }
}

// Generic version-agnostic constant extraction
fn extract_view_definition_constants<VD: ViewDefinitionTrait>(
    view_definition: &VD,
) -> Result<HashMap<String, EvaluationResult>, SofError> {
    let mut variables = HashMap::new();

    if let Some(constants) = view_definition.constants() {
        for constant in constants {
            let name = constant
                .name()
                .ok_or_else(|| {
                    SofError::InvalidViewDefinition("Constant name is required".to_string())
                })?
                .to_string();

            let eval_result = constant.to_evaluation_result()?;
            variables.insert(name, eval_result);
        }
    }

    Ok(variables)
}

// Generic version-agnostic ViewDefinition processing
fn process_view_definition_generic<VD, B>(
    view_definition: VD,
    bundle: B,
) -> Result<ProcessedResult, SofError>
where
    VD: ViewDefinitionTrait,
    B: BundleTrait,
    B::Resource: ResourceTrait,
{
    validate_view_definition(&view_definition)?;

    // Step 1: Extract constants/variables from ViewDefinition
    let variables = extract_view_definition_constants(&view_definition)?;

    // Step 2: Filter resources by type and profile
    let target_resource_type = view_definition.resource().ok_or_else(|| {
        SofError::InvalidViewDefinition("Resource type is required".to_string())
    })?;

    let filtered_resources = filter_resources(&bundle, target_resource_type)?;

    // Step 3: Apply where clauses to filter resources
    let filtered_resources =
        apply_where_clauses(filtered_resources, view_definition.where_clauses(), &variables)?;

    // Step 4: Process all select clauses to generate rows with forEach support
    let select_clauses = view_definition.select().ok_or_else(|| {
        SofError::InvalidViewDefinition("At least one select clause is required".to_string())
    })?;

    // Generate rows for each resource using the forEach-aware approach
    let (all_columns, rows) =
        generate_rows_from_selects(&filtered_resources, select_clauses, &variables)?;

    Ok(ProcessedResult {
        columns: all_columns,
        rows,
    })
}

// Generic version-agnostic validation
fn validate_view_definition<VD: ViewDefinitionTrait>(view_def: &VD) -> Result<(), SofError> {
    // Basic validation
    if view_def.resource().map_or(true, |s| s.is_empty()) {
        return Err(SofError::InvalidViewDefinition(
            "ViewDefinition must specify a resource type".to_string(),
        ));
    }

    if view_def.select().map_or(true, |s| s.is_empty()) {
        return Err(SofError::InvalidViewDefinition(
            "ViewDefinition must have at least one select".to_string(),
        ));
    }

    // Validate where clauses
    if let Some(where_clauses) = view_def.where_clauses() {
        validate_where_clauses(where_clauses)?;
    }

    // Validate selects
    if let Some(selects) = view_def.select() {
        for select in selects {
            validate_select(select)?;
        }
    }

    Ok(())
}

// Generic where clause validation
fn validate_where_clauses<W: ViewDefinitionWhereTrait>(
    where_clauses: &[W],
) -> Result<(), SofError> {
    // Basic validation - just ensure paths are provided
    // Type checking will be done during actual evaluation
    for where_clause in where_clauses {
        if where_clause.path().is_none() {
            return Err(SofError::InvalidViewDefinition(
                "Where clause must have a path specified".to_string()
            ));
        }
    }
    Ok(())
}

// Generic helper - no longer needs to be version-specific
fn can_be_coerced_to_boolean(result: &EvaluationResult) -> bool {
    // Check if the result can be meaningfully used as a boolean in a where clause
    match result {
        // Boolean values are obviously OK
        EvaluationResult::Boolean(_, _) => true,
        
        // Empty is OK (evaluates to false)
        EvaluationResult::Empty => true,
        
        // Collections are OK - they evaluate based on whether they're empty or not
        EvaluationResult::Collection { .. } => true,
        
        // Other types cannot be meaningfully coerced to boolean for where clauses
        // This includes: String, Integer, Decimal, Date, DateTime, Time, Quantity, Object
        _ => false,
    }
}

// Generic select validation
fn validate_select<S: ViewDefinitionSelectTrait>(select: &S) -> Result<(), SofError> {
    validate_select_with_context(select, false)
}

fn validate_select_with_context<S: ViewDefinitionSelectTrait>(
    select: &S,
    in_foreach_context: bool,
) -> Result<(), SofError>
where
    S::Select: ViewDefinitionSelectTrait,
{
    // Determine if we're entering a forEach context at this level
    let entering_foreach = select.for_each().is_some() || select.for_each_or_null().is_some();
    let current_foreach_context = in_foreach_context || entering_foreach;

    // Validate collection attribute with the current forEach context
    if let Some(columns) = select.column() {
        for column in columns {
            if let Some(collection_value) = column.collection() {
                if !collection_value && !current_foreach_context {
                    return Err(SofError::InvalidViewDefinition(
                        "Column 'collection' attribute must be true when specified".to_string(),
                    ));
                }
            }
        }
    }

    // Validate unionAll column consistency
    if let Some(union_selects) = select.union_all() {
        validate_union_all_columns(union_selects)?;
    }

    // Recursively validate nested selects
    if let Some(nested_selects) = select.select() {
        for nested_select in nested_selects {
            validate_select_with_context(nested_select, current_foreach_context)?;
        }
    }

    // Validate unionAll selects with forEach context
    if let Some(union_selects) = select.union_all() {
        for union_select in union_selects {
            validate_select_with_context(union_select, current_foreach_context)?;
        }
    }

    Ok(())
}

// Generic union validation
fn validate_union_all_columns<S: ViewDefinitionSelectTrait>(
    union_selects: &[S],
) -> Result<(), SofError> {
    if union_selects.len() < 2 {
        return Ok(());
    }

    // Get column names and order from first select
    let first_select = &union_selects[0];
    let first_columns = get_column_names(first_select)?;

    // Validate all other selects have the same column names in the same order
    for (index, union_select) in union_selects.iter().enumerate().skip(1) {
        let current_columns = get_column_names(union_select)?;

        if current_columns != first_columns {
            if current_columns.len() != first_columns.len()
                || !current_columns
                    .iter()
                    .all(|name| first_columns.contains(name))
            {
                return Err(SofError::InvalidViewDefinition(format!(
                    "UnionAll branch {} has different column names than first branch",
                    index
                )));
            } else {
                return Err(SofError::InvalidViewDefinition(format!(
                    "UnionAll branch {} has columns in different order than first branch",
                    index
                )));
            }
        }
    }

    Ok(())
}

// Generic column name extraction
fn get_column_names<S: ViewDefinitionSelectTrait>(select: &S) -> Result<Vec<String>, SofError> {
    let mut column_names = Vec::new();

    // Collect direct column names
    if let Some(columns) = select.column() {
        for column in columns {
            if let Some(name) = column.name() {
                column_names.push(name.to_string());
            }
        }
    }

    // If this select has unionAll but no direct columns, get columns from first unionAll branch
    if column_names.is_empty() {
        if let Some(union_selects) = select.union_all() {
            if !union_selects.is_empty() {
                return get_column_names(&union_selects[0]);
            }
        }
    }

    Ok(column_names)
}

// Generic resource filtering
fn filter_resources<'a, B: BundleTrait>(
    bundle: &'a B,
    resource_type: &str,
) -> Result<Vec<&'a B::Resource>, SofError> {
    Ok(bundle
        .entries()
        .into_iter()
        .filter(|resource| resource.resource_name() == resource_type)
        .collect())
}

// Generic where clause application
fn apply_where_clauses<'a, R, W>(
    resources: Vec<&'a R>,
    where_clauses: Option<&[W]>,
    variables: &HashMap<String, EvaluationResult>,
) -> Result<Vec<&'a R>, SofError>
where
    R: ResourceTrait,
    W: ViewDefinitionWhereTrait,
{
    if let Some(wheres) = where_clauses {
        let mut filtered = Vec::new();

        for resource in resources {
            let mut include_resource = true;

            // All where clauses must evaluate to true for the resource to be included
            for where_clause in wheres {
                let fhir_resource = resource.to_fhir_resource();
                let mut context = EvaluationContext::new(vec![fhir_resource]);

                // Add variables to the context
                for (name, value) in variables {
                    context.set_variable_result(name, value.clone());
                }

                let path = where_clause.path().ok_or_else(|| {
                    SofError::InvalidViewDefinition("Where clause path is required".to_string())
                })?;

                match evaluate_expression(path, &context) {
                    Ok(result) => {
                        // Check if the result can be meaningfully used as a boolean
                        if !can_be_coerced_to_boolean(&result) {
                            return Err(SofError::InvalidViewDefinition(format!(
                                "Where clause path '{}' returns type '{}' which cannot be used as a boolean condition. \
                                 Where clauses must return boolean values, collections, or empty results.",
                                path, result.type_name()
                            )));
                        }
                        
                        // Check if result is truthy (non-empty and not false)
                        if !is_truthy(&result) {
                            include_resource = false;
                            break;
                        }
                    }
                    Err(e) => {
                        return Err(SofError::FhirPathError(format!(
                            "Error evaluating where clause '{}': {}",
                            path, e
                        )));
                    }
                }
            }

            if include_resource {
                filtered.push(resource);
            }
        }

        Ok(filtered)
    } else {
        Ok(resources)
    }
}

// Removed generate_rows_per_resource_r4 - replaced with new forEach-aware implementation

// Removed generate_rows_with_for_each_r4 - replaced with new forEach-aware implementation

// Helper functions for FHIRPath result processing
fn is_truthy(result: &EvaluationResult) -> bool {
    match result {
        EvaluationResult::Empty => false,
        EvaluationResult::Boolean(b, _) => *b,
        EvaluationResult::Collection { items, .. } => !items.is_empty(),
        _ => true, // Non-empty, non-false values are truthy
    }
}

fn fhirpath_result_to_json_value_collection(result: EvaluationResult) -> Option<serde_json::Value> {
    match result {
        EvaluationResult::Empty => Some(serde_json::Value::Array(vec![])),
        EvaluationResult::Collection { items, .. } => {
            // Always return array for collection columns, even if empty
            let values: Vec<serde_json::Value> = items
                .into_iter()
                .filter_map(fhirpath_result_to_json_value)
                .collect();
            Some(serde_json::Value::Array(values))
        }
        // For non-collection results in collection columns, wrap in array
        single_result => {
            if let Some(json_val) = fhirpath_result_to_json_value(single_result) {
                Some(serde_json::Value::Array(vec![json_val]))
            } else {
                Some(serde_json::Value::Array(vec![]))
            }
        }
    }
}

fn fhirpath_result_to_json_value(result: EvaluationResult) -> Option<serde_json::Value> {
    match result {
        EvaluationResult::Empty => None,
        EvaluationResult::Boolean(b, _) => Some(serde_json::Value::Bool(b)),
        EvaluationResult::Integer(i, _) => {
            Some(serde_json::Value::Number(serde_json::Number::from(i)))
        }
        EvaluationResult::Decimal(d, _) => {
            // Check if this Decimal represents a whole number
            if d.fract().is_zero() {
                // Convert to integer if no fractional part
                if let Ok(i) = d.to_string().parse::<i64>() {
                    Some(serde_json::Value::Number(serde_json::Number::from(i)))
                } else {
                    // Handle very large numbers as strings
                    Some(serde_json::Value::String(d.to_string()))
                }
            } else {
                // Convert Decimal to a float for fractional numbers
                if let Ok(f) = d.to_string().parse::<f64>() {
                    if let Some(num) = serde_json::Number::from_f64(f) {
                        Some(serde_json::Value::Number(num))
                    } else {
                        Some(serde_json::Value::String(d.to_string()))
                    }
                } else {
                    Some(serde_json::Value::String(d.to_string()))
                }
            }
        }
        EvaluationResult::String(s, _) => Some(serde_json::Value::String(s)),
        EvaluationResult::Date(s, _) => Some(serde_json::Value::String(s)),
        EvaluationResult::DateTime(s, _) => Some(serde_json::Value::String(s)),
        EvaluationResult::Time(s, _) => Some(serde_json::Value::String(s)),
        EvaluationResult::Collection { items, .. } => {
            if items.len() == 1 {
                // Single item collection - unwrap to the item itself
                fhirpath_result_to_json_value(items.into_iter().next().unwrap())
            } else if items.is_empty() {
                None
            } else {
                // Multiple items - convert to array
                let values: Vec<serde_json::Value> = items
                    .into_iter()
                    .filter_map(fhirpath_result_to_json_value)
                    .collect();
                Some(serde_json::Value::Array(values))
            }
        }
        EvaluationResult::Object { map, .. } => {
            let mut json_map = serde_json::Map::new();
            for (k, v) in map {
                if let Some(json_val) = fhirpath_result_to_json_value(v) {
                    json_map.insert(k, json_val);
                }
            }
            Some(serde_json::Value::Object(json_map))
        }
        // Handle other result types as strings
        _ => Some(serde_json::Value::String(format!("{:?}", result))),
    }
}

fn extract_iteration_items(result: EvaluationResult) -> Vec<EvaluationResult> {
    match result {
        EvaluationResult::Collection { items, .. } => items,
        EvaluationResult::Empty => Vec::new(),
        single_item => vec![single_item],
    }
}

// Generic row generation functions

fn generate_rows_from_selects<R, S>(
    resources: &[&R],
    selects: &[S],
    variables: &HashMap<String, EvaluationResult>,
) -> Result<(Vec<String>, Vec<ProcessedRow>), SofError>
where
    R: ResourceTrait,
    S: ViewDefinitionSelectTrait,
    S::Select: ViewDefinitionSelectTrait,
{
    let mut all_columns = Vec::new();
    let mut all_rows = Vec::new();

    // For each resource, generate all possible row combinations
    for resource in resources {
        let resource_rows =
            generate_rows_for_resource(*resource, selects, &mut all_columns, variables)?;
        all_rows.extend(resource_rows);
    }

    Ok((all_columns, all_rows))
}

fn generate_rows_for_resource<R, S>(
    resource: &R,
    selects: &[S],
    all_columns: &mut Vec<String>,
    variables: &HashMap<String, EvaluationResult>,
) -> Result<Vec<ProcessedRow>, SofError>
where
    R: ResourceTrait,
    S: ViewDefinitionSelectTrait,
    S::Select: ViewDefinitionSelectTrait,
{
    let fhir_resource = resource.to_fhir_resource();
    let mut context = EvaluationContext::new(vec![fhir_resource]);

    // Add variables to the context
    for (name, value) in variables {
        context.set_variable_result(name, value.clone());
    }

    // Generate all possible row combinations for this resource
    let row_combinations = generate_row_combinations(&context, selects, all_columns, variables)?;

    Ok(row_combinations)
}

#[derive(Debug, Clone)]
struct RowCombination {
    values: Vec<Option<serde_json::Value>>,
}

fn generate_row_combinations<S>(
    context: &EvaluationContext,
    selects: &[S],
    all_columns: &mut Vec<String>,
    variables: &HashMap<String, EvaluationResult>,
) -> Result<Vec<ProcessedRow>, SofError>
where
    S: ViewDefinitionSelectTrait,
    S::Select: ViewDefinitionSelectTrait,
{
    // First pass: collect all column names to ensure consistent ordering
    collect_all_columns(selects, all_columns)?;

    // Second pass: generate all row combinations
    let mut row_combinations = vec![RowCombination {
        values: vec![None; all_columns.len()],
    }];

    for select in selects {
        row_combinations = expand_select_combinations(
            context,
            select,
            &row_combinations,
            all_columns,
            variables,
        )?;
    }

    // Convert to ProcessedRow format
    Ok(row_combinations
        .into_iter()
        .map(|combo| ProcessedRow {
            values: combo.values,
        })
        .collect())
}

fn collect_all_columns<S>(
    selects: &[S],
    all_columns: &mut Vec<String>,
) -> Result<(), SofError>
where
    S: ViewDefinitionSelectTrait,
{
    for select in selects {
        // Add columns from this select
        if let Some(columns) = select.column() {
            for col in columns {
                if let Some(name) = col.name() {
                    if !all_columns.contains(&name.to_string()) {
                        all_columns.push(name.to_string());
                    }
                }
            }
        }

        // Recursively collect from nested selects
        if let Some(nested_selects) = select.select() {
            collect_all_columns(nested_selects, all_columns)?;
        }

        // Collect from unionAll
        if let Some(union_selects) = select.union_all() {
            collect_all_columns(union_selects, all_columns)?;
        }
    }
    Ok(())
}

fn expand_select_combinations<S>(
    context: &EvaluationContext,
    select: &S,
    existing_combinations: &[RowCombination],
    all_columns: &[String],
    variables: &HashMap<String, EvaluationResult>,
) -> Result<Vec<RowCombination>, SofError>
where
    S: ViewDefinitionSelectTrait,
    S::Select: ViewDefinitionSelectTrait,
{
    // Handle forEach and forEachOrNull
    if let Some(for_each_path) = select.for_each() {
        return expand_for_each_combinations(
            context,
            select,
            existing_combinations,
            all_columns,
            for_each_path,
            false,
            variables,
        );
    }

    if let Some(for_each_or_null_path) = select.for_each_or_null() {
        return expand_for_each_combinations(
            context,
            select,
            existing_combinations,
            all_columns,
            for_each_or_null_path,
            true,
            variables,
        );
    }

    // Handle regular columns (no forEach)
    let mut new_combinations = Vec::new();

    for existing_combo in existing_combinations {
        let mut new_combo = existing_combo.clone();

        // Add values from this select's columns
        if let Some(columns) = select.column() {
            for col in columns {
                if let Some(col_name) = col.name() {
                    if let Some(col_index) = all_columns.iter().position(|name| name == col_name) {
                        let path = col.path().ok_or_else(|| {
                            SofError::InvalidViewDefinition("Column path is required".to_string())
                        })?;

                        match evaluate_expression(path, context) {
                            Ok(result) => {
                                // Check if this column is marked as a collection
                                let is_collection = col.collection().unwrap_or(false);

                                new_combo.values[col_index] = if is_collection {
                                    fhirpath_result_to_json_value_collection(result)
                                } else {
                                    fhirpath_result_to_json_value(result)
                                };
                            }
                            Err(e) => {
                                return Err(SofError::FhirPathError(format!(
                                    "Error evaluating column '{}' with path '{}': {}",
                                    col_name, path, e
                                )));
                            }
                        }
                    }
                }
            }
        }

        new_combinations.push(new_combo);
    }

    // Handle nested selects
    if let Some(nested_selects) = select.select() {
        for nested_select in nested_selects {
            new_combinations = expand_select_combinations(
                context,
                nested_select,
                &new_combinations,
                all_columns,
                variables,
            )?;
        }
    }

    // Handle unionAll
    if let Some(union_selects) = select.union_all() {
        let mut union_combinations = Vec::new();

        // Process each unionAll select independently, using the combinations that already have
        // values from this select's columns and nested selects
        for union_select in union_selects {
            let select_combinations = expand_select_combinations(
                context,
                union_select,
                &new_combinations,
                all_columns,
                variables,
            )?;
            union_combinations.extend(select_combinations);
        }

        // unionAll replaces new_combinations with the union results
        // If no union results, this resource should be filtered out (no rows for this resource)
        new_combinations = union_combinations;
    }

    Ok(new_combinations)
}

fn expand_for_each_combinations<S>(
    context: &EvaluationContext,
    select: &S,
    existing_combinations: &[RowCombination],
    all_columns: &[String],
    for_each_path: &str,
    allow_null: bool,
    variables: &HashMap<String, EvaluationResult>,
) -> Result<Vec<RowCombination>, SofError>
where
    S: ViewDefinitionSelectTrait,
    S::Select: ViewDefinitionSelectTrait,
{
    // Evaluate the forEach expression to get iteration items
    let for_each_result = evaluate_expression(for_each_path, context).map_err(|e| {
        SofError::FhirPathError(format!(
            "Error evaluating forEach expression '{}': {}",
            for_each_path, e
        ))
    })?;

    let iteration_items = extract_iteration_items(for_each_result);

    if iteration_items.is_empty() {
        if allow_null {
            // forEachOrNull: generate null rows
            let mut new_combinations = Vec::new();
            for existing_combo in existing_combinations {
                let mut new_combo = existing_combo.clone();

                // Set column values to null for this forEach scope
                if let Some(columns) = select.column() {
                    for col in columns {
                        if let Some(col_name) = col.name() {
                            if let Some(col_index) =
                                all_columns.iter().position(|name| name == col_name)
                            {
                                new_combo.values[col_index] = None;
                            }
                        }
                    }
                }

                new_combinations.push(new_combo);
            }
            return Ok(new_combinations);
        } else {
            // forEach with empty collection: no rows
            return Ok(Vec::new());
        }
    }

    let mut new_combinations = Vec::new();

    // For each iteration item, create new combinations
    for item in &iteration_items {
        // Create a new context with the iteration item
        let _item_context = create_iteration_context(item, variables);

        for existing_combo in existing_combinations {
            let mut new_combo = existing_combo.clone();

            // Evaluate columns in the context of the iteration item
            if let Some(columns) = select.column() {
                for col in columns {
                    if let Some(col_name) = col.name() {
                        if let Some(col_index) =
                            all_columns.iter().position(|name| name == col_name)
                        {
                            let path = col.path().ok_or_else(|| {
                                SofError::InvalidViewDefinition(
                                    "Column path is required".to_string(),
                                )
                            })?;

                            // Use the iteration item directly for path evaluation
                            let result = if path == "$this" {
                                // Special case: $this refers to the current iteration item
                                item.clone()
                            } else {
                                // Evaluate the path on the iteration item
                                evaluate_path_on_item(path, item, variables)?
                            };

                            // Check if this column is marked as a collection
                            let is_collection = col.collection().unwrap_or(false);

                            new_combo.values[col_index] = if is_collection {
                                fhirpath_result_to_json_value_collection(result)
                            } else {
                                fhirpath_result_to_json_value(result)
                            };
                        }
                    }
                }
            }

            new_combinations.push(new_combo);
        }
    }

    // Handle nested selects with the forEach context
    if let Some(nested_selects) = select.select() {
        let mut final_combinations = Vec::new();

        for item in &iteration_items {
            let item_context = create_iteration_context(item, variables);

            // For each iteration item, we need to start with the combinations that have
            // the correct column values for this forEach scope
            for existing_combo in existing_combinations {
                // Find the combination that corresponds to this iteration item
                // by looking at the values we set for columns in this forEach scope
                let mut base_combo = existing_combo.clone();

                // Update the base combination with column values for this iteration item
                if let Some(columns) = select.column() {
                    for col in columns {
                        if let Some(col_name) = col.name() {
                            if let Some(col_index) =
                                all_columns.iter().position(|name| name == col_name)
                            {
                                let path = col.path().ok_or_else(|| {
                                    SofError::InvalidViewDefinition(
                                        "Column path is required".to_string(),
                                    )
                                })?;

                                let result = if path == "$this" {
                                    item.clone()
                                } else {
                                    evaluate_path_on_item(path, item, variables)?
                                };

                                // Check if this column is marked as a collection
                                let is_collection = col.collection().unwrap_or(false);

                                base_combo.values[col_index] = if is_collection {
                                    fhirpath_result_to_json_value_collection(result)
                                } else {
                                    fhirpath_result_to_json_value(result)
                                };
                            }
                        }
                    }
                }

                // Start with this base combination for nested processing
                let mut item_combinations = vec![base_combo];

                // Process nested selects
                for nested_select in nested_selects {
                    item_combinations = expand_select_combinations(
                        &item_context,
                        nested_select,
                        &item_combinations,
                        all_columns,
                        variables,
                    )?;
                }

                final_combinations.extend(item_combinations);
            }
        }

        new_combinations = final_combinations;
    }

    // Handle unionAll within forEach context
    if let Some(union_selects) = select.union_all() {
        let mut union_combinations = Vec::new();

        for item in &iteration_items {
            let item_context = create_iteration_context(item, variables);

            // For each iteration item, process all unionAll selects
            for existing_combo in existing_combinations {
                let mut base_combo = existing_combo.clone();

                // Update the base combination with column values for this iteration item
                if let Some(columns) = select.column() {
                    for col in columns {
                        if let Some(col_name) = col.name() {
                            if let Some(col_index) =
                                all_columns.iter().position(|name| name == col_name)
                            {
                                let path = col.path().ok_or_else(|| {
                                    SofError::InvalidViewDefinition(
                                        "Column path is required".to_string(),
                                    )
                                })?;

                                let result = if path == "$this" {
                                    item.clone()
                                } else {
                                    evaluate_path_on_item(path, item, variables)?
                                };

                                // Check if this column is marked as a collection
                                let is_collection = col.collection().unwrap_or(false);

                                base_combo.values[col_index] = if is_collection {
                                    fhirpath_result_to_json_value_collection(result)
                                } else {
                                    fhirpath_result_to_json_value(result)
                                };
                            }
                        }
                    }
                }

                // Also evaluate columns from nested selects and add them to base_combo
                if let Some(nested_selects) = select.select() {
                    for nested_select in nested_selects {
                        if let Some(nested_columns) = nested_select.column() {
                            for col in nested_columns {
                                if let Some(col_name) = col.name() {
                                    if let Some(col_index) =
                                        all_columns.iter().position(|name| name == col_name)
                                    {
                                        let path = col.path().ok_or_else(|| {
                                            SofError::InvalidViewDefinition(
                                                "Column path is required".to_string(),
                                            )
                                        })?;

                                        let result = if path == "$this" {
                                            item.clone()
                                        } else {
                                            evaluate_path_on_item(path, item, variables)?
                                        };

                                        // Check if this column is marked as a collection
                                        let is_collection = col.collection().unwrap_or(false);

                                        base_combo.values[col_index] = if is_collection {
                                            fhirpath_result_to_json_value_collection(result)
                                        } else {
                                            fhirpath_result_to_json_value(result)
                                        };
                                    }
                                }
                            }
                        }
                    }
                }

                // Process each unionAll select independently for this iteration item
                for union_select in union_selects {
                    let mut select_combinations = vec![base_combo.clone()];
                    select_combinations = expand_select_combinations(
                        &item_context,
                        union_select,
                        &select_combinations,
                        all_columns,
                        variables,
                    )?;
                    union_combinations.extend(select_combinations);
                }
            }
        }

        // unionAll replaces new_combinations with the union results
        // If no union results, filter out this resource (no rows for this resource)
        new_combinations = union_combinations;
    }

    Ok(new_combinations)
}

// Generic helper functions
fn evaluate_path_on_item(
    path: &str,
    item: &EvaluationResult,
    variables: &HashMap<String, EvaluationResult>,
) -> Result<EvaluationResult, SofError> {
    // Create a temporary context with the iteration item as the root resource
    let mut temp_context = match item {
        EvaluationResult::Object { .. } => {
            // Convert the iteration item to a resource-like structure for FHIRPath evaluation
            // For simplicity, we'll create a basic context where the item is available for evaluation
            let mut context = EvaluationContext::new(vec![]);
            context.this = Some(item.clone());
            context
        }
        _ => EvaluationContext::new(vec![]),
    };

    // Add variables to the temporary context
    for (name, value) in variables {
        temp_context.set_variable_result(name, value.clone());
    }

    // Evaluate the FHIRPath expression in the context of the iteration item
    match evaluate_expression(path, &temp_context) {
        Ok(result) => Ok(result),
        Err(_e) => {
            // If FHIRPath evaluation fails, try simple property access as fallback
            match item {
                EvaluationResult::Object { map, .. } => {
                    if let Some(value) = map.get(path) {
                        Ok(value.clone())
                    } else {
                        Ok(EvaluationResult::Empty)
                    }
                }
                _ => Ok(EvaluationResult::Empty),
            }
        }
    }
}

fn create_iteration_context(
    item: &EvaluationResult,
    variables: &HashMap<String, EvaluationResult>,
) -> EvaluationContext {
    // Create a new context with the iteration item as the root
    let mut context = EvaluationContext::new(vec![]);
    context.this = Some(item.clone());

    // Preserve variables from the parent context
    for (name, value) in variables {
        context.set_variable_result(name, value.clone());
    }

    context
}

fn format_output(result: ProcessedResult, content_type: ContentType) -> Result<Vec<u8>, SofError> {
    match content_type {
        ContentType::Csv | ContentType::CsvWithHeader => {
            format_csv(result, content_type == ContentType::CsvWithHeader)
        }
        ContentType::Json => format_json(result),
        ContentType::NdJson => format_ndjson(result),
        ContentType::Parquet => Err(SofError::UnsupportedContentType(
            "Parquet not yet implemented".to_string(),
        )),
    }
}

fn format_csv(result: ProcessedResult, include_header: bool) -> Result<Vec<u8>, SofError> {
    let mut wtr = csv::Writer::from_writer(vec![]);

    if include_header {
        wtr.write_record(&result.columns)?;
    }

    for row in result.rows {
        let record: Vec<String> = row
            .values
            .iter()
            .map(|v| match v {
                Some(val) => serde_json::to_string(val).unwrap_or_default(),
                None => String::new(),
            })
            .collect();
        wtr.write_record(&record)?;
    }

    wtr.into_inner()
        .map_err(|e| SofError::CsvWriterError(e.to_string()))
}

fn format_json(result: ProcessedResult) -> Result<Vec<u8>, SofError> {
    let mut output = Vec::new();

    for row in result.rows {
        let mut row_obj = serde_json::Map::new();
        for (i, column) in result.columns.iter().enumerate() {
            let value = row
                .values
                .get(i)
                .and_then(|v| v.as_ref())
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            row_obj.insert(column.clone(), value);
        }
        output.push(serde_json::Value::Object(row_obj));
    }

    Ok(serde_json::to_vec_pretty(&output)?)
}

fn format_ndjson(result: ProcessedResult) -> Result<Vec<u8>, SofError> {
    let mut output = Vec::new();

    for row in result.rows {
        let mut row_obj = serde_json::Map::new();
        for (i, column) in result.columns.iter().enumerate() {
            let value = row
                .values
                .get(i)
                .and_then(|v| v.as_ref())
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            row_obj.insert(column.clone(), value);
        }
        let line = serde_json::to_string(&serde_json::Value::Object(row_obj))?;
        output.extend_from_slice(line.as_bytes());
        output.push(b'\n');
    }

    Ok(output)
}
