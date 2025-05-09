# Quantity Handling Implementation Fix

This document provides a step-by-step implementation guide for fixing quantity handling and unit conversion in the FHIRPath implementation.

## 1. Quantity Type Enhancement

### 1.1. Create a Robust Quantity Type

First, we need to enhance the representation of quantities with proper unit conversion support:

```rust
// quantity_impl.rs

use rust_decimal::Decimal;
use std::collections::HashMap;
use std::sync::LazyLock;
use fhirpath_support::{EvaluationResult, EvaluationError};

/// Represents a quantity with a value and unit
#[derive(Debug, Clone, PartialEq)]
pub struct Quantity {
    /// The numeric value
    pub value: Decimal,
    /// The unit of measurement
    pub unit: String,
    /// The canonical unit system (UCUM, FHIR, etc.)
    pub system: Option<String>,
}

impl Quantity {
    /// Creates a new quantity with the given value and unit
    pub fn new(value: Decimal, unit: impl Into<String>) -> Self {
        Self {
            value,
            unit: unit.into(),
            system: None,
        }
    }
    
    /// Creates a new quantity with the given value, unit, and system
    pub fn with_system(value: Decimal, unit: impl Into<String>, system: impl Into<String>) -> Self {
        Self {
            value,
            unit: unit.into(),
            system: Some(system.into()),
        }
    }
    
    /// Converts this quantity to a different unit, if possible
    pub fn convert_to(&self, target_unit: &str) -> Result<Self, EvaluationError> {
        // If units are the same, no conversion needed
        if self.unit == target_unit {
            return Ok(self.clone());
        }
        
        // Get the conversion factor between the units
        if let Some(factor) = get_conversion_factor(&self.unit, target_unit) {
            // Apply the conversion
            let new_value = self.value * factor;
            
            // Create a new quantity with the converted value and target unit
            Ok(Self {
                value: new_value,
                unit: target_unit.to_string(),
                system: self.system.clone(),
            })
        } else {
            // Units are incompatible
            Err(EvaluationError::InvalidOperation(
                format!("Cannot convert from '{}' to '{}'", self.unit, target_unit)
            ))
        }
    }
    
    /// Checks if this quantity is equivalent to another
    pub fn is_equivalent_to(&self, other: &Self, tolerance: Option<Decimal>) -> Result<bool, EvaluationError> {
        // Try to convert the other quantity to this unit
        let converted_other = other.convert_to(&self.unit)?;
        
        // Apply tolerance if provided
        let default_tolerance = Decimal::from_f64(0.01).unwrap(); // 1% by default
        let tolerance_value = tolerance.unwrap_or_else(|| self.value.abs() * default_tolerance);
        
        // Check if the values are within the tolerance
        let difference = (self.value - converted_other.value).abs();
        Ok(difference <= tolerance_value)
    }
    
    /// Adds another quantity to this one
    pub fn add(&self, other: &Self) -> Result<Self, EvaluationError> {
        // Convert the other quantity to this unit
        let converted_other = other.convert_to(&self.unit)?;
        
        // Add the values
        let new_value = self.value + converted_other.value;
        
        // Create a new quantity with the result
        Ok(Self {
            value: new_value,
            unit: self.unit.clone(),
            system: self.system.clone(),
        })
    }
    
    /// Subtracts another quantity from this one
    pub fn subtract(&self, other: &Self) -> Result<Self, EvaluationError> {
        // Convert the other quantity to this unit
        let converted_other = other.convert_to(&self.unit)?;
        
        // Subtract the values
        let new_value = self.value - converted_other.value;
        
        // Create a new quantity with the result
        Ok(Self {
            value: new_value,
            unit: self.unit.clone(),
            system: self.system.clone(),
        })
    }
    
    /// Multiplies this quantity by another
    pub fn multiply(&self, other: &Self) -> Result<Self, EvaluationError> {
        // Multiply the values
        let new_value = self.value * other.value;
        
        // Combine the units
        let new_unit = if self.unit == "1" {
            other.unit.clone()
        } else if other.unit == "1" {
            self.unit.clone()
        } else {
            // For simple cases, just combine the units
            format!("{}*{}", self.unit, other.unit)
        };
        
        // Create a new quantity with the result
        Ok(Self {
            value: new_value,
            unit: new_unit,
            system: self.system.clone(),
        })
    }
    
    /// Divides this quantity by another
    pub fn divide(&self, other: &Self) -> Result<Self, EvaluationError> {
        // Check for division by zero
        if other.value.is_zero() {
            return Err(EvaluationError::DivisionByZero);
        }
        
        // Divide the values
        let new_value = self.value / other.value;
        
        // Combine the units
        let new_unit = if self.unit == other.unit {
            // Same units cancel out
            "1".to_string()
        } else if other.unit == "1" {
            // Division by unitless quantity
            self.unit.clone()
        } else {
            // For simple cases, just combine the units
            format!("{}/{}", self.unit, other.unit)
        };
        
        // Create a new quantity with the result
        Ok(Self {
            value: new_value,
            unit: new_unit,
            system: self.system.clone(),
        })
    }
    
    /// Gets the absolute value of this quantity
    pub fn abs(&self) -> Self {
        Self {
            value: self.value.abs(),
            unit: self.unit.clone(),
            system: self.system.clone(),
        }
    }
    
    /// Converts this quantity to a string representation
    pub fn to_string(&self) -> String {
        format!("{} '{}'", self.value, self.unit)
    }
}

/// Define unit conversion registry
pub struct UnitRegistry {
    /// Map of unit categories (e.g., mass, length, time)
    categories: HashMap<String, UnitCategory>,
}

/// Represents a category of units with conversion factors
struct UnitCategory {
    /// Base unit for this category
    base_unit: String,
    /// Conversion factors to the base unit
    conversion_factors: HashMap<String, Decimal>,
}

impl UnitRegistry {
    /// Creates a new unit registry
    pub fn new() -> Self {
        Self {
            categories: HashMap::new(),
        }
    }
    
    /// Adds a category with the given base unit
    pub fn add_category(&mut self, category: &str, base_unit: &str) {
        let mut conversion_factors = HashMap::new();
        conversion_factors.insert(base_unit.to_string(), Decimal::ONE);
        
        self.categories.insert(
            category.to_string(),
            UnitCategory {
                base_unit: base_unit.to_string(),
                conversion_factors,
            },
        );
    }
    
    /// Adds a unit conversion factor to a category
    pub fn add_conversion(&mut self, category: &str, unit: &str, factor: Decimal) {
        if let Some(cat) = self.categories.get_mut(category) {
            cat.conversion_factors.insert(unit.to_string(), factor);
        }
    }
    
    /// Gets the conversion factor between two units
    pub fn get_conversion_factor(&self, from_unit: &str, to_unit: &str) -> Option<Decimal> {
        // If units are the same, return 1.0
        if from_unit == to_unit {
            return Some(Decimal::ONE);
        }
        
        // Handle dimensionless conversions
        if from_unit == "1" && to_unit == "1" {
            return Some(Decimal::ONE);
        }
        
        // Search for a category that contains both units
        for category in self.categories.values() {
            if let (Some(from_factor), Some(to_factor)) = (
                category.conversion_factors.get(from_unit),
                category.conversion_factors.get(to_unit),
            ) {
                // Convert from the source unit to the base unit, then to the target unit
                let conversion = from_factor / to_factor;
                return Some(conversion);
            }
        }
        
        // Special case for UCUM prefixes
        if let Some(conversion) = try_ucum_prefix_conversion(from_unit, to_unit) {
            return Some(conversion);
        }
        
        None
    }
}

/// Try to convert between units using UCUM prefix rules
fn try_ucum_prefix_conversion(from_unit: &str, to_unit: &str) -> Option<Decimal> {
    // Handle common metric prefixes
    let prefix_factors: HashMap<&str, Decimal> = [
        ("Y", Decimal::from(10).pow(24)), // yotta
        ("Z", Decimal::from(10).pow(21)), // zetta
        ("E", Decimal::from(10).pow(18)), // exa
        ("P", Decimal::from(10).pow(15)), // peta
        ("T", Decimal::from(10).pow(12)), // tera
        ("G", Decimal::from(10).pow(9)),  // giga
        ("M", Decimal::from(10).pow(6)),  // mega
        ("k", Decimal::from(10).pow(3)),  // kilo
        ("h", Decimal::from(10).pow(2)),  // hecto
        ("da", Decimal::from(10).pow(1)), // deka
        ("d", Decimal::from(10).pow(-1)), // deci
        ("c", Decimal::from(10).pow(-2)), // centi
        ("m", Decimal::from(10).pow(-3)), // milli
        ("u", Decimal::from(10).pow(-6)), // micro
        ("n", Decimal::from(10).pow(-9)), // nano
        ("p", Decimal::from(10).pow(-12)), // pico
        ("f", Decimal::from(10).pow(-15)), // femto
        ("a", Decimal::from(10).pow(-18)), // atto
        ("z", Decimal::from(10).pow(-21)), // zepto
        ("y", Decimal::from(10).pow(-24)), // yocto
    ].iter().cloned().collect();
    
    // Try to extract prefix and base unit
    for (from_prefix, from_factor) in &prefix_factors {
        if from_unit.starts_with(from_prefix) {
            let from_base = &from_unit[from_prefix.len()..];
            
            for (to_prefix, to_factor) in &prefix_factors {
                if to_unit.starts_with(to_prefix) {
                    let to_base = &to_unit[to_prefix.len()..];
                    
                    // If the base units match, we can convert
                    if from_base == to_base {
                        return Some(from_factor / to_factor);
                    }
                }
            }
            
            // Check if converting to the base unit
            if from_base == to_unit {
                return Some(*from_factor);
            }
        }
    }
    
    // Check if converting from the base unit
    for (to_prefix, to_factor) in &prefix_factors {
        if to_unit.starts_with(to_prefix) {
            let to_base = &to_unit[to_prefix.len()..];
            
            if from_unit == to_base {
                return Some(Decimal::ONE / to_factor);
            }
        }
    }
    
    None
}

/// Initialize the unit registry with common conversions
static UNIT_REGISTRY: LazyLock<UnitRegistry> = LazyLock::new(|| {
    let mut registry = UnitRegistry::new();
    
    // Mass units
    registry.add_category("mass", "g");
    registry.add_conversion("mass", "kg", Decimal::from(1000));
    registry.add_conversion("mass", "mg", Decimal::from_f64(0.001).unwrap());
    registry.add_conversion("mass", "ug", Decimal::from_f64(0.000001).unwrap());
    registry.add_conversion("mass", "lb", Decimal::from_f64(453.59237).unwrap());
    registry.add_conversion("mass", "oz", Decimal::from_f64(28.3495231).unwrap());
    registry.add_conversion("mass", "[lb_av]", Decimal::from_f64(453.59237).unwrap()); // UCUM pound
    
    // Length units
    registry.add_category("length", "m");
    registry.add_conversion("length", "km", Decimal::from(1000));
    registry.add_conversion("length", "cm", Decimal::from_f64(0.01).unwrap());
    registry.add_conversion("length", "mm", Decimal::from_f64(0.001).unwrap());
    registry.add_conversion("length", "um", Decimal::from_f64(0.000001).unwrap());
    registry.add_conversion("length", "in", Decimal::from_f64(0.0254).unwrap());
    registry.add_conversion("length", "ft", Decimal::from_f64(0.3048).unwrap());
    registry.add_conversion("length", "yd", Decimal::from_f64(0.9144).unwrap());
    registry.add_conversion("length", "mi", Decimal::from_f64(1609.344).unwrap());
    
    // Area units
    registry.add_category("area", "m2");
    registry.add_conversion("area", "cm2", Decimal::from_f64(0.0001).unwrap());
    registry.add_conversion("area", "mm2", Decimal::from_f64(0.000001).unwrap());
    registry.add_conversion("area", "in2", Decimal::from_f64(0.00064516).unwrap());
    registry.add_conversion("area", "ft2", Decimal::from_f64(0.09290304).unwrap());
    
    // Volume units
    registry.add_category("volume", "m3");
    registry.add_conversion("volume", "L", Decimal::from_f64(0.001).unwrap());
    registry.add_conversion("volume", "mL", Decimal::from_f64(0.000001).unwrap());
    registry.add_conversion("volume", "cm3", Decimal::from_f64(0.000001).unwrap());
    registry.add_conversion("volume", "in3", Decimal::from_f64(0.000016387064).unwrap());
    registry.add_conversion("volume", "fl_oz", Decimal::from_f64(0.0000295735296).unwrap());
    
    // Time units
    registry.add_category("time", "s");
    registry.add_conversion("time", "min", Decimal::from(60));
    registry.add_conversion("time", "h", Decimal::from(3600));
    registry.add_conversion("time", "d", Decimal::from(86400));
    registry.add_conversion("time", "wk", Decimal::from(604800));
    registry.add_conversion("time", "mo", Decimal::from(2592000)); // Approx 30 days
    registry.add_conversion("time", "a", Decimal::from(31536000)); // Approx 365 days
    registry.add_conversion("time", "day", Decimal::from(86400));
    registry.add_conversion("time", "week", Decimal::from(604800));
    registry.add_conversion("time", "month", Decimal::from(2592000));
    registry.add_conversion("time", "year", Decimal::from(31536000));
    registry.add_conversion("time", "days", Decimal::from(86400));
    registry.add_conversion("time", "weeks", Decimal::from(604800));
    registry.add_conversion("time", "months", Decimal::from(2592000));
    registry.add_conversion("time", "years", Decimal::from(31536000));
    
    registry
});

/// Gets the conversion factor between two units
pub fn get_conversion_factor(from_unit: &str, to_unit: &str) -> Option<Decimal> {
    UNIT_REGISTRY.get_conversion_factor(from_unit, to_unit)
}

/// Parses a quantity from a string
pub fn parse_quantity(value: &str) -> Result<Quantity, EvaluationError> {
    // Split the string by spaces
    let parts: Vec<&str> = value.trim().split_whitespace().collect();
    
    if parts.len() < 2 {
        return Err(EvaluationError::InvalidFormat(
            format!("Invalid quantity format: {}", value)
        ));
    }
    
    // Parse the numeric value
    let value_str = parts[0];
    let value = Decimal::from_str(value_str).map_err(|e| {
        EvaluationError::InvalidFormat(
            format!("Invalid quantity value: {} - {}", value_str, e)
        )
    })?;
    
    // Extract the unit, removing any surrounding quotes
    let unit_str = parts[1].trim_matches('\'');
    
    Ok(Quantity::new(value, unit_str))
}

/// Convert EvaluationResult to Quantity
pub fn result_to_quantity(result: &EvaluationResult) -> Result<Quantity, EvaluationError> {
    match result {
        EvaluationResult::Quantity(value, unit) => {
            Ok(Quantity::new(*value, unit.clone()))
        },
        EvaluationResult::Integer(value) => {
            // Default to unitless quantity
            Ok(Quantity::new(Decimal::from(*value), "1"))
        },
        EvaluationResult::Decimal(value) => {
            // Default to unitless quantity
            Ok(Quantity::new(*value, "1"))
        },
        EvaluationResult::String(value) => {
            // Try to parse the string as a quantity
            parse_quantity(value)
        },
        _ => Err(EvaluationError::TypeError(
            format!("Cannot convert {} to Quantity", result.type_name())
        )),
    }
}

/// Convert Quantity to EvaluationResult
pub fn quantity_to_result(quantity: &Quantity) -> EvaluationResult {
    EvaluationResult::Quantity(quantity.value, quantity.unit.clone())
}
```

### 1.2. Enhanced Quantity Comparison

Implement improved quantity comparison operations in the evaluator:

```rust
// In evaluator.rs

use crate::quantity_impl::{result_to_quantity, quantity_to_result, Quantity};

/// Compares two quantities
fn compare_quantities(left: &EvaluationResult, right: &EvaluationResult, op: &str) -> Result<EvaluationResult, EvaluationError> {
    // Convert both values to Quantity
    let left_quantity = result_to_quantity(left)?;
    let right_quantity = result_to_quantity(right)?;
    
    match op {
        "=" => {
            // Try to convert to the same unit and compare values
            let converted_right = right_quantity.convert_to(&left_quantity.unit)?;
            Ok(EvaluationResult::Boolean(left_quantity.value == converted_right.value))
        },
        "!=" => {
            // Try to convert to the same unit and compare values
            let converted_right = right_quantity.convert_to(&left_quantity.unit)?;
            Ok(EvaluationResult::Boolean(left_quantity.value != converted_right.value))
        },
        "~" => {
            // Check if the quantities are equivalent within tolerance
            let are_equivalent = left_quantity.is_equivalent_to(&right_quantity, None)?;
            Ok(EvaluationResult::Boolean(are_equivalent))
        },
        "!~" => {
            // Check if the quantities are not equivalent within tolerance
            let are_equivalent = left_quantity.is_equivalent_to(&right_quantity, None)?;
            Ok(EvaluationResult::Boolean(!are_equivalent))
        },
        "<" => {
            // Try to convert to the same unit and compare values
            let converted_right = right_quantity.convert_to(&left_quantity.unit)?;
            Ok(EvaluationResult::Boolean(left_quantity.value < converted_right.value))
        },
        "<=" => {
            // Try to convert to the same unit and compare values
            let converted_right = right_quantity.convert_to(&left_quantity.unit)?;
            Ok(EvaluationResult::Boolean(left_quantity.value <= converted_right.value))
        },
        ">" => {
            // Try to convert to the same unit and compare values
            let converted_right = right_quantity.convert_to(&left_quantity.unit)?;
            Ok(EvaluationResult::Boolean(left_quantity.value > converted_right.value))
        },
        ">=" => {
            // Try to convert to the same unit and compare values
            let converted_right = right_quantity.convert_to(&left_quantity.unit)?;
            Ok(EvaluationResult::Boolean(left_quantity.value >= converted_right.value))
        },
        _ => Err(EvaluationError::InvalidOperation(
            format!("Unsupported quantity comparison operation: {}", op)
        )),
    }
}

// Update the equality comparison functions to use quantity comparison
fn compare_equality(left: &EvaluationResult, op: &str, right: &EvaluationResult) -> Result<EvaluationResult, EvaluationError> {
    // Special handling for quantities
    if (left.is_quantity() || left.is_decimal() || left.is_integer()) &&
       (right.is_quantity() || right.is_decimal() || right.is_integer()) {
        if left.is_quantity() || right.is_quantity() {
            // At least one is a quantity, use quantity comparison
            return compare_quantities(left, right, op);
        }
    }
    
    // Handle other equality comparisons...
}

// Update the inequality comparison functions similarly
fn compare_inequality(left: &EvaluationResult, op: &str, right: &EvaluationResult) -> Result<EvaluationResult, EvaluationError> {
    // Special handling for quantities
    if (left.is_quantity() || left.is_decimal() || left.is_integer()) &&
       (right.is_quantity() || right.is_decimal() || right.is_integer()) {
        if left.is_quantity() || right.is_quantity() {
            // At least one is a quantity, use quantity comparison
            return compare_quantities(left, right, op);
        }
    }
    
    // Handle other inequality comparisons...
}
```

### 1.3. Enhanced Quantity Arithmetic

Implement improved quantity arithmetic operations:

```rust
// In evaluator.rs

/// Applies arithmetic operations to quantities
fn apply_quantity_arithmetic(left: &Quantity, op: &str, right: &Quantity) -> Result<EvaluationResult, EvaluationError> {
    match op {
        "+" => {
            // Add the quantities
            let result = left.add(right)?;
            Ok(quantity_to_result(&result))
        },
        "-" => {
            // Subtract the quantities
            let result = left.subtract(right)?;
            Ok(quantity_to_result(&result))
        },
        "*" => {
            // Multiply the quantities
            let result = left.multiply(right)?;
            Ok(quantity_to_result(&result))
        },
        "/" => {
            // Divide the quantities
            let result = left.divide(right)?;
            Ok(quantity_to_result(&result))
        },
        _ => Err(EvaluationError::InvalidOperation(
            format!("Unsupported quantity arithmetic operation: {}", op)
        )),
    }
}

// Update the arithmetic operation functions to use quantity arithmetic
fn apply_multiplicative(left: &EvaluationResult, op: &str, right: &EvaluationResult) -> Result<EvaluationResult, EvaluationError> {
    // Special handling for quantities
    if (left.is_quantity() || left.is_decimal() || left.is_integer()) &&
       (right.is_quantity() || right.is_decimal() || right.is_integer()) {
        if left.is_quantity() || right.is_quantity() {
            // At least one is a quantity, use quantity arithmetic
            let left_quantity = result_to_quantity(left)?;
            let right_quantity = result_to_quantity(right)?;
            return apply_quantity_arithmetic(&left_quantity, op, &right_quantity);
        }
    }
    
    // Handle other multiplicative operations...
}

// Update the additive operation functions similarly
fn apply_additive(left: &EvaluationResult, op: &str, right: &EvaluationResult) -> Result<EvaluationResult, EvaluationError> {
    // Special handling for quantities
    if (left.is_quantity() || left.is_decimal() || left.is_integer()) &&
       (right.is_quantity() || right.is_decimal() || right.is_integer()) {
        if left.is_quantity() || right.is_quantity() {
            // At least one is a quantity, use quantity arithmetic
            let left_quantity = result_to_quantity(left)?;
            let right_quantity = result_to_quantity(right)?;
            return apply_quantity_arithmetic(&left_quantity, op, &right_quantity);
        }
    }
    
    // Handle other additive operations...
}
```

### 1.4. Implement Quantity Conversion Functions

Add functions for converting between quantities and handling quantity operations:

```rust
// In evaluator.rs

/// Evaluates the abs() function for quantities
fn evaluate_abs_quantity(value: &EvaluationResult) -> Result<EvaluationResult, EvaluationError> {
    match value {
        EvaluationResult::Quantity(amount, unit) => {
            Ok(EvaluationResult::Quantity(amount.abs(), unit.clone()))
        },
        _ => {
            // Try to convert to quantity first
            let quantity = result_to_quantity(value)?;
            let abs_quantity = quantity.abs();
            Ok(quantity_to_result(&abs_quantity))
        }
    }
}

/// Evaluates the toQuantity() function
fn evaluate_to_quantity(value: &EvaluationResult, unit_arg: Option<&EvaluationResult>) -> Result<EvaluationResult, EvaluationError> {
    // Get the target unit if provided
    let target_unit = if let Some(EvaluationResult::String(unit)) = unit_arg {
        Some(unit.as_str())
    } else {
        None
    };
    
    match value {
        EvaluationResult::Quantity(amount, unit) => {
            if let Some(target) = target_unit {
                // Convert to the target unit
                let quantity = Quantity::new(*amount, unit.clone());
                let converted = quantity.convert_to(target)?;
                Ok(quantity_to_result(&converted))
            } else {
                // No unit provided, return as is
                Ok(value.clone())
            }
        },
        EvaluationResult::String(s) => {
            // Try to parse the string as a quantity
            let mut quantity = parse_quantity(s)?;
            
            // Convert to the target unit if provided
            if let Some(target) = target_unit {
                quantity = quantity.convert_to(target)?;
            }
            
            Ok(quantity_to_result(&quantity))
        },
        EvaluationResult::Integer(i) => {
            if let Some(target) = target_unit {
                // Create a quantity with the target unit
                let quantity = Quantity::new(Decimal::from(*i), target);
                Ok(quantity_to_result(&quantity))
            } else {
                // No unit provided, create a unitless quantity
                let quantity = Quantity::new(Decimal::from(*i), "1");
                Ok(quantity_to_result(&quantity))
            }
        },
        EvaluationResult::Decimal(d) => {
            if let Some(target) = target_unit {
                // Create a quantity with the target unit
                let quantity = Quantity::new(*d, target);
                Ok(quantity_to_result(&quantity))
            } else {
                // No unit provided, create a unitless quantity
                let quantity = Quantity::new(*d, "1");
                Ok(quantity_to_result(&quantity))
            }
        },
        _ => Err(EvaluationError::TypeError(
            format!("Cannot convert {} to Quantity", value.type_name())
        )),
    }
}

/// Evaluates the convertsToQuantity() function
fn evaluate_converts_to_quantity(value: &EvaluationResult, unit_arg: Option<&EvaluationResult>) -> Result<EvaluationResult, EvaluationError> {
    // Try to convert to quantity, but don't propagate errors
    match evaluate_to_quantity(value, unit_arg) {
        Ok(_) => Ok(EvaluationResult::Boolean(true)),
        Err(_) => Ok(EvaluationResult::Boolean(false)),
    }
}
```

## 2. Quantity Type Integration

### 2.1. Update Function Dispatch in Evaluator

Integrate the quantity functions into the evaluator's function dispatch mechanism:

```rust
// In evaluator.rs

/// Evaluates a function invocation
fn evaluate_function(
    name: &str,
    args: &[EvaluationResult],
    context: &EvaluationContext,
) -> Result<EvaluationResult, EvaluationError> {
    match name {
        // ... existing functions ...
        
        // Quantity functions
        "toQuantity" => {
            let unit_arg = args.get(0);
            evaluate_to_quantity(args[0], unit_arg)
        },
        "convertsToQuantity" => {
            let unit_arg = args.get(0);
            evaluate_converts_to_quantity(args[0], unit_arg)
        },
        "abs" => {
            if args.len() != 1 {
                return Err(EvaluationError::InvalidArgument(
                    format!("abs() requires exactly one argument, got {}", args.len())
                ));
            }
            
            match args[0] {
                EvaluationResult::Integer(i) => Ok(EvaluationResult::Integer(i.abs())),
                EvaluationResult::Decimal(d) => Ok(EvaluationResult::Decimal(d.abs())),
                EvaluationResult::Quantity(_, _) => evaluate_abs_quantity(&args[0]),
                _ => Err(EvaluationError::TypeError(
                    format!("abs() requires a numeric argument, got {}", args[0].type_name())
                )),
            }
        },
        
        // ... other functions ...
    }
}

// Update function invocation in evaluate_invocation
fn evaluate_invocation(
    invocation_base: &EvaluationResult,
    invocation: &Invocation,
    context: &EvaluationContext,
) -> Result<EvaluationResult, EvaluationError> {
    match invocation {
        // ... existing cases ...
        
        Invocation::Function(name, args_exprs) => {
            // ... existing function handling ...
            
            match name.as_str() {
                // ... existing functions ...
                
                "toQuantity" => {
                    // Handle toQuantity() function
                    if args.len() > 1 {
                        return Err(EvaluationError::InvalidArgument(
                            format!("toQuantity() takes at most one argument, got {}", args.len())
                        ));
                    }
                    
                    let unit_arg = args.get(0);
                    evaluate_to_quantity(invocation_base, unit_arg)
                },
                "convertsToQuantity" => {
                    // Handle convertsToQuantity() function
                    if args.len() > 1 {
                        return Err(EvaluationError::InvalidArgument(
                            format!("convertsToQuantity() takes at most one argument, got {}", args.len())
                        ));
                    }
                    
                    let unit_arg = args.get(0);
                    evaluate_converts_to_quantity(invocation_base, unit_arg)
                },
                
                // ... other functions ...
            }
        },
        
        // ... other cases ...
    }
}
```

### 2.2. Update toString Function for Quantities

Enhance the toString function to properly format quantities:

```rust
// In evaluator.rs

/// Evaluates the toString() function
fn evaluate_to_string(value: &EvaluationResult) -> Result<EvaluationResult, EvaluationError> {
    match value {
        EvaluationResult::Quantity(amount, unit) => {
            // Format quantity with proper quoting
            let unit_str = if unit == "1" {
                "1".to_string()
            } else {
                format!("'{}'", unit)
            };
            
            let result = format!("{} {}", amount, unit_str);
            Ok(EvaluationResult::String(result))
        },
        // ... existing cases ...
    }
}
```

## 3. Testing the Implementation

Create unit tests to verify the quantity fixes:

```rust
// In appropriate test file

#[test]
fn test_quantity_equality() {
    // Test basic quantity equality
    assert_eq!(
        evaluate("4.0 'g' = 4.0 'g'", &context, None).unwrap(),
        EvaluationResult::Boolean(true)
    );
    
    // Test quantity equality with unit conversion
    assert_eq!(
        evaluate("4.0 'g' = 4000 'mg'", &context, None).unwrap(),
        EvaluationResult::Boolean(true)
    );
    
    // Test quantity inequality
    assert_eq!(
        evaluate("4.0 'g' != 5.0 'g'", &context, None).unwrap(),
        EvaluationResult::Boolean(true)
    );
    
    // Test quantity equivalence
    assert_eq!(
        evaluate("4.0 'g' ~ 4000 'mg'", &context, None).unwrap(),
        EvaluationResult::Boolean(true)
    );
    
    // Test quantity equivalence with tolerance
    assert_eq!(
        evaluate("4.0 'g' ~ 4040 'mg'", &context, None).unwrap(),
        EvaluationResult::Boolean(true)
    );
}

#[test]
fn test_time_unit_conversion() {
    // Test time unit equality
    assert_eq!(
        evaluate("7 'days' = 1 'week'", &context, None).unwrap(),
        EvaluationResult::Boolean(true)
    );
    
    // Test time unit comparisons
    assert_eq!(
        evaluate("6 'days' < 1 'week'", &context, None).unwrap(),
        EvaluationResult::Boolean(true)
    );
    
    assert_eq!(
        evaluate("8 'days' > 1 'week'", &context, None).unwrap(),
        EvaluationResult::Boolean(true)
    );
}

#[test]
fn test_quantity_arithmetic() {
    // Test quantity addition
    assert_eq!(
        evaluate("2.0 'g' + 3.0 'g'", &context, None).unwrap(),
        EvaluationResult::Quantity(Decimal::from_f64(5.0).unwrap(), "g".to_string())
    );
    
    // Test quantity addition with unit conversion
    assert_eq!(
        evaluate("2.0 'g' + 500 'mg'", &context, None).unwrap(),
        EvaluationResult::Quantity(Decimal::from_f64(2.5).unwrap(), "g".to_string())
    );
    
    // Test quantity subtraction
    assert_eq!(
        evaluate("5.0 'g' - 2.0 'g'", &context, None).unwrap(),
        EvaluationResult::Quantity(Decimal::from_f64(3.0).unwrap(), "g".to_string())
    );
    
    // Test quantity multiplication
    assert_eq!(
        evaluate("2.0 'cm' * 2.0 'm'", &context, None).unwrap(),
        EvaluationResult::Quantity(Decimal::from_f64(4.0).unwrap(), "cm*m".to_string())
    );
    
    // Test quantity division
    assert_eq!(
        evaluate("4.0 'g' / 2.0 'm'", &context, None).unwrap(),
        EvaluationResult::Quantity(Decimal::from_f64(2.0).unwrap(), "g/m".to_string())
    );
}

#[test]
fn test_quantity_functions() {
    // Test toQuantity() function
    assert_eq!(
        evaluate("'5 g'.toQuantity()", &context, None).unwrap(),
        EvaluationResult::Quantity(Decimal::from_f64(5.0).unwrap(), "g".to_string())
    );
    
    // Test toQuantity() with unit conversion
    assert_eq!(
        evaluate("'5 g'.toQuantity('kg')", &context, None).unwrap(),
        EvaluationResult::Quantity(Decimal::from_f64(0.005).unwrap(), "kg".to_string())
    );
    
    // Test convertsToQuantity() function
    assert_eq!(
        evaluate("'5 g'.convertsToQuantity()", &context, None).unwrap(),
        EvaluationResult::Boolean(true)
    );
    
    assert_eq!(
        evaluate("'not a quantity'.convertsToQuantity()", &context, None).unwrap(),
        EvaluationResult::Boolean(false)
    );
    
    // Test abs() function with quantities
    assert_eq!(
        evaluate("(-5.5 'mg').abs()", &context, None).unwrap(),
        EvaluationResult::Quantity(Decimal::from_f64(5.5).unwrap(), "mg".to_string())
    );
}

#[test]
fn test_quantity_toString() {
    // Test toString() for quantities
    assert_eq!(
        evaluate("(5 'g').toString()", &context, None).unwrap(),
        EvaluationResult::String("5 'g'".to_string())
    );
    
    // Test toString() for quantities with units containing spaces
    assert_eq!(
        evaluate("(5 'mg/mL').toString()", &context, None).unwrap(),
        EvaluationResult::String("5 'mg/mL'".to_string())
    );
    
    // Test toString() for time units
    assert_eq!(
        evaluate("(1 'week').toString()", &context, None).unwrap(),
        EvaluationResult::String("1 'week'".to_string())
    );
}
```

## 4. Main Changes Summary

This implementation fixes the following key issues:

1. **Quantity Unit Conversion**:
   - Implements a comprehensive unit conversion system
   - Supports metric prefixes and UCUM units
   - Handles time unit conversions (days, weeks, etc.)

2. **Quantity Comparison**:
   - Properly handles equality and inequality comparisons with unit conversion
   - Implements the equivalent (~) operator with tolerance
   - Fixes comparisons between quantities with different units

3. **Quantity Arithmetic**:
   - Implements proper arithmetic operations (add, subtract, multiply, divide)
   - Handles unit conversions during arithmetic
   - Properly combines units during multiplication and division

4. **Quantity Functions**:
   - Enhances toQuantity() and convertsToQuantity() functions
   - Fixes abs() function to work with quantities
   - Improves toString() formatting for quantities

These changes address the quantity-related failing tests, which are an important part of the FHIRPath implementation.