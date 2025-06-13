use fhir::FhirResource;
use fhirpath::EvaluationResult;
use crate::SofError;

/// Trait for abstracting ViewDefinition across FHIR versions
pub trait ViewDefinitionTrait {
    type Select: ViewDefinitionSelectTrait;
    type Where: ViewDefinitionWhereTrait;
    type Constant: ViewDefinitionConstantTrait;
    
    fn resource(&self) -> Option<&str>;
    fn select(&self) -> Option<&[Self::Select]>;
    fn where_clauses(&self) -> Option<&[Self::Where]>;
    fn constants(&self) -> Option<&[Self::Constant]>;
}

/// Trait for ViewDefinitionSelect abstraction
pub trait ViewDefinitionSelectTrait {
    type Column: ViewDefinitionColumnTrait;
    type Select: ViewDefinitionSelectTrait;
    
    fn column(&self) -> Option<&[Self::Column]>;
    fn select(&self) -> Option<&[Self::Select]>;
    fn for_each(&self) -> Option<&str>;
    fn for_each_or_null(&self) -> Option<&str>;
    fn union_all(&self) -> Option<&[Self::Select]>;
}

/// Trait for ViewDefinitionColumn abstraction
pub trait ViewDefinitionColumnTrait {
    fn name(&self) -> Option<&str>;
    fn path(&self) -> Option<&str>;
    fn collection(&self) -> Option<bool>;
}

/// Trait for ViewDefinitionWhere abstraction
pub trait ViewDefinitionWhereTrait {
    fn path(&self) -> Option<&str>;
}

/// Trait for ViewDefinitionConstant abstraction
pub trait ViewDefinitionConstantTrait {
    fn name(&self) -> Option<&str>;
    fn to_evaluation_result(&self) -> Result<EvaluationResult, SofError>;
}

/// Trait for Bundle abstraction
pub trait BundleTrait {
    type Resource: ResourceTrait;
    
    fn entries(&self) -> Vec<&Self::Resource>;
}

/// Trait for Resource abstraction
pub trait ResourceTrait: Clone {
    fn resource_name(&self) -> &str;
    fn to_fhir_resource(&self) -> FhirResource;
}

// R4 Implementations
#[cfg(feature = "R4")]
mod r4_impl {
    use super::*;
    use fhir::r4::*;
    
    impl ViewDefinitionTrait for ViewDefinition {
        type Select = ViewDefinitionSelect;
        type Where = ViewDefinitionWhere;
        type Constant = ViewDefinitionConstant;
        
        fn resource(&self) -> Option<&str> {
            self.resource.value.as_deref()
        }
        
        fn select(&self) -> Option<&[Self::Select]> {
            self.select.as_deref()
        }
        
        fn where_clauses(&self) -> Option<&[Self::Where]> {
            self.r#where.as_deref()
        }
        
        fn constants(&self) -> Option<&[Self::Constant]> {
            self.constant.as_deref()
        }
    }
    
    impl ViewDefinitionSelectTrait for ViewDefinitionSelect {
        type Column = ViewDefinitionSelectColumn;
        type Select = ViewDefinitionSelect;
        
        fn column(&self) -> Option<&[Self::Column]> {
            self.column.as_deref()
        }
        
        fn select(&self) -> Option<&[Self::Select]> {
            self.select.as_deref()
        }
        
        fn for_each(&self) -> Option<&str> {
            self.for_each.as_ref()?.value.as_deref()
        }
        
        fn for_each_or_null(&self) -> Option<&str> {
            self.for_each_or_null.as_ref()?.value.as_deref()
        }
        
        fn union_all(&self) -> Option<&[Self::Select]> {
            self.union_all.as_deref()
        }
    }
    
    impl ViewDefinitionColumnTrait for ViewDefinitionSelectColumn {
        fn name(&self) -> Option<&str> {
            self.name.value.as_deref()
        }
        
        fn path(&self) -> Option<&str> {
            self.path.value.as_deref()
        }
        
        fn collection(&self) -> Option<bool> {
            self.collection.as_ref()?.value
        }
    }
    
    impl ViewDefinitionWhereTrait for ViewDefinitionWhere {
        fn path(&self) -> Option<&str> {
            self.path.value.as_deref()
        }
    }
    
    impl ViewDefinitionConstantTrait for ViewDefinitionConstant {
        fn name(&self) -> Option<&str> {
            self.name.value.as_deref()
        }
        
        fn to_evaluation_result(&self) -> Result<EvaluationResult, SofError> {
            let name = self.name().unwrap_or("unknown");
            
            if let Some(value) = &self.value {
                let eval_result = match value {
                    ViewDefinitionConstantValue::String(s) => {
                        EvaluationResult::String(s.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Boolean(b) => {
                        EvaluationResult::Boolean(b.value.unwrap_or(false), None)
                    }
                    ViewDefinitionConstantValue::Integer(i) => {
                        EvaluationResult::Integer(i.value.unwrap_or(0) as i64, None)
                    }
                    ViewDefinitionConstantValue::Decimal(d) => {
                        if let Some(precise_decimal) = &d.value {
                            match precise_decimal.original_string().parse() {
                                Ok(decimal_value) => EvaluationResult::Decimal(decimal_value, None),
                                Err(_) => {
                                    return Err(SofError::InvalidViewDefinition(format!(
                                        "Invalid decimal value for constant '{}'",
                                        name
                                    )));
                                }
                            }
                        } else {
                            EvaluationResult::Decimal("0".parse().unwrap(), None)
                        }
                    }
                    ViewDefinitionConstantValue::Date(d) => {
                        EvaluationResult::Date(d.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::DateTime(dt) => {
                        EvaluationResult::DateTime(dt.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Time(t) => {
                        EvaluationResult::Time(t.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Code(c) => {
                        EvaluationResult::String(c.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Base64Binary(b) => {
                        EvaluationResult::String(b.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Id(i) => {
                        EvaluationResult::String(i.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Instant(i) => {
                        EvaluationResult::DateTime(i.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Oid(o) => {
                        EvaluationResult::String(o.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::PositiveInt(p) => {
                        EvaluationResult::Integer(p.value.unwrap_or(1) as i64, None)
                    }
                    ViewDefinitionConstantValue::UnsignedInt(u) => {
                        EvaluationResult::Integer(u.value.unwrap_or(0) as i64, None)
                    }
                    ViewDefinitionConstantValue::Uri(u) => {
                        EvaluationResult::String(u.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Url(u) => {
                        EvaluationResult::String(u.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Uuid(u) => {
                        EvaluationResult::String(u.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Canonical(c) => {
                        EvaluationResult::String(c.value.clone().unwrap_or_default(), None)
                    }
                };
                
                Ok(eval_result)
            } else {
                Err(SofError::InvalidViewDefinition(format!(
                    "Constant '{}' must have a value",
                    name
                )))
            }
        }
    }
    
    impl BundleTrait for Bundle {
        type Resource = Resource;
        
        fn entries(&self) -> Vec<&Self::Resource> {
            self.entry
                .as_ref()
                .map(|entries| {
                    entries
                        .iter()
                        .filter_map(|e| e.resource.as_ref())
                        .collect()
                })
                .unwrap_or_default()
        }
    }
    
    impl ResourceTrait for Resource {
        fn resource_name(&self) -> &str {
            self.resource_name()
        }
        
        fn to_fhir_resource(&self) -> FhirResource {
            FhirResource::R4(Box::new(self.clone()))
        }
    }
}

// R4B Implementations
#[cfg(feature = "R4B")]
mod r4b_impl {
    use super::*;
    use fhir::r4b::*;
    
    impl ViewDefinitionTrait for ViewDefinition {
        type Select = ViewDefinitionSelect;
        type Where = ViewDefinitionWhere;
        type Constant = ViewDefinitionConstant;
        
        fn resource(&self) -> Option<&str> {
            self.resource.value.as_deref()
        }
        
        fn select(&self) -> Option<&[Self::Select]> {
            self.select.as_deref()
        }
        
        fn where_clauses(&self) -> Option<&[Self::Where]> {
            self.r#where.as_deref()
        }
        
        fn constants(&self) -> Option<&[Self::Constant]> {
            self.constant.as_deref()
        }
    }
    
    impl ViewDefinitionSelectTrait for ViewDefinitionSelect {
        type Column = ViewDefinitionSelectColumn;
        type Select = ViewDefinitionSelect;
        
        fn column(&self) -> Option<&[Self::Column]> {
            self.column.as_deref()
        }
        
        fn select(&self) -> Option<&[Self::Select]> {
            self.select.as_deref()
        }
        
        fn for_each(&self) -> Option<&str> {
            self.for_each.as_ref()?.value.as_deref()
        }
        
        fn for_each_or_null(&self) -> Option<&str> {
            self.for_each_or_null.as_ref()?.value.as_deref()
        }
        
        fn union_all(&self) -> Option<&[Self::Select]> {
            self.union_all.as_deref()
        }
    }
    
    impl ViewDefinitionColumnTrait for ViewDefinitionSelectColumn {
        fn name(&self) -> Option<&str> {
            self.name.value.as_deref()
        }
        
        fn path(&self) -> Option<&str> {
            self.path.value.as_deref()
        }
        
        fn collection(&self) -> Option<bool> {
            self.collection.as_ref()?.value
        }
    }
    
    impl ViewDefinitionWhereTrait for ViewDefinitionWhere {
        fn path(&self) -> Option<&str> {
            self.path.value.as_deref()
        }
    }
    
    impl ViewDefinitionConstantTrait for ViewDefinitionConstant {
        fn name(&self) -> Option<&str> {
            self.name.value.as_deref()
        }
        
        fn to_evaluation_result(&self) -> Result<EvaluationResult, SofError> {
            let name = self.name().unwrap_or("unknown");
            
            if let Some(value) = &self.value {
                let eval_result = match value {
                    ViewDefinitionConstantValue::String(s) => {
                        EvaluationResult::String(s.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Boolean(b) => {
                        EvaluationResult::Boolean(b.value.unwrap_or(false), None)
                    }
                    ViewDefinitionConstantValue::Integer(i) => {
                        EvaluationResult::Integer(i.value.unwrap_or(0) as i64, None)
                    }
                    ViewDefinitionConstantValue::Decimal(d) => {
                        if let Some(precise_decimal) = &d.value {
                            match precise_decimal.original_string().parse() {
                                Ok(decimal_value) => EvaluationResult::Decimal(decimal_value, None),
                                Err(_) => {
                                    return Err(SofError::InvalidViewDefinition(format!(
                                        "Invalid decimal value for constant '{}'",
                                        name
                                    )));
                                }
                            }
                        } else {
                            EvaluationResult::Decimal("0".parse().unwrap(), None)
                        }
                    }
                    ViewDefinitionConstantValue::Date(d) => {
                        EvaluationResult::Date(d.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::DateTime(dt) => {
                        EvaluationResult::DateTime(dt.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Time(t) => {
                        EvaluationResult::Time(t.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Code(c) => {
                        EvaluationResult::String(c.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Base64Binary(b) => {
                        EvaluationResult::String(b.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Id(i) => {
                        EvaluationResult::String(i.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Instant(i) => {
                        EvaluationResult::DateTime(i.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Oid(o) => {
                        EvaluationResult::String(o.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::PositiveInt(p) => {
                        EvaluationResult::Integer(p.value.unwrap_or(1) as i64, None)
                    }
                    ViewDefinitionConstantValue::UnsignedInt(u) => {
                        EvaluationResult::Integer(u.value.unwrap_or(0) as i64, None)
                    }
                    ViewDefinitionConstantValue::Uri(u) => {
                        EvaluationResult::String(u.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Url(u) => {
                        EvaluationResult::String(u.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Uuid(u) => {
                        EvaluationResult::String(u.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Canonical(c) => {
                        EvaluationResult::String(c.value.clone().unwrap_or_default(), None)
                    }
                };
                
                Ok(eval_result)
            } else {
                Err(SofError::InvalidViewDefinition(format!(
                    "Constant '{}' must have a value",
                    name
                )))
            }
        }
    }
    
    impl BundleTrait for Bundle {
        type Resource = Resource;
        
        fn entries(&self) -> Vec<&Self::Resource> {
            self.entry
                .as_ref()
                .map(|entries| {
                    entries
                        .iter()
                        .filter_map(|e| e.resource.as_ref())
                        .collect()
                })
                .unwrap_or_default()
        }
    }
    
    impl ResourceTrait for Resource {
        fn resource_name(&self) -> &str {
            self.resource_name()
        }
        
        fn to_fhir_resource(&self) -> FhirResource {
            FhirResource::R4B(Box::new(self.clone()))
        }
    }
}

// R5 Implementations
#[cfg(feature = "R5")]
mod r5_impl {
    use super::*;
    use fhir::r5::*;
    
    impl ViewDefinitionTrait for ViewDefinition {
        type Select = ViewDefinitionSelect;
        type Where = ViewDefinitionWhere;
        type Constant = ViewDefinitionConstant;
        
        fn resource(&self) -> Option<&str> {
            self.resource.value.as_deref()
        }
        
        fn select(&self) -> Option<&[Self::Select]> {
            self.select.as_deref()
        }
        
        fn where_clauses(&self) -> Option<&[Self::Where]> {
            self.r#where.as_deref()
        }
        
        fn constants(&self) -> Option<&[Self::Constant]> {
            self.constant.as_deref()
        }
    }
    
    impl ViewDefinitionSelectTrait for ViewDefinitionSelect {
        type Column = ViewDefinitionSelectColumn;
        type Select = ViewDefinitionSelect;
        
        fn column(&self) -> Option<&[Self::Column]> {
            self.column.as_deref()
        }
        
        fn select(&self) -> Option<&[Self::Select]> {
            self.select.as_deref()
        }
        
        fn for_each(&self) -> Option<&str> {
            self.for_each.as_ref()?.value.as_deref()
        }
        
        fn for_each_or_null(&self) -> Option<&str> {
            self.for_each_or_null.as_ref()?.value.as_deref()
        }
        
        fn union_all(&self) -> Option<&[Self::Select]> {
            self.union_all.as_deref()
        }
    }
    
    impl ViewDefinitionColumnTrait for ViewDefinitionSelectColumn {
        fn name(&self) -> Option<&str> {
            self.name.value.as_deref()
        }
        
        fn path(&self) -> Option<&str> {
            self.path.value.as_deref()
        }
        
        fn collection(&self) -> Option<bool> {
            self.collection.as_ref()?.value
        }
    }
    
    impl ViewDefinitionWhereTrait for ViewDefinitionWhere {
        fn path(&self) -> Option<&str> {
            self.path.value.as_deref()
        }
    }
    
    impl ViewDefinitionConstantTrait for ViewDefinitionConstant {
        fn name(&self) -> Option<&str> {
            self.name.value.as_deref()
        }
        
        fn to_evaluation_result(&self) -> Result<EvaluationResult, SofError> {
            let name = self.name().unwrap_or("unknown");
            
            if let Some(value) = &self.value {
                // R5 implementation identical to R4
                let eval_result = match value {
                    ViewDefinitionConstantValue::String(s) => {
                        EvaluationResult::String(s.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Boolean(b) => {
                        EvaluationResult::Boolean(b.value.unwrap_or(false), None)
                    }
                    ViewDefinitionConstantValue::Integer(i) => {
                        EvaluationResult::Integer(i.value.unwrap_or(0) as i64, None)
                    }
                    ViewDefinitionConstantValue::Decimal(d) => {
                        if let Some(precise_decimal) = &d.value {
                            match precise_decimal.original_string().parse() {
                                Ok(decimal_value) => EvaluationResult::Decimal(decimal_value, None),
                                Err(_) => {
                                    return Err(SofError::InvalidViewDefinition(format!(
                                        "Invalid decimal value for constant '{}'",
                                        name
                                    )));
                                }
                            }
                        } else {
                            EvaluationResult::Decimal("0".parse().unwrap(), None)
                        }
                    }
                    ViewDefinitionConstantValue::Date(d) => {
                        EvaluationResult::Date(d.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::DateTime(dt) => {
                        EvaluationResult::DateTime(dt.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Time(t) => {
                        EvaluationResult::Time(t.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Code(c) => {
                        EvaluationResult::String(c.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Base64Binary(b) => {
                        EvaluationResult::String(b.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Id(i) => {
                        EvaluationResult::String(i.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Instant(i) => {
                        EvaluationResult::DateTime(i.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Oid(o) => {
                        EvaluationResult::String(o.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::PositiveInt(p) => {
                        EvaluationResult::Integer(p.value.unwrap_or(1) as i64, None)
                    }
                    ViewDefinitionConstantValue::UnsignedInt(u) => {
                        EvaluationResult::Integer(u.value.unwrap_or(0) as i64, None)
                    }
                    ViewDefinitionConstantValue::Uri(u) => {
                        EvaluationResult::String(u.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Url(u) => {
                        EvaluationResult::String(u.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Uuid(u) => {
                        EvaluationResult::String(u.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Canonical(c) => {
                        EvaluationResult::String(c.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Integer64(i) => {
                        EvaluationResult::Integer64(i.value.unwrap_or(0), None)
                    }
                };
                
                Ok(eval_result)
            } else {
                Err(SofError::InvalidViewDefinition(format!(
                    "Constant '{}' must have a value",
                    name
                )))
            }
        }
    }
    
    impl BundleTrait for Bundle {
        type Resource = Resource;
        
        fn entries(&self) -> Vec<&Self::Resource> {
            self.entry
                .as_ref()
                .map(|entries| {
                    entries
                        .iter()
                        .filter_map(|e| e.resource.as_deref())  // Note: R5 uses Box<Resource>
                        .collect()
                })
                .unwrap_or_default()
        }
    }
    
    impl ResourceTrait for Resource {
        fn resource_name(&self) -> &str {
            self.resource_name()
        }
        
        fn to_fhir_resource(&self) -> FhirResource {
            FhirResource::R5(Box::new(self.clone()))
        }
    }
}

// R6 Implementations
#[cfg(feature = "R6")]
mod r6_impl {
    use super::*;
    use fhir::r6::*;
    
    impl ViewDefinitionTrait for ViewDefinition {
        type Select = ViewDefinitionSelect;
        type Where = ViewDefinitionWhere;
        type Constant = ViewDefinitionConstant;
        
        fn resource(&self) -> Option<&str> {
            self.resource.value.as_deref()
        }
        
        fn select(&self) -> Option<&[Self::Select]> {
            self.select.as_deref()
        }
        
        fn where_clauses(&self) -> Option<&[Self::Where]> {
            self.r#where.as_deref()
        }
        
        fn constants(&self) -> Option<&[Self::Constant]> {
            self.constant.as_deref()
        }
    }
    
    impl ViewDefinitionSelectTrait for ViewDefinitionSelect {
        type Column = ViewDefinitionSelectColumn;
        type Select = ViewDefinitionSelect;
        
        fn column(&self) -> Option<&[Self::Column]> {
            self.column.as_deref()
        }
        
        fn select(&self) -> Option<&[Self::Select]> {
            self.select.as_deref()
        }
        
        fn for_each(&self) -> Option<&str> {
            self.for_each.as_ref()?.value.as_deref()
        }
        
        fn for_each_or_null(&self) -> Option<&str> {
            self.for_each_or_null.as_ref()?.value.as_deref()
        }
        
        fn union_all(&self) -> Option<&[Self::Select]> {
            self.union_all.as_deref()
        }
    }
    
    impl ViewDefinitionColumnTrait for ViewDefinitionSelectColumn {
        fn name(&self) -> Option<&str> {
            self.name.value.as_deref()
        }
        
        fn path(&self) -> Option<&str> {
            self.path.value.as_deref()
        }
        
        fn collection(&self) -> Option<bool> {
            self.collection.as_ref()?.value
        }
    }
    
    impl ViewDefinitionWhereTrait for ViewDefinitionWhere {
        fn path(&self) -> Option<&str> {
            self.path.value.as_deref()
        }
    }
    
    impl ViewDefinitionConstantTrait for ViewDefinitionConstant {
        fn name(&self) -> Option<&str> {
            self.name.value.as_deref()
        }
        
        fn to_evaluation_result(&self) -> Result<EvaluationResult, SofError> {
            let name = self.name().unwrap_or("unknown");
            
            if let Some(value) = &self.value {
                // R5 implementation identical to R4
                let eval_result = match value {
                    ViewDefinitionConstantValue::String(s) => {
                        EvaluationResult::String(s.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Boolean(b) => {
                        EvaluationResult::Boolean(b.value.unwrap_or(false), None)
                    }
                    ViewDefinitionConstantValue::Integer(i) => {
                        EvaluationResult::Integer(i.value.unwrap_or(0) as i64, None)
                    }
                    ViewDefinitionConstantValue::Decimal(d) => {
                        if let Some(precise_decimal) = &d.value {
                            match precise_decimal.original_string().parse() {
                                Ok(decimal_value) => EvaluationResult::Decimal(decimal_value, None),
                                Err(_) => {
                                    return Err(SofError::InvalidViewDefinition(format!(
                                        "Invalid decimal value for constant '{}'",
                                        name
                                    )));
                                }
                            }
                        } else {
                            EvaluationResult::Decimal("0".parse().unwrap(), None)
                        }
                    }
                    ViewDefinitionConstantValue::Date(d) => {
                        EvaluationResult::Date(d.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::DateTime(dt) => {
                        EvaluationResult::DateTime(dt.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Time(t) => {
                        EvaluationResult::Time(t.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Code(c) => {
                        EvaluationResult::String(c.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Base64Binary(b) => {
                        EvaluationResult::String(b.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Id(i) => {
                        EvaluationResult::String(i.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Instant(i) => {
                        EvaluationResult::DateTime(i.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Oid(o) => {
                        EvaluationResult::String(o.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::PositiveInt(p) => {
                        EvaluationResult::Integer(p.value.unwrap_or(1) as i64, None)
                    }
                    ViewDefinitionConstantValue::UnsignedInt(u) => {
                        EvaluationResult::Integer(u.value.unwrap_or(0) as i64, None)
                    }
                    ViewDefinitionConstantValue::Uri(u) => {
                        EvaluationResult::String(u.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Url(u) => {
                        EvaluationResult::String(u.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Uuid(u) => {
                        EvaluationResult::String(u.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Canonical(c) => {
                        EvaluationResult::String(c.value.clone().unwrap_or_default(), None)
                    }
                    ViewDefinitionConstantValue::Integer64(i) => {
                        EvaluationResult::Integer(i.value.unwrap_or(0), None)
                    }
                };
                
                Ok(eval_result)
            } else {
                Err(SofError::InvalidViewDefinition(format!(
                    "Constant '{}' must have a value",
                    name
                )))
            }
        }
    }
    
    impl BundleTrait for Bundle {
        type Resource = Resource;
        
        fn entries(&self) -> Vec<&Self::Resource> {
            self.entry
                .as_ref()
                .map(|entries| {
                    entries
                        .iter()
                        .filter_map(|e| e.resource.as_deref())  // Note: R6 uses Box<Resource>
                        .collect()
                })
                .unwrap_or_default()
        }
    }
    
    impl ResourceTrait for Resource {
        fn resource_name(&self) -> &str {
            self.resource_name()
        }
        
        fn to_fhir_resource(&self) -> FhirResource {
            FhirResource::R6(Box::new(self.clone()))
        }
    }
}