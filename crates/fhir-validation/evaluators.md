Nice—this is a great point to formalize the design. You’ve done a non-trivial architectural upgrade, and documenting it now will pay off later (especially when you move to StructureDefinition-based validation).

Since the edit tool failed, I’ll give you both:
1.	Inline documentation (ready to paste into evaluators.rs)
2.	A clean .md file (README-style)

⸻

✅ 1. Inline Documentation (paste into evaluators.rs)

🔹 Add at the very top of the file

//! # FHIRPath Evaluators (Atrius Validation Engine v1)
//!
//! ## Overview
//!
//! This module implements the FHIRPath evaluation layer used by the Atrius
//! validation engine. It is responsible for:
//!
//! - Evaluating invariant expressions (FHIRPath)
//! - Resolving declared paths into focus values
//! - Supporting both single and bulk invariant execution
//!
//! ## Why this refactor?
//!
//! The original design evaluated invariants one-by-one:
//!
//! - Each invariant created its own evaluation context
//! - Each invariant resolved its own focus
//! - Each invariant executed independently
//!
//! This resulted in:
//! - repeated context cloning
//! - repeated FHIRPath evaluation
//! - poor scaling for arrays and nested structures
//!
//! ## New Design (v1)
//!
//! We now use a **batch evaluation model**:
//!
//! - Resolve focus once
//! - Evaluate multiple invariant expressions on that focus
//!
//! ```text
//! Validator
//!   → resolve focus values
//!   → evaluator.eval_invariants_on(...)
//!       → evaluate N expressions
//!       → return results
//! ```
//!
//! ## Key Decisions
//!
//! ### 1. Owned `EvaluationResult`
//!
//! We use:
//! ```rust
//! focus: EvaluationResult
//! ```
//!
//! instead of:
//! ```rust
//! focus: &EvaluationResult
//! ```
//!
//! Reasons:
//! - avoids lifetime complexity
//! - works cleanly with `EvaluationContext::set_this`
//! - aligns with Helios FHIRPath internals
//!
//! Trade-off: cloning may occur, but correctness and simplicity are prioritized.
//!
//! ### 2. `InvariantExprRef`
//!
//! Instead of passing full `InvariantDef`, we use a lightweight struct:
//!
//! ```rust
//! pub struct InvariantExprRef<'a> {
//!     pub declared_path: &'a str,
//!     pub expression: &'a str,
//! }
//! ```
//!
//! This:
//! - decouples evaluator from generator
//! - enables dynamic/runtime invariants
//! - supports future StructureDefinition validation
//!
//! ### 3. Bulk Evaluation API
//!
//! ```rust
//! eval_invariants_on(focus, invariants)
//! ```
//!
//! This:
//! - sets context once
//! - evaluates multiple expressions
//! - returns one result per invariant
//!
//! ## Summary
//!
//! This module provides a:
//! - performant
//! - decoupled
//! - extensible
//!
//! foundation for FHIR invariant evaluation in Atrius.


⸻

🔹 Add above InvariantExprRef

/// Lightweight reference to an invariant expression.
///
/// This struct borrows data instead of owning it, allowing the evaluator
/// to operate independently of generator-specific types (`InvariantDef`).
///
/// Benefits:
/// - enables bulk evaluation
/// - reduces allocation overhead
/// - supports dynamic validation rules in the future


⸻

🔹 Add above eval_invariants_on

/// Evaluate multiple invariants on a single focus value.
///
/// This is the core optimization in the Atrius validation engine:
///
/// - The evaluation context is created once
/// - `%resource` and `%rootResource` are reused
/// - Multiple expressions are evaluated against the same focus
///
/// Compared to per-invariant execution, this:
/// - reduces repeated context cloning
/// - improves performance on arrays and nested structures
///
/// Returns one result per invariant expression.


⸻

🔹 Add above eval_invariant_on

/// Evaluate a single invariant on a focus value.
///
/// This is a thin wrapper over `eval_invariants_on` and exists for API
/// compatibility.
///
/// All actual logic is delegated to the bulk evaluation path to ensure
/// consistent behavior.


⸻

✅ 2. Markdown File (create docs/evaluators.md or README section)

# FHIRPath Evaluators — Atrius Validation Engine v1

## Overview

The evaluator layer is responsible for executing FHIRPath expressions used in:

- Invariants
- Future StructureDefinition rules
- Dynamic validation logic

---

## Problem in v0

The original model evaluated invariants one-by-one:

- Each invariant:
    - resolved its own focus
    - created its own context
    - executed independently

### Issues

- Repeated context cloning
- Repeated FHIRPath execution
- Poor scalability for:
    - arrays
    - nested resources
    - large bundles

---

## Solution in v1

### Batch Evaluation Model

Instead of:

for invariant:
evaluate independently

We now do:

resolve focus once
evaluate N invariants

---

## Core API

```rust
eval_invariants_on(
    focus: EvaluationResult,
    invariants: &[InvariantExprRef]
)


⸻

Key Design Decisions

1. Owned EvaluationResult

We use:

EvaluationResult (owned)

instead of:

&EvaluationResult (borrowed)

Why?
	•	Avoid lifetime complexity
	•	Compatible with Helios FHIRPath
	•	Enables clean propagation across layers

Trade-off
	•	Some cloning
	•	Acceptable for now

⸻

2. InvariantExprRef

pub struct InvariantExprRef<'a> {
    pub declared_path: &'a str,
    pub expression: &'a str,
}

Why?
	•	Decouples evaluator from generator
	•	Enables:
	•	dynamic invariants
	•	StructureDefinition support
	•	runtime rules

⸻

3. Bulk Evaluation

Benefits:
	•	Single context setup
	•	Reuse %resource and %rootResource
	•	Better performance

⸻

Architecture

Validator
  → extract focus values
  → evaluator.eval_invariants_on(...)
      → evaluate expressions
      → return results


⸻

Future Extensions

This design enables:
	•	Profile-based validation
	•	Dynamic rule injection
	•	CDS Hooks validation
	•	Partial resource validation

⸻

Summary

Aspect	        Old	                    New
Execution	    Per invariant	        Batch
Context setup	Repeated	            Once
Coupling	    Tight (InvariantDef)	Decoupled
Extensibility	Limited	                High


⸻

One-line intuition

Evaluate many expressions on one focus — not one expression at a time

---

