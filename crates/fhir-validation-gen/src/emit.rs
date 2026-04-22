//! Emit generated validation code from normalized `TypeValidationModel`s.
//!
//! This module is the second half of the validation generator pipeline:
//! - `extract.rs` parses raw FHIR definitions into normalized intermediate models
//! - `emit.rs` turns those models into generated Rust code
//!
//! Responsibilities:
//! - emit generated invariant metadata (`InvariantDef` constants)
//! - emit generated binding metadata (`BindingDef` constants)
//! - emit version-aware `Validatable` impls for each generated Rust type
//! - emit recursive validation traversal for nested datatypes, backbone elements,
//!   choice fields, and contained resources
//! - emit version-aware resource dispatcher methods used by the runtime validator
//!
//! Key design points:
//! - `StructureKind` is used to identify concrete dispatchable resources
//! - `ParentKind` is used to decide recursive structural traversal behavior
//! - binding application is version-aware (`apply_r4_bindings`, `apply_r5_bindings`, ...)
//! - invariant evaluation stays on the shared runtime validator path
//!
//! Output is **sharded** by [`emit_types_to_files`]: `parts/part_*.rs` (types) plus
//! `parts/dispatch.rs` (resource dispatchers), aggregated by a small `all.rs` that
//! uses `include!` so `fhir-validation` can pull everything into one module.
use crate::model::{
    BindingModel, BindingTargetKindModel, FieldModel, InvariantModel, ParentKind, StructureKind,
    TypeValidationModel,
};
use crate::versions::FhirVersion;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// How many `TypeValidationModel`s to emit into each `parts/part_XXX.rs` shard.
///
/// Smaller shards improve IDE navigation and merge diffs; larger shards reduce file count.
const TYPES_PER_SHARD: usize = 36;

/// Emit generated metadata constants for a single normalized type.
///
/// This emits only declarative metadata:
/// - invariant definitions
/// - binding definitions
///
/// Executable recursive traversal is emitted separately by
/// `emit_validatable_impl_for_type`.
pub fn emit_validation_metadata_for_type(ty: &TypeValidationModel, output: &mut String) {
    if ty.invariants.is_empty() && ty.bindings.is_empty() {
        return;
    }

    if !ty.invariants.is_empty() {
        emit_invariants_const(ty, &ty.invariants, output);
        output.push('\n');
    }

    if !ty.bindings.is_empty() {
        emit_bindings_const(ty, &ty.bindings, output);
        output.push('\n');
    }
}

/// Emit the version-specific `Validatable` impl for one generated Rust type.
///
/// The generated impl has two responsibilities:
/// - apply metadata declared directly on `Self`
/// - recurse into child fields that require nested validation
///
/// The emitted binding path is version-aware because typed binding validation is
/// implemented in version-specific runtime modules (`apply_r4_bindings`,
/// `apply_r5_bindings`, etc.).
///
/// Invariant application remains shared at runtime and uses the supplied
/// `FhirPathEvaluator`.
pub fn emit_validatable_impl_for_type(
    version: FhirVersion,
    ty: &TypeValidationModel,
    type_index_by_path: &HashMap<&str, &TypeValidationModel>,
    type_index_by_rust_type: &HashMap<&str, &TypeValidationModel>,
    output: &mut String,
) {
    let trait_name = version.validatable_trait_name();
    let feature_name = version.validation_feature();
    let validation_module_path = validation_trait_module_path(version);

    output.push_str(&format!("#[cfg(feature = {:?})]\n", feature_name));
    output.push_str(&format!(
        "impl {validation_module_path}::{} for {} {{\n",
        trait_name, ty.rust_type
    ));

    output.push_str("    fn validate_bindings(\n");
    output.push_str("        &self,\n");
    output.push_str("        validator: &fhir_validation::Validator,\n");
    output.push_str("        terminology: Option<&dyn fhir_validation::TerminologyServiceSync>,\n");
    output.push_str("    ) -> Vec<fhir_validation::ValidationIssue> {\n");
    output.push_str("        let mut issues = Vec::new();\n");

    if ty.bindings.is_empty() {
        output.push_str("        let _ = (validator, terminology);\n");
    } else {
        let apply_bindings_method = apply_bindings_method_name(version);
        output.push_str(&format!(
            "        issues.extend(validator.{apply_bindings_method}(self, {}.as_slice(), terminology));\n",
            bindings_const_name(ty),
            apply_bindings_method = apply_bindings_method,
        ));
    }

    emit_recursive_validation(
        version,
        ty,
        ValidationPass::Bindings,
        type_index_by_path,
        type_index_by_rust_type,
        output,
    );
    output.push_str("        issues\n");
    output.push_str("    }\n\n");

    output.push_str("    fn validate_invariants(\n");
    output.push_str("        &self,\n");
    output.push_str("        validator: &fhir_validation::Validator,\n");
    output.push_str("        evaluator: &dyn fhir_validation::FhirPathEvaluator,\n");
    output.push_str("    ) -> Vec<fhir_validation::ValidationIssue> {\n");
    output.push_str("        let mut issues = Vec::new();\n");

    if ty.invariants.is_empty() {
        output.push_str("        let _ = (validator, evaluator);\n");
    } else {
        output.push_str(&format!(
            "        issues.extend(validator.apply_invariants(self, {}.as_slice(), evaluator, {:?}));\n",
            invariants_const_name(ty),
            ty.fhir_path
        ));
    }

    emit_recursive_validation(
        version,
        ty,
        ValidationPass::Invariants,
        type_index_by_path,
        type_index_by_rust_type,
        output,
    );

    output.push_str("        issues\n");
    output.push_str("    }\n");
    output.push_str("}\n\n");

    let trait_name_async = version.validatable_trait_name_async();

    output.push_str(&format!("#[cfg(feature = {:?})]\n", feature_name));
    output.push_str("#[async_trait::async_trait]\n");
    output.push_str(&format!(
        "impl {validation_module_path}::{} for {} {{\n",
        trait_name_async, ty.rust_type
    ));

    output.push_str("    async fn validate_bindings_async(\n");
    output.push_str("        &self,\n");
    output.push_str("        validator: &fhir_validation::Validator,\n");
    output.push_str("        terminology: Option<&dyn fhir_validation::TerminologyService>,\n");
    output.push_str("    ) -> Vec<fhir_validation::ValidationIssue> {\n");
    output.push_str("        let mut issues = Vec::new();\n");

    if ty.bindings.is_empty() {
        output.push_str("        let _ = (validator, terminology);\n");
    } else {
        let apply_bindings_method_async = apply_bindings_method_name_async(version);
        output.push_str(&format!(
            "        issues.extend(validator.{apply_bindings_method_async}(self, {}.as_slice(), terminology).await);\n",
            bindings_const_name(ty),
            apply_bindings_method_async = apply_bindings_method_async,
        ));
    }

    emit_recursive_validation(
        version,
        ty,
        ValidationPass::BindingsAsync,
        type_index_by_path,
        type_index_by_rust_type,
        output,
    );
    output.push_str("        issues\n");
    output.push_str("    }\n");
    output.push_str("    }\n\n");
}

/// Emit all generated validation code into `out_dir`:
/// - `all.rs` — `include!` aggregator (included by `fhir-validation` via `include!`)
/// - `parts/part_XXX.rs` — sharded type metadata + `Validatable` impls
/// - `parts/dispatch.rs` — `Validator` resource dispatchers
///
/// `include!` paths in `all.rs` assume the canonical layout
/// `crates/fhir-validation-gen/generated/<r4|r5|…>/` next to `crates/fhir-validation/`.
pub fn emit_types_to_files(
    version: FhirVersion,
    types: &[TypeValidationModel],
    out_dir: &Path,
    input_paths: &[PathBuf],
    structure_definition_count: usize,
) -> Result<(), String> {
    fs::create_dir_all(out_dir).map_err(|e| {
        format!(
            "failed to create output directory '{}': {e}",
            out_dir.display()
        )
    })?;

    let parts_dir = out_dir.join("parts");
    fs::create_dir_all(&parts_dir).map_err(|e| {
        format!(
            "failed to create parts directory '{}': {e}",
            parts_dir.display()
        )
    })?;

    clear_stale_part_files(&parts_dir)?;

    let type_index_by_path: HashMap<&str, &TypeValidationModel> =
        types.iter().map(|ty| (ty.fhir_path.as_str(), ty)).collect();

    let type_index_by_rust_type: HashMap<&str, &TypeValidationModel> =
        types.iter().map(|ty| (ty.rust_type.as_str(), ty)).collect();

    let mut part_idx: usize = 0;
    let mut buffer = String::new();
    let mut count_in_shard: usize = 0;

    for ty in types {
        emit_type(
            version,
            ty,
            &type_index_by_path,
            &type_index_by_rust_type,
            &mut buffer,
        );
        count_in_shard += 1;
        if count_in_shard >= TYPES_PER_SHARD {
            write_shard_file(&parts_dir, part_idx, version, &buffer)?;
            part_idx += 1;
            buffer.clear();
            count_in_shard = 0;
        }
    }

    if !buffer.is_empty() || part_idx == 0 {
        write_shard_file(&parts_dir, part_idx, version, &buffer)?;
        part_idx += 1;
    }

    let mut dispatch = String::new();
    emit_resource_bindings_dispatcher(version, types, &mut dispatch);
    emit_resource_bindings_async_dispatcher(version, types, &mut dispatch);
    emit_resource_invariants_dispatcher(version, types, &mut dispatch);

    let dispatch_path = parts_dir.join("dispatch.rs");
    let mut dispatch_src = String::new();
    dispatch_src.push_str("// @generated by fhir-validation-gen\n");
    dispatch_src.push_str(&format!("// FHIR version: {}\n", version.display_name()));
    dispatch_src.push_str("// Resource dispatchers for `fhir_validation::Validator`.\n\n");
    dispatch_src.push_str(&dispatch);

    fs::write(&dispatch_path, dispatch_src)
        .map_err(|e| format!("failed to write '{}': {e}", dispatch_path.display()))?;

    write_aggregate_all_rs(
        version,
        out_dir,
        input_paths,
        structure_definition_count,
        part_idx,
    )?;

    Ok(())
}

fn clear_stale_part_files(parts_dir: &Path) -> Result<(), String> {
    let Ok(entries) = fs::read_dir(parts_dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if (name.starts_with("part_") && name.ends_with(".rs")) || name == "dispatch.rs" {
            fs::remove_file(entry.path()).map_err(|e| {
                format!(
                    "failed to remove stale shard '{}': {e}",
                    entry.path().display()
                )
            })?;
        }
    }
    Ok(())
}

fn write_shard_file(
    parts_dir: &Path,
    part_index: usize,
    version: FhirVersion,
    body: &str,
) -> Result<(), String> {
    let path = parts_dir.join(format!("part_{part_index:03}.rs"));
    let mut s = String::new();
    s.push_str("// @generated by fhir-validation-gen\n");
    s.push_str(&format!("// FHIR version: {}\n", version.display_name()));
    s.push_str(&format!("// shard parts/part_{part_index:03}.rs\n\n"));
    s.push_str(body);
    fs::write(&path, s).map_err(|e| format!("failed to write '{}': {e}", path.display()))
}

fn write_aggregate_all_rs(
    version: FhirVersion,
    out_dir: &Path,
    input_paths: &[PathBuf],
    structure_definition_count: usize,
    part_count: usize,
) -> Result<(), String> {
    let mut s = String::new();
    s.push_str("// @generated by fhir-validation-gen\n");
    s.push_str(&format!("// FHIR version: {}\n", version.display_name()));
    s.push_str("// Source StructureDefinition bundles:\n");
    for input_path in input_paths {
        s.push_str(&format!("//   - {}\n", input_path.display()));
    }
    s.push_str(&format!(
        "// StructureDefinitions processed: {}\n",
        structure_definition_count
    ));
    s.push_str("//\n");
    s.push_str("// Sharded: `parts/part_*.rs` + `parts/dispatch.rs` are included below.\n");
    s.push_str(
        "// Paths use `CARGO_MANIFEST_DIR` of the `fhir-validation` crate (include! site).\n\n",
    );

    let mod_name = version.module_name();
    for i in 0..part_count {
        s.push_str(&format!(
            "include!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/../fhir-validation-gen/generated/{}/parts/part_{:03}.rs\"));\n",
            mod_name, i
        ));
    }
    s.push_str(&format!(
        "include!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/../fhir-validation-gen/generated/{}/parts/dispatch.rs\"));\n",
        mod_name
    ));

    let path = out_dir.join("all.rs");
    fs::write(&path, s).map_err(|e| format!("failed to write '{}': {e}", path.display()))
}

/// Emit metadata and executable validation code for one normalized type.
pub fn emit_type(
    version: FhirVersion,
    ty: &TypeValidationModel,
    type_index_by_path: &HashMap<&str, &TypeValidationModel>,
    type_index_by_rust_type: &HashMap<&str, &TypeValidationModel>,
    output: &mut String,
) {
    emit_validation_metadata_for_type(ty, output);
    emit_validatable_impl_for_type(
        version,
        ty,
        type_index_by_path,
        type_index_by_rust_type,
        output,
    );
}

/// Return true when a type should participate in top-level resource dispatch.
///
/// This is based on `StructureKind`, not `ParentKind`, because resource
/// dispatcher generation follows FHIR specification category rather than
/// inheritance/runtime structure.
fn is_dispatchable_resource(ty: &TypeValidationModel) -> bool {
    matches!(ty.structure_kind, StructureKind::Resource)
}

/// Emit the version-specific resource dispatcher for binding validation.
///
/// The generated method matches over the versioned `Resource` enum and forwards
/// to each concrete resource type's generated `validate_bindings` impl.
fn emit_resource_bindings_dispatcher(
    version: FhirVersion,
    types: &[TypeValidationModel],
    output: &mut String,
) {
    let resources: Vec<&TypeValidationModel> = types
        .iter()
        .filter(|ty| is_dispatchable_resource(ty))
        .collect();

    if resources.is_empty() {
        return;
    }

    let feature_name = version.validation_feature();
    let trait_name = version.validatable_trait_name();
    let validation_module_path = validation_trait_module_path(version);
    let resource_enum_path = resource_enum_path(version);
    let method_name = contained_dispatch_method_name(version, ValidationPass::Bindings);

    output.push_str(&format!("#[cfg(feature = {:?})]\n", feature_name));
    output.push_str("impl fhir_validation::Validator {\n");
    output.push_str(&format!(
        "    pub fn {method_name}(\n",
        method_name = method_name,
    ));
    output.push_str(&format!(
        "        &self,\n        resource: &{resource_enum_path},\n",
        resource_enum_path = resource_enum_path,
    ));
    output.push_str("        terminology: Option<&dyn fhir_validation::TerminologyServiceSync>,\n");
    output.push_str("    ) -> Vec<fhir_validation::ValidationIssue> {\n");
    output.push_str("        match resource {\n");

    for resource in resources {
        let rust_type = &resource.rust_type;
        output.push_str(&format!(
            "            {resource_enum_path}::{rust_type}(value) => <{rust_type} as {validation_module_path}::{trait_name}>::validate_bindings(value.as_ref(), self, terminology),\n",
            resource_enum_path = resource_enum_path,
            rust_type = rust_type,
            validation_module_path = validation_module_path,
            trait_name = trait_name,
        ));
    }

    output.push_str("        }\n");
    output.push_str("    }\n");
    output.push_str("}\n\n");
}

/// Emit the version-specific resource dispatcher for async binding validation.
///
/// The generated method matches over the versioned `Resource` enum and forwards
/// to each concrete resource type's generated `validate_bindings_async` impl.
fn emit_resource_bindings_async_dispatcher(
    version: FhirVersion,
    types: &[TypeValidationModel],
    output: &mut String,
) {
    let resources: Vec<&TypeValidationModel> = types
        .iter()
        .filter(|ty| is_dispatchable_resource(ty))
        .collect();

    if resources.is_empty() {
        return;
    }

    let feature_name = version.validation_feature();
    let trait_name_async = version.validatable_trait_name_async();
    let validation_module_path = validation_trait_module_path(version);
    let resource_enum_path = resource_enum_path(version);
    let method_name = contained_dispatch_method_name(version, ValidationPass::BindingsAsync);

    output.push_str(&format!("#[cfg(feature = {:?})]\n", feature_name));
    output.push_str("impl fhir_validation::Validator {\n");
    output.push_str(&format!(
        "    pub async fn {method_name}(\n",
        method_name = method_name,
    ));
    output.push_str(&format!(
        "        &self,\n        resource: &{resource_enum_path},\n",
        resource_enum_path = resource_enum_path,
    ));
    output.push_str("        terminology: Option<&dyn fhir_validation::TerminologyService>,\n");
    output.push_str("    ) -> Vec<fhir_validation::ValidationIssue> {\n");
    output.push_str("        match resource {\n");

    for resource in resources {
        let rust_type = &resource.rust_type;
        output.push_str(&format!(
            "            {resource_enum_path}::{rust_type}(value) => <{rust_type} as {validation_module_path}::{trait_name_async}>::validate_bindings_async(value.as_ref(), self, terminology).await,\n",
            resource_enum_path = resource_enum_path,
            rust_type = rust_type,
            validation_module_path = validation_module_path,
            trait_name_async = trait_name_async,
        ));
    }

    output.push_str("        }\n");
    output.push_str("    }\n");
    output.push_str("}\n\n");
}

/// Emit the version-specific resource dispatcher for invariant validation.
///
/// The generated method matches over the versioned `Resource` enum and forwards
/// to each concrete resource type's generated `validate_invariants` impl.
fn emit_resource_invariants_dispatcher(
    version: FhirVersion,
    types: &[TypeValidationModel],
    output: &mut String,
) {
    let resources: Vec<&TypeValidationModel> = types
        .iter()
        .filter(|ty| is_dispatchable_resource(ty))
        .collect();

    if resources.is_empty() {
        return;
    }

    let feature_name = version.validation_feature();
    let trait_name = version.validatable_trait_name();
    let validation_module_path = validation_trait_module_path(version);
    let resource_enum_path = resource_enum_path(version);
    let method_name = contained_dispatch_method_name(version, ValidationPass::Invariants);

    output.push_str(&format!("#[cfg(feature = {:?})]\n", feature_name));
    output.push_str("impl fhir_validation::Validator {\n");
    output.push_str(&format!(
        "    pub fn {method_name}(\n",
        method_name = method_name,
    ));
    output.push_str(&format!(
        "        &self,\n        resource: &{resource_enum_path},\n",
        resource_enum_path = resource_enum_path,
    ));
    output.push_str("        evaluator: &dyn fhir_validation::FhirPathEvaluator,\n");
    output.push_str("    ) -> Vec<fhir_validation::ValidationIssue> {\n");
    output.push_str("        match resource {\n");

    for resource in resources {
        let rust_type = &resource.rust_type;
        output.push_str(&format!(
            "            {resource_enum_path}::{rust_type}(value) => <{rust_type} as {validation_module_path}::{trait_name}>::validate_invariants(value.as_ref(), self, evaluator),\n",
            resource_enum_path = resource_enum_path,
            rust_type = rust_type,
            validation_module_path = validation_module_path,
            trait_name = trait_name,
        ));
    }

    output.push_str("        }\n");
    output.push_str("    }\n");
    output.push_str("}\n\n");
}

/// Return the version-specific runtime binding application method name.
///
/// Bindings are applied in version-aware runtime modules because typed binding
/// validation depends on versioned `Coding` / `CodeableConcept` model types.
fn apply_bindings_method_name(version: FhirVersion) -> &'static str {
    match version {
        FhirVersion::R4 => "apply_r4_bindings",
        FhirVersion::R4B => "apply_r4b_bindings",
        FhirVersion::R5 => "apply_r5_bindings",
        FhirVersion::R6 => "apply_r6_bindings",
    }
}
fn apply_bindings_method_name_async(version: FhirVersion) -> &'static str {
    match version {
        FhirVersion::R4 => "apply_r4_bindings_async",
        FhirVersion::R4B => "apply_r4b_bindings_async",
        FhirVersion::R5 => "apply_r5_bindings_async",
        FhirVersion::R6 => "apply_r6_bindings_async",
    }
}

/// Return the version-specific validation trait module path used in generated impls.
fn validation_trait_module_path(version: FhirVersion) -> &'static str {
    match version {
        FhirVersion::R4 => "fhir_validation::r4",
        FhirVersion::R4B => "fhir_validation::r4b",
        FhirVersion::R5 => "fhir_validation::r5",
        FhirVersion::R6 => "fhir_validation::r6",
    }
}

/// Return the version-specific `Resource` enum path used in generated dispatchers.
fn resource_enum_path(version: FhirVersion) -> &'static str {
    match version {
        FhirVersion::R4 => "helios_fhir::r4::Resource",
        FhirVersion::R4B => "helios_fhir::r4b::Resource",
        FhirVersion::R5 => "helios_fhir::r5::Resource",
        FhirVersion::R6 => "helios_fhir::r6::Resource",
    }
}
fn terminology_module_path(version: FhirVersion) -> &'static str {
    match version {
        FhirVersion::R4 => "helios_fhir::r4::terminology",
        FhirVersion::R4B => "helios_fhir::r4b::terminology",
        FhirVersion::R5 => "helios_fhir::r5::terminology",
        FhirVersion::R6 => "helios_fhir::r6::terminology",
    }
}

/// Which validation phase the emitter is currently generating code for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValidationPass {
    Bindings,
    Invariants,
    BindingsAsync,
}

/// Emit the generated invariant metadata constant for one type.
fn emit_invariants_const(
    ty: &TypeValidationModel,
    invariants: &[InvariantModel],
    output: &mut String,
) {
    let const_name = invariants_const_name(ty);
    output.push_str(&format!(
        "static {const_name}: std::sync::LazyLock<Vec<fhir_validation_types::InvariantDef>> = std::sync::LazyLock::new(|| vec![\n",
        const_name = const_name,
    ));

    for invariant in invariants {
        output.push_str("    fhir_validation_types::InvariantDef {\n");
        output.push_str(&format!("        key: {:?}.to_string(),\n", invariant.key));
        output.push_str(&format!(
            "        severity: {},\n",
            invariant.severity.as_rust_tokens()
        ));
        output.push_str(&format!(
            "        path: {:?}.to_string(),\n",
            invariant.path
        ));
        output.push_str(&format!(
            "        expression: {:?}.to_string(),\n",
            invariant.expression
        ));
        output.push_str(&format!(
            "        human: {:?}.to_string(),\n",
            invariant.human
        ));
        output.push_str("    },\n");
    }

    output.push_str("]);\n");
}

/// Emit the generated binding metadata constant for one type.
fn emit_bindings_const(ty: &TypeValidationModel, bindings: &[BindingModel], output: &mut String) {
    let const_name = bindings_const_name(ty);
    output.push_str(&format!(
        "static {const_name}: std::sync::LazyLock<Vec<fhir_validation_types::BindingDef>> = std::sync::LazyLock::new(|| vec![\n",
        const_name = const_name,
    ));

    for binding in bindings {
        output.push_str("    fhir_validation_types::BindingDef {\n");
        output.push_str(&format!("        path: {:?}.to_string(),\n", binding.path));
        output.push_str(&format!(
            "        strength: {},\n",
            binding.strength.as_rust_tokens()
        ));
        output.push_str(&format!(
            "        value_set: {:?}.to_string(),\n",
            binding.value_set
        ));

        match &binding.binding_name {
            Some(name) => {
                output.push_str(&format!(
                    "        binding_name: Some({:?}.to_string()),\n",
                    name
                ));
            }
            None => output.push_str("        binding_name: None,\n"),
        }

        output.push_str(&format!(
            "        target_kind: {},\n",
            binding.target_kind.as_rust_tokens()
        ));
        if matches!(binding.target_kind, BindingTargetKindModel::Choice) {
            output.push_str("        choice_type_codes: Some(vec![");
            for (i, code) in binding.bindable_type_codes.iter().enumerate() {
                if i > 0 {
                    output.push_str(", ");
                }
                output.push_str(&format!("{:?}.to_string()", code));
            }
            output.push_str("]),\n");
        } else {
            output.push_str("        choice_type_codes: None,\n");
        }
        output.push_str("    },\n");
    }

    output.push_str("]);\n");
}

/// Emit recursive validation code for child fields under one generated type.
///
/// This function separates:
/// - executable recursive candidates that can be emitted directly
/// - deferred candidates that need specialized handling, such as choice fields
///   or contained resources
///
/// The same traversal strategy is reused for both bindings and invariants.
fn emit_recursive_validation(
    version: FhirVersion,
    ty: &TypeValidationModel,
    pass: ValidationPass,
    type_index_by_path: &HashMap<&str, &TypeValidationModel>,
    type_index_by_rust_type: &HashMap<&str, &TypeValidationModel>,
    output: &mut String,
) {
    let executable_candidates: Vec<&FieldModel> = ty
        .fields
        .iter()
        .filter(|field| {
            should_emit_executable_recursion(
                pass,
                ty.parent_kind,
                field,
                type_index_by_path,
                type_index_by_rust_type,
            )
        })
        .collect();

    let deferred_candidates: Vec<&FieldModel> = ty
        .fields
        .iter()
        .filter(|field| {
            should_emit_deferred_todo(
                pass,
                ty.parent_kind,
                field,
                type_index_by_path,
                type_index_by_rust_type,
            )
        })
        .collect();

    if executable_candidates.is_empty() && deferred_candidates.is_empty() {
        return;
    }

    output.push('\n');

    if matches!(pass, ValidationPass::Invariants) {
        for field in ty.fields.iter().filter(|field| field.is_array) {
            emit_empty_array_check(&ty.fhir_path, field, output);
        }

        if !ty.fields.is_empty() {
            output.push('\n');
        }
    }

    for field in executable_candidates {
        emit_field_recursive_validation(&ty.fhir_path, version, field, pass, output);
    }

    if !deferred_candidates.is_empty() {
        output.push('\n');
        output.push_str("        // Deferred recursive validation candidates.\n");
        output.push_str("        // These need specialized handling (for example choice enums or contained resources).\n");

        for field in deferred_candidates {
            if field.rust_field_name == "contained" {
                emit_contained_field_recursive_validation(
                    &ty.fhir_path,
                    version,
                    field,
                    pass,
                    output,
                );
                continue;
            }
            if field.is_choice {
                emit_choice_field_recursive_validation(
                    version,
                    &ty.fhir_path,
                    field,
                    pass,
                    &ty.bindings,
                    type_index_by_path,
                    type_index_by_rust_type,
                    output,
                );
                continue;
            }

            let field_name = emitted_field_name(field);
            let pass_name = match pass {
                ValidationPass::Bindings => "bindings",
                ValidationPass::Invariants => "invariants",
                ValidationPass::BindingsAsync => "bindings_async",
            };

            let cardinality = if field.is_array {
                "repeating"
            } else {
                "single"
            };
            let choice = if field.is_choice { ", choice[x]" } else { "" };
            let enum_name = field
                .choice_enum_name
                .as_deref()
                .map(|name| format!(", enum={name}"))
                .unwrap_or_default();

            output.push_str(&format!(
            "        // TODO({pass_name}): recurse into self.{field_name} // path={path}, type_codes={type_codes:?}, {cardinality}{choice}{enum_name}\n",
            field_name = field_name,
            path = field.fhir_path,
            type_codes = field.type_codes,
            cardinality = cardinality,
            choice = choice,
            enum_name = enum_name,
        ));
        }
    }
}

/// Emit a structural validation check that rejects present-but-empty arrays.
///
/// In FHIR JSON, repeating elements should be omitted when they have no values
/// rather than being serialized as `[]`. This helper emits a single structural
/// issue during the invariants pass for any array field that is present but
/// empty.
fn emit_empty_array_check(current_type_path: &str, field: &FieldModel, output: &mut String) {
    let field_name = emitted_field_name(field);
    let rebase_path = local_rebase_path(current_type_path, field);

    output.push_str(&format!(
        "        if let Some(values) = &self.{field_name} {{\n",
        field_name = field_name,
    ));
    output.push_str("            if values.is_empty() {\n");
    output.push_str(&format!(
        "                issues.push(fhir_validation::ValidationIssue::error(\"structure\", {:?}, \"Array cannot be empty - the property should not be present if it has no values\").with_instance_path({:?}));\n",
        field.fhir_path,
        rebase_path,
    ));
    output.push_str("            }\n");
    output.push_str("        }\n");
}

/// Emit recursive validation for a FHIR choice field.
///
/// Choice fields require special handling because the generated Rust model uses
/// an enum rather than a single concrete child type path. This function emits a
/// match over the generated choice enum and forwards validation to the selected
/// variant when validator metadata exists for that child type.
#[allow(clippy::too_many_arguments)]
fn emit_choice_field_recursive_validation(
    version: FhirVersion,
    current_type_path: &str,
    field: &FieldModel,
    pass: ValidationPass,
    current_bindings: &[BindingModel],
    type_index_by_path: &HashMap<&str, &TypeValidationModel>,
    type_index_by_rust_type: &HashMap<&str, &TypeValidationModel>,
    output: &mut String,
) {
    let Some(enum_name) = field.choice_enum_name.as_deref() else {
        emit_choice_todo(field, pass, "missing choice enum name", output);
        return;
    };

    let field_name = emitted_field_name(field);
    let rebase_path = local_rebase_path(current_type_path, field);
    let validation_module_path = validation_trait_module_path(version);
    let terminology_module_path = terminology_module_path(version);

    output.push_str(&format!(
        "        if let Some(choice) = &self.{field_name} {{\n",
        field_name = field_name,
    ));
    output.push_str("            match choice {\n");

    let direct_choice_binding = current_bindings
        .iter()
        .find(|binding| binding.path == field.fhir_path && binding.is_choice_binding);

    for type_code in &field.type_codes {
        let variant_name = choice_variant_name_from_type_code(type_code);
        let child_path = child_type_path_for_choice_variant(type_code);
        let child_rust_type = child_rust_type_for_choice_variant(type_code);
        let child_model = child_path
            .as_deref()
            .and_then(|path| type_index_by_path.get(path).copied())
            .or_else(|| {
                child_rust_type
                    .as_deref()
                    .and_then(|name| type_index_by_rust_type.get(name).copied())
            });

        match (pass, child_model) {
            (ValidationPass::Bindings, Some(model)) => {
                let handled_by_field_binding = direct_choice_binding
                    .map(|binding| {
                        binding
                            .bindable_type_codes
                            .iter()
                            .any(|bindable| bindable == type_code)
                    })
                    .unwrap_or(false);

                if handled_by_field_binding
                    && is_parent_bound_choice_complex_binding_type_code(type_code)
                {
                    if let Some(binding) = direct_choice_binding {
                        let helper_name = complex_choice_binding_helper_name(pass, type_code);
                        let local_validator_name = complex_choice_local_validator_name(type_code);
                        output.push_str(&format!(
                            "                {enum_name}::{variant_name}(value) => {{\n"
                        ));
                        output.push_str(&format!(
                            "                    let binding_ctx = fhir_validation::binding::common::BindingCheckContextSync::new(validator, {fhir_path:?}, {value_set:?}, {strength}, terminology);\n",
                            fhir_path = binding.path,
                            value_set = binding.value_set,
                            strength = binding.strength.as_rust_tokens(),
                        ));
                        output.push_str(&format!(
                            "                    let child_issues = {validation_module_path}::{helper_name}(&binding_ctx, Some(value), |value| {terminology_module_path}::{local_validator_name}({value_set:?}, value));\n",
                            validation_module_path = validation_module_path,
                            helper_name = helper_name,
                            value_set = binding.value_set,
                            terminology_module_path = terminology_module_path,
                            local_validator_name = local_validator_name,
                        ));
                        output.push_str("                    issues.extend(child_issues);\n");
                        output.push_str("                }\n");
                    } else {
                        output.push_str(&format!(
                            "                {enum_name}::{variant_name}(value) => {{\n"
                        ));
                        output.push_str(
                            "                    let child_issues = value.validate_bindings(validator, terminology);\n",
                        );
                        output.push_str(&format!(
                            "                    issues.extend(validator.rebase_instance_paths(child_issues, {:?}));\n",
                            rebase_path,
                        ));
                        output.push_str("                }\n");
                    }
                } else if !model.bindings.is_empty() {
                    output.push_str(&format!(
                        "                {enum_name}::{variant_name}(value) => {{\n"
                    ));
                    output.push_str(
                        "                    let child_issues = value.validate_bindings(validator, terminology);\n",
                    );
                    output.push_str(&format!(
                        "                    issues.extend(validator.rebase_instance_paths(child_issues, {:?}));\n",
                        rebase_path,
                    ));
                    output.push_str("                }\n");
                } else {
                    output.push_str(&format!(
                        "                {enum_name}::{variant_name}(_value) => {{\n"
                    ));
                    output.push_str("                }\n");
                }
            }
            (ValidationPass::Bindings, _) => {
                let handled_by_field_binding = direct_choice_binding
                    .map(|binding| {
                        binding
                            .bindable_type_codes
                            .iter()
                            .any(|bindable| bindable == type_code)
                    })
                    .unwrap_or(false);

                if handled_by_field_binding && is_parent_bound_choice_primitive_type_code(type_code)
                {
                    if let Some(binding) = direct_choice_binding {
                        let helper_name = primitive_choice_binding_helper_name(pass, type_code);
                        let local_validator_name = primitive_choice_local_validator_name(type_code);
                        output.push_str(&format!(
                            "                {enum_name}::{variant_name}(value) => {{\n"
                        ));
                        output.push_str(&format!(
                            "                    let binding_ctx = fhir_validation::binding::common::BindingCheckContextSync::new(validator, {fhir_path:?}, {value_set:?}, {strength}, terminology);\n",
                            fhir_path = binding.path,
                            value_set = binding.value_set,
                            strength = binding.strength.as_rust_tokens(),
                        ));
                        if type_code == "code" {
                            output.push_str(&format!(
                                "                    let child_issues = {validation_module_path}::{helper_name}(&binding_ctx, value.value.as_deref(), Some({terminology_module_path}::implicit_system({value_set:?})), |value| {terminology_module_path}::{local_validator_name}({value_set:?}, value));\n",
                                validation_module_path = validation_module_path,
                                helper_name = helper_name,
                                value_set = binding.value_set,
                                terminology_module_path = terminology_module_path,
                                local_validator_name = local_validator_name,
                            ));
                        } else {
                            output.push_str(&format!(
                                "                    let child_issues = {validation_module_path}::{helper_name}(&binding_ctx, value.value.as_deref(), |value| {terminology_module_path}::{local_validator_name}({value_set:?}, value));\n",
                                validation_module_path = validation_module_path,
                                helper_name = helper_name,
                                value_set = binding.value_set,
                                terminology_module_path = terminology_module_path,
                                local_validator_name = local_validator_name,
                            ));
                        }
                        output.push_str("                    issues.extend(child_issues);\n");
                        output.push_str("                }\n");
                    } else {
                        output.push_str(&format!(
                            "                {enum_name}::{variant_name}(_value) => {{\n"
                        ));
                        output.push_str("                }\n");
                    }
                } else {
                    output.push_str(&format!(
                        "                {enum_name}::{variant_name}(_value) => {{\n"
                    ));
                    output.push_str("                }\n");
                }
            }
            (ValidationPass::BindingsAsync, Some(model)) => {
                let handled_by_field_binding = direct_choice_binding
                    .map(|binding| {
                        binding
                            .bindable_type_codes
                            .iter()
                            .any(|bindable| bindable == type_code)
                    })
                    .unwrap_or(false);

                if handled_by_field_binding
                    && is_parent_bound_choice_complex_binding_type_code(type_code)
                {
                    if let Some(binding) = direct_choice_binding {
                        let helper_name = complex_choice_binding_helper_name(pass, type_code);
                        let local_validator_name = complex_choice_local_validator_name(type_code);
                        output.push_str(&format!(
                            "                {enum_name}::{variant_name}(value) => {{\n"
                        ));
                        output.push_str(&format!(
                            "                    let binding_ctx = fhir_validation::binding::common::BindingCheckContextAsync::new(validator, {fhir_path:?}, {value_set:?}, {strength}, terminology);\n",
                            fhir_path = binding.path,
                            value_set = binding.value_set,
                            strength = binding.strength.as_rust_tokens(),
                        ));
                        output.push_str(&format!(
                            "                    let child_issues = {validation_module_path}::{helper_name}(&binding_ctx, Some(value), |value| {terminology_module_path}::{local_validator_name}({value_set:?}, value)).await;\n",
                            validation_module_path = validation_module_path,
                            helper_name = helper_name,
                            value_set = binding.value_set,
                            terminology_module_path = terminology_module_path,
                            local_validator_name = local_validator_name,
                        ));
                        output.push_str("                    issues.extend(child_issues);\n");
                        output.push_str("                }\n");
                    } else {
                        output.push_str(&format!(
                            "                {enum_name}::{variant_name}(value) => {{\n"
                        ));
                        output.push_str(
                            "                    let child_issues = value.validate_bindings_async(validator, terminology).await;\n",
                        );
                        output.push_str(&format!(
                            "                    issues.extend(validator.rebase_instance_paths(child_issues, {:?}));\n",
                            rebase_path,
                        ));
                        output.push_str("                }\n");
                    }
                } else if !model.bindings.is_empty() {
                    output.push_str(&format!(
                        "                {enum_name}::{variant_name}(value) => {{\n"
                    ));
                    output.push_str(
                        "                    let child_issues = value.validate_bindings_async(validator, terminology).await;\n",
                    );
                    output.push_str(&format!(
                        "                    issues.extend(validator.rebase_instance_paths(child_issues, {:?}));\n",
                        rebase_path,
                    ));
                    output.push_str("                }\n");
                } else {
                    output.push_str(&format!(
                        "                {enum_name}::{variant_name}(_value) => {{\n"
                    ));
                    output.push_str("                }\n");
                }
            }
            (ValidationPass::BindingsAsync, _) => {
                let handled_by_field_binding = direct_choice_binding
                    .map(|binding| {
                        binding
                            .bindable_type_codes
                            .iter()
                            .any(|bindable| bindable == type_code)
                    })
                    .unwrap_or(false);

                if handled_by_field_binding && is_parent_bound_choice_primitive_type_code(type_code)
                {
                    if let Some(binding) = direct_choice_binding {
                        let helper_name = primitive_choice_binding_helper_name(pass, type_code);
                        let local_validator_name = primitive_choice_local_validator_name(type_code);
                        output.push_str(&format!(
                            "                {enum_name}::{variant_name}(value) => {{\n"
                        ));
                        output.push_str(&format!(
                            "                    let binding_ctx = fhir_validation::binding::common::BindingCheckContextAsync::new(validator, {fhir_path:?}, {value_set:?}, {strength}, terminology);\n",
                            fhir_path = binding.path,
                            value_set = binding.value_set,
                            strength = binding.strength.as_rust_tokens(),
                        ));
                        if type_code == "code" {
                            output.push_str(&format!(
                                "                    let child_issues = {validation_module_path}::{helper_name}(&binding_ctx, value.value.as_deref(), Some({terminology_module_path}::implicit_system({value_set:?})), |value| {terminology_module_path}::{local_validator_name}({value_set:?}, value)).await;\n",
                                validation_module_path = validation_module_path,
                                helper_name = helper_name,
                                value_set = binding.value_set,
                                terminology_module_path = terminology_module_path,
                                local_validator_name = local_validator_name,
                            ));
                        } else {
                            output.push_str(&format!(
                                "                    let child_issues = {validation_module_path}::{helper_name}(&binding_ctx, value.value.as_deref(), |value| {terminology_module_path}::{local_validator_name}({value_set:?}, value)).await;\n",
                                validation_module_path = validation_module_path,
                                helper_name = helper_name,
                                value_set = binding.value_set,
                                terminology_module_path = terminology_module_path,
                                local_validator_name = local_validator_name,
                            ));
                        }
                        output.push_str("                    issues.extend(child_issues);\n");
                        output.push_str("                }\n");
                    } else {
                        output.push_str(&format!(
                            "                {enum_name}::{variant_name}(_value) => {{\n"
                        ));
                        output.push_str("                }\n");
                    }
                } else {
                    output.push_str(&format!(
                        "                {enum_name}::{variant_name}(_value) => {{\n"
                    ));
                    output.push_str("                }\n");
                }
            }
            (ValidationPass::Invariants, Some(model)) if !model.invariants.is_empty() => {
                output.push_str(&format!(
                    "                {enum_name}::{variant_name}(value) => {{\n"
                ));
                output.push_str(
                    "                    let child_issues = value.validate_invariants(validator, evaluator);\n",
                );
                output.push_str(&format!(
                    "                    issues.extend(validator.rebase_instance_paths(child_issues, {:?}));\n",
                    rebase_path,
                ));
                output.push_str("                }\n");
            }
            (ValidationPass::Invariants, _) => {
                output.push_str(&format!(
                    "                {enum_name}::{variant_name}(_value) => {{\n"
                ));
                output.push_str("                }\n");
            }
        }
    }

    output.push_str("            }\n");
    output.push_str("        }\n");
}

/// Emit a TODO comment for a choice field that could not yet be emitted
/// executable recursion for.
fn emit_choice_todo(field: &FieldModel, pass: ValidationPass, reason: &str, output: &mut String) {
    let field_name = emitted_field_name(field);

    output.push_str(&format!(
        "        // TODO({pass_name}): recurse into self.{field_name} // path={path}, type_codes={type_codes:?}, single, choice[x], reason={reason}\n",
        pass_name = validation_pass_name(pass),
        field_name = field_name,
        path = field.fhir_path,
        type_codes = field.type_codes,
        reason = reason,
    ));
}

/// Return a human-readable name for the current validation pass.
fn validation_pass_name(pass: ValidationPass) -> &'static str {
    match pass {
        ValidationPass::Bindings => "bindings",
        ValidationPass::Invariants => "invariants",
        ValidationPass::BindingsAsync => "bindings_async",
    }
}

/// Convert a FHIR type code into the corresponding generated choice enum variant name.
fn choice_variant_name_from_type_code(type_code: &str) -> String {
    capitalize_first_letter(&normalize_choice_variant_base(type_code))
}

/// Normalize a FHIR type code into the base Rust naming form used for choice variants.
fn normalize_choice_variant_base(type_code: &str) -> String {
    match type_code {
        "base64Binary" => "base64Binary".to_string(),
        "boolean" => "boolean".to_string(),
        "canonical" => "canonical".to_string(),
        "code" => "code".to_string(),
        "date" => "date".to_string(),
        "dateTime" => "dateTime".to_string(),
        "decimal" => "decimal".to_string(),
        "id" => "id".to_string(),
        "instant" => "instant".to_string(),
        "integer" => "integer".to_string(),
        "integer64" => "integer64".to_string(),
        "markdown" => "markdown".to_string(),
        "oid" => "oid".to_string(),
        "positiveInt" => "positiveInt".to_string(),
        "string" => "string".to_string(),
        "time" => "time".to_string(),
        "unsignedInt" => "unsignedInt".to_string(),
        "uri" => "uri".to_string(),
        "url" => "url".to_string(),
        "uuid" => "uuid".to_string(),
        other => other.to_string(),
    }
}

/// Return the generated child type path corresponding to a choice variant type code.
fn child_type_path_for_choice_variant(type_code: &str) -> Option<String> {
    Some(choice_variant_name_from_type_code(type_code))
}
fn is_parent_bound_choice_primitive_type_code(type_code: &str) -> bool {
    matches!(type_code, "code" | "string" | "uri")
}

fn is_parent_bound_choice_complex_binding_type_code(type_code: &str) -> bool {
    matches!(
        type_code,
        "Coding" | "CodeableConcept" | "Quantity" | "CodeableReference"
    )
}

fn primitive_choice_binding_helper_name(pass: ValidationPass, type_code: &str) -> &'static str {
    match (pass, type_code) {
        (ValidationPass::Bindings, "code") => "validate_primitive_code_binding",
        (ValidationPass::BindingsAsync, "code") => "validate_primitive_code_binding_async",
        (ValidationPass::Bindings, "string") | (ValidationPass::Bindings, "uri") => {
            "validate_primitive_value_binding"
        }
        (ValidationPass::BindingsAsync, "string") | (ValidationPass::BindingsAsync, "uri") => {
            "validate_primitive_value_binding_async"
        }
        _ => unreachable!("unsupported primitive choice binding type: {type_code}"),
    }
}

fn primitive_choice_local_validator_name(type_code: &str) -> &'static str {
    match type_code {
        "code" | "string" | "uri" => "validate_code",
        _ => unreachable!("unsupported primitive choice local validator type: {type_code}"),
    }
}

fn complex_choice_binding_helper_name(pass: ValidationPass, type_code: &str) -> &'static str {
    match (pass, type_code) {
        (ValidationPass::Bindings, "Coding") => "validate_coding_binding",
        (ValidationPass::BindingsAsync, "Coding") => "validate_coding_binding_async",
        (ValidationPass::Bindings, "CodeableConcept") => "validate_codeable_concept_binding",
        (ValidationPass::BindingsAsync, "CodeableConcept") => {
            "validate_codeable_concept_binding_async"
        }
        (ValidationPass::Bindings, "CodeableReference") => "validate_codeable_reference_binding",
        (ValidationPass::BindingsAsync, "CodeableReference") => {
            "validate_codeable_reference_binding_async"
        }
        (ValidationPass::Bindings, "Quantity") => "validate_quantity_binding",
        (ValidationPass::BindingsAsync, "Quantity") => "validate_quantity_binding_async",
        _ => unreachable!("unsupported complex choice binding type: {type_code}"),
    }
}

fn complex_choice_local_validator_name(type_code: &str) -> &'static str {
    match type_code {
        "Coding" => "validate_coding",
        "CodeableConcept" => "validate_codeable_concept",
        "Quantity" => "validate_quantity",
        _ => unreachable!("unsupported complex choice local validator type: {type_code}"),
    }
}

/// Capitalize the first character of a string for generated Rust type/variant names.
fn capitalize_first_letter(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Return the child rebase path to use inside the currently generated type.
///
/// Top-level generated resource/datatype models use the full `field.fhir_path`
/// (for example `Patient.identifier` or `HumanName.use`).
///
/// Nested generated helper types such as `Patient.contact` must rebase child
/// issues relative to a helper-local root (for example `contact.relationship`),
/// otherwise parent rebasing will either duplicate path segments or lose the
/// child field name entirely.
fn local_rebase_path(current_type_path: &str, field: &FieldModel) -> String {
    if let Some(stripped) = field
        .fhir_path
        .strip_prefix(&format!("{current_type_path}."))
    {
        if let Some((_, local_root)) = current_type_path.rsplit_once('.') {
            return format!("{local_root}.{stripped}");
        }
    }

    field.fhir_path.clone()
}

/// Emit direct recursive validation for a normal child field.
///
/// This handles:
/// - optional vs required fields
/// - repeating vs scalar fields
/// - pass-specific rebasing for invariant issues
fn emit_field_recursive_validation(
    current_type_path: &str,
    version: FhirVersion,
    field: &FieldModel,
    pass: ValidationPass,
    output: &mut String,
) {
    let field_name = emitted_field_name(field);
    let rebase_path = local_rebase_path(current_type_path, field);

    if field.type_codes.len() == 1
        && field.type_codes[0] == "Resource"
        && !field.is_array
        && !field.is_choice
    {
        let dispatch_method = contained_dispatch_method_name(version, pass);
        let arg_name = contained_dispatch_arg_name(pass);

        if field.is_required {
            match pass {
                ValidationPass::BindingsAsync => {
                    output.push_str(&format!(
                        "        let child_issues = validator.{dispatch_method}(&self.{field_name}, {arg_name}).await;\n",
                        dispatch_method = dispatch_method,
                        field_name = field_name,
                        arg_name = arg_name,
                    ));
                }
                _ => {
                    output.push_str(&format!(
                        "        let child_issues = validator.{dispatch_method}(&self.{field_name}, {arg_name});\n",
                        dispatch_method = dispatch_method,
                        field_name = field_name,
                        arg_name = arg_name,
                    ));
                }
            }
            output.push_str(&format!(
                "        issues.extend(validator.rebase_instance_paths(child_issues, {:?}));\n",
                rebase_path,
            ));
        } else {
            output.push_str(&format!(
                "        if let Some(value) = &self.{field_name} {{\n",
                field_name = field_name,
            ));
            match pass {
                ValidationPass::BindingsAsync => {
                    output.push_str(&format!(
                        "            let child_issues = validator.{dispatch_method}(value, {arg_name}).await;\n",
                        dispatch_method = dispatch_method,
                        arg_name = arg_name,
                    ));
                }
                _ => {
                    output.push_str(&format!(
                        "            let child_issues = validator.{dispatch_method}(value, {arg_name});\n",
                        dispatch_method = dispatch_method,
                        arg_name = arg_name,
                    ));
                }
            }
            output.push_str(&format!(
                "            issues.extend(validator.rebase_instance_paths(child_issues, {:?}));\n",
                rebase_path,
            ));
            output.push_str("        }\n");
        }
        return;
    }
    match pass {
        ValidationPass::Bindings => {
            if field.is_array {
                output.push_str(&format!(
                    "        if let Some(values) = &self.{field_name} {{\n",
                    field_name = field_name,
                ));
                output.push_str("            for (idx, value) in values.iter().enumerate() {\n");
                output.push_str("                let child_issues = value.validate_bindings(validator, terminology);\n");
                output.push_str(&format!(
                    "                issues.extend(validator.rebase_instance_paths(child_issues, &format!(\"{}[{{idx}}]\")));\n",
                    rebase_path,
                ));
                output.push_str("            }\n");
                output.push_str("        }\n");
            } else if field.is_required {
                output.push_str(&format!(
                    "        let child_issues = self.{field_name}.validate_bindings(validator, terminology);\n",
                    field_name = field_name,
                ));
                output.push_str(&format!(
                    "        issues.extend(validator.rebase_instance_paths(child_issues, {:?}));\n",
                    rebase_path,
                ));
            } else {
                output.push_str(&format!(
                    "        if let Some(value) = &self.{field_name} {{\n",
                    field_name = field_name,
                ));
                output.push_str("            let child_issues = value.validate_bindings(validator, terminology);\n");
                output.push_str(&format!(
                    "            issues.extend(validator.rebase_instance_paths(child_issues, {:?}));\n",
                    rebase_path,
                ));
                output.push_str("        }\n");
            }
        }
        ValidationPass::BindingsAsync => {
            if field.is_array {
                output.push_str(&format!(
                    "        if let Some(values) = &self.{field_name} {{\n",
                    field_name = field_name,
                ));
                output.push_str("            for (idx, value) in values.iter().enumerate() {\n");
                output.push_str("                let child_issues = value.validate_bindings_async(validator, terminology).await;\n");
                output.push_str(&format!(
                    "                issues.extend(validator.rebase_instance_paths(child_issues, &format!(\"{}[{{idx}}]\")));\n",
                    rebase_path,
                ));
                output.push_str("            }\n");
                output.push_str("        }\n");
            } else if field.is_required {
                output.push_str(&format!(
                    "        let child_issues = self.{field_name}.validate_bindings_async(validator, terminology).await;\n",
                    field_name = field_name,
                ));
                output.push_str(&format!(
                    "        issues.extend(validator.rebase_instance_paths(child_issues, {:?}));\n",
                    rebase_path,
                ));
            } else {
                output.push_str(&format!(
                    "        if let Some(value) = &self.{field_name} {{\n",
                    field_name = field_name,
                ));
                output.push_str("            let child_issues = value.validate_bindings_async(validator, terminology).await;\n");
                output.push_str(&format!(
                    "            issues.extend(validator.rebase_instance_paths(child_issues, {:?}));\n",
                    rebase_path,
                ));
                output.push_str("        }\n");
            }
        }
        ValidationPass::Invariants => {
            if field.is_array {
                output.push_str(&format!(
                    "        if let Some(values) = &self.{field_name} {{\n",
                    field_name = field_name,
                ));
                output.push_str("            for (idx, value) in values.iter().enumerate() {\n");
                output.push_str("                let child_issues = value.validate_invariants(validator, evaluator);\n");
                output.push_str(&format!(
                    "                issues.extend(validator.rebase_instance_paths(child_issues, &format!(\"{}[{{idx}}]\")));\n",
                    rebase_path,
                ));
                output.push_str("            }\n");
                output.push_str("        }\n");
            } else if field.is_required {
                output.push_str(&format!(
                    "        let child_issues = self.{field_name}.validate_invariants(validator, evaluator);\n",
                    field_name = field_name,
                ));
                output.push_str(&format!(
                    "        issues.extend(validator.rebase_instance_paths(child_issues, {:?}));\n",
                    rebase_path,
                ));
            } else {
                output.push_str(&format!(
                    "        if let Some(value) = &self.{field_name} {{\n",
                    field_name = field_name,
                ));
                output.push_str("            let child_issues = value.validate_invariants(validator, evaluator);\n");
                output.push_str(&format!(
                    "            issues.extend(validator.rebase_instance_paths(child_issues, {:?}));\n",
                    rebase_path,
                ));
                output.push_str("        }\n");
            }
        }
    }
}

/// Decide whether executable recursive validation should be emitted directly
/// for this field in the current validation pass.
fn should_emit_executable_recursion(
    pass: ValidationPass,
    parent_kind: ParentKind,
    field: &FieldModel,
    type_index_by_path: &HashMap<&str, &TypeValidationModel>,
    type_index_by_rust_type: &HashMap<&str, &TypeValidationModel>,
) -> bool {
    if !should_recurse_into_field(parent_kind, field) {
        return false;
    }

    if field.is_choice {
        return false;
    }

    if field.rust_field_name == "contained" {
        return false;
    }
    if field.type_codes.len() == 1
        && field.type_codes[0] == "Resource"
        && !field.is_array
        && !field.is_choice
    {
        return true;
    }
    let child_model = resolve_child_model(field, type_index_by_path, type_index_by_rust_type);
    let Some(child_model) = child_model else {
        return false;
    };

    match pass {
        ValidationPass::Bindings => !child_model.bindings.is_empty(),
        ValidationPass::Invariants => !child_model.invariants.is_empty(),
        ValidationPass::BindingsAsync => !child_model.bindings.is_empty(),
    }
}

/// Decide whether this field should be emitted as a deferred TODO rather than
/// executable recursive validation.
fn should_emit_deferred_todo(
    pass: ValidationPass,
    parent_kind: ParentKind,
    field: &FieldModel,
    type_index_by_path: &HashMap<&str, &TypeValidationModel>,
    type_index_by_rust_type: &HashMap<&str, &TypeValidationModel>,
) -> bool {
    if !should_recurse_into_field(parent_kind, field) {
        return false;
    }

    if should_emit_executable_recursion(
        pass,
        parent_kind,
        field,
        type_index_by_path,
        type_index_by_rust_type,
    ) {
        return false;
    }

    if field.rust_field_name == "contained" {
        return true;
    }

    if field.is_choice {
        return true;
    }

    let resolved_child = resolve_child_model(field, type_index_by_path, type_index_by_rust_type);
    let Some(_child_model) = resolved_child else {
        return true;
    };

    false
}

/// Resolve the normalized child validation model for a field, if one exists.
///
/// Resolution first tries the full FHIR path and then falls back to a single
/// declared type code's generated Rust type name.
fn resolve_child_model<'a>(
    field: &FieldModel,
    type_index_by_path: &'a HashMap<&str, &'a TypeValidationModel>,
    type_index_by_rust_type: &'a HashMap<&str, &'a TypeValidationModel>,
) -> Option<&'a TypeValidationModel> {
    if let Some(model) = type_index_by_path.get(field.fhir_path.as_str()).copied() {
        return Some(model);
    }

    if field.type_codes.len() == 1 {
        let rust_type_name = rust_type_name_from_type_code(&field.type_codes[0]);
        if let Some(model) = type_index_by_rust_type
            .get(rust_type_name.as_str())
            .copied()
        {
            return Some(model);
        }
    }

    None
}

/// Convert a FHIR type code into the generated Rust type name.
fn rust_type_name_from_type_code(type_code: &str) -> String {
    capitalize_first_letter(type_code)
}

/// Return the generated Rust type name corresponding to a choice variant type code.
fn child_rust_type_for_choice_variant(type_code: &str) -> Option<String> {
    Some(rust_type_name_from_type_code(type_code))
}

/// Decide whether a field is structurally eligible for recursive validation.
///
/// This uses `ParentKind` because recursion is about inheritance/runtime
/// behavior (`DomainResource`, `BackboneElement`, `Element`), not specification
/// category.
fn should_recurse_into_field(parent_kind: ParentKind, field: &FieldModel) -> bool {
    if field.type_codes.is_empty() {
        return false;
    }

    if field.is_choice {
        return true;
    }

    for code in &field.type_codes {
        if is_recursive_type_code(code) {
            return true;
        }
    }

    matches!(
        parent_kind,
        ParentKind::Resource | ParentKind::DomainResource | ParentKind::BackboneElement
    ) && !field
        .type_codes
        .iter()
        .all(|code| is_primitive_type_code(code))
}

/// Return true when the supplied FHIR type code represents a recursively
/// validated complex/runtime type.
fn is_recursive_type_code(code: &str) -> bool {
    matches!(
        code,
        "Address"
            | "Attachment"
            | "CodeableConcept"
            | "Coding"
            | "ContactPoint"
            | "HumanName"
            | "Identifier"
            | "Meta"
            | "Money"
            | "Period"
            | "Quantity"
            | "Range"
            | "Ratio"
            | "Reference"
            | "SampledData"
            | "Signature"
            | "Timing"
            | "Dosage"
            | "Annotation"
            | "Narrative"
            | "Extension"
            | "Element"
            | "BackboneElement"
            | "Resource"
            | "DomainResource"
    )
}

/// Return true when the supplied FHIR type code is a primitive FHIR type.
fn is_primitive_type_code(code: &str) -> bool {
    matches!(
        code,
        "base64Binary"
            | "boolean"
            | "canonical"
            | "code"
            | "date"
            | "dateTime"
            | "decimal"
            | "id"
            | "instant"
            | "integer"
            | "integer64"
            | "markdown"
            | "oid"
            | "positiveInt"
            | "string"
            | "time"
            | "unsignedInt"
            | "uri"
            | "url"
            | "uuid"
            | "xhtml"
    )
}

/// Emit recursive validation for contained resources.
///
/// Contained resources are validated through the version-specific resource
/// dispatchers because they are held as versioned `Resource` enum values rather
/// than normal nested datatype structs.
fn emit_contained_field_recursive_validation(
    current_type_path: &str,
    version: FhirVersion,
    field: &FieldModel,
    pass: ValidationPass,
    output: &mut String,
) {
    let dispatch_method = contained_dispatch_method_name(version, pass);
    let field_name = emitted_field_name(field);
    let rebase_path = local_rebase_path(current_type_path, field);

    output.push_str(&format!(
        "        if let Some(values) = &self.{field_name} {{\n",
        field_name = field_name,
    ));
    output.push_str("            for (idx, value) in values.iter().enumerate() {\n");
    match pass {
        ValidationPass::BindingsAsync => {
            output.push_str(&format!(
                "                let child_issues = validator.{dispatch_method}(value, {arg_name}).await;\n",
                dispatch_method = dispatch_method,
                arg_name = contained_dispatch_arg_name(pass),
            ));
        }
        _ => {
            output.push_str(&format!(
                "                let child_issues = validator.{dispatch_method}(value, {arg_name});\n",
                dispatch_method = dispatch_method,
                arg_name = contained_dispatch_arg_name(pass),
            ));
        }
    }
    output.push_str(&format!(
        "                issues.extend(validator.rebase_instance_paths(child_issues, &format!(\"{}[{{idx}}]\")));\n",
        rebase_path,
    ));
    output.push_str("            }\n");
    output.push_str("        }\n");
}

/// Return the emitted Rust field identifier for a field, applying raw-identifier
/// escaping when required.
fn emitted_field_name(field: &FieldModel) -> String {
    raw_ident(&field.rust_field_name)
}

/// Escape Rust keywords so generated field access remains valid Rust syntax.
fn raw_ident(name: &str) -> String {
    match name {
        "type" | "match" | "ref" | "loop" | "self" | "super" | "crate" | "mod" | "move"
        | "async" | "await" | "dyn" | "use" | "for" | "where" => format!("r#{name}"),
        _ => name.to_string(),
    }
}

/// Return the version- and pass-specific contained-resource dispatcher method name.
fn contained_dispatch_method_name(version: FhirVersion, pass: ValidationPass) -> &'static str {
    match (version, pass) {
        (FhirVersion::R4, ValidationPass::Bindings) => "validate_r4_resource_bindings",
        (FhirVersion::R4, ValidationPass::Invariants) => "validate_r4_resource_invariants",
        (FhirVersion::R4B, ValidationPass::Bindings) => "validate_r4b_resource_bindings",
        (FhirVersion::R4B, ValidationPass::Invariants) => "validate_r4b_resource_invariants",
        (FhirVersion::R5, ValidationPass::Bindings) => "validate_r5_resource_bindings",
        (FhirVersion::R5, ValidationPass::Invariants) => "validate_r5_resource_invariants",
        (FhirVersion::R6, ValidationPass::Bindings) => "validate_r6_resource_bindings",
        (FhirVersion::R6, ValidationPass::Invariants) => "validate_r6_resource_invariants",
        (FhirVersion::R4, ValidationPass::BindingsAsync) => "validate_r4_resource_bindings_async",
        (FhirVersion::R4B, ValidationPass::BindingsAsync) => "validate_r4b_resource_bindings_async",
        (FhirVersion::R5, ValidationPass::BindingsAsync) => "validate_r5_resource_bindings_async",
        (FhirVersion::R6, ValidationPass::BindingsAsync) => "validate_r6_resource_bindings_async",
    }
}

/// Return the runtime argument name passed to contained-resource dispatch for
/// the current validation pass.
fn contained_dispatch_arg_name(pass: ValidationPass) -> &'static str {
    match pass {
        ValidationPass::Bindings => "terminology",
        ValidationPass::Invariants => "evaluator",
        ValidationPass::BindingsAsync => "terminology",
    }
}

/// Return the generated binding constant name for a type.
fn bindings_const_name(ty: &TypeValidationModel) -> String {
    format!("{}_BINDINGS", upper_snake_case(&ty.rust_type))
}

/// Return the generated invariant constant name for a type.
fn invariants_const_name(ty: &TypeValidationModel) -> String {
    format!("{}_INVARIANTS", upper_snake_case(&ty.rust_type))
}

/// Convert a Rust type name into the generated constant-name form.
fn upper_snake_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 8);
    let mut prev_is_lower_or_digit = false;

    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            if ch.is_ascii_uppercase() {
                if prev_is_lower_or_digit && !out.is_empty() {
                    out.push('_');
                }
                out.push(ch);
                prev_is_lower_or_digit = false;
            } else if ch.is_ascii_lowercase() {
                out.push(ch.to_ascii_uppercase());
                prev_is_lower_or_digit = true;
            } else {
                out.push(ch);
                prev_is_lower_or_digit = true;
            }
        } else {
            if !out.ends_with('_') && !out.is_empty() {
                out.push('_');
            }
            prev_is_lower_or_digit = false;
        }
    }

    out.trim_matches('_').to_string()
}

/// Return the argument type for contained dispatchers for each validation pass.
#[allow(dead_code)]
fn contained_dispatch_arg_type(pass: ValidationPass) -> &'static str {
    match pass {
        ValidationPass::Bindings => "Option<&dyn fhir_validation::TerminologyServiceSync>",
        ValidationPass::BindingsAsync => "Option<&dyn fhir_validation::TerminologyService>",
        ValidationPass::Invariants => "&dyn fhir_validation::FhirPathEvaluator",
    }
}
