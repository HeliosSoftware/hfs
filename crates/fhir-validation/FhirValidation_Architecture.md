Here is a top-level README.md draft for Atrius Validation Engine v1.

# Atrius Validation Engine v1

Atrius Validation Engine v1 is the validation subsystem for the Atrius FHIR stack. It is designed to validate FHIR resources against specification-defined rules using a combination of generated metadata, FHIRPath-based invariant evaluation, and terminology-aware binding validation.

The engine is built around a layered architecture with a local-first terminology strategy, generated resource-specific validators, and support for both synchronous and asynchronous validation flows.

---

## Goals

Atrius Validation Engine v1 aims to provide:

- robust validation of FHIR resources against specification rules
- support for multiple FHIR versions, with current focus on R5
- generated invariant and binding metadata derived from the FHIR specification
- local-first terminology validation with remote fallback
- recursive validation of nested elements and contained resources
- clear reporting of validation issues with severity, FHIR path, and instance path
- separation between validation logic, terminology services, and concrete terminology backends

---

## Current Status

### FHIR versions

- **R5**: primary working target
- **R4**: supported, but not all binding functions are fully updated yet

### Validation types implemented

- ✅ **Invariants**
    - specification-derived constraints evaluated using FHIRPath
- ✅ **Bindings**
    - terminology validation for:
        - `code`
        - `Coding`
        - `CodeableConcept`
        - `CodeableReference`
        - `Quantity`

### Terminology support

- ✅ local generated ValueSet membership checks
- ✅ async remote terminology fallback
- ✅ Helios-backed remote terminology adapter
- ✅ local-first terminology strategy

### Generation pipeline

- ✅ binding extraction into generated metadata
- ✅ invariant extraction into generated metadata
- ✅ emitted validators per resource
- ✅ recursive resource and element validation wiring

---

## High-Level Architecture

The validation engine is organized into four main layers:

1. **Generated validation metadata**
2. **Validation runtime**
3. **Terminology subsystem**
4. **Resource-specific generated validators**

### Layer overview

```text
FHIR specification inputs
        ↓
Generator pipeline
        ↓
Generated BindingDef / InvariantDef / resource validator impls
        ↓
Validation runtime
   ├── invariant evaluation (FHIRPath)
   ├── binding dispatch
   └── recursive child validation
        ↓
Terminology subsystem
   ├── local generated ValueSet checks
   └── remote terminology fallback
        ↓
ValidationIssue output
```

⸻

### Core Concepts

## InvariantDef

- An InvariantDef represents one generated invariant attached to a resource or element.

Examples:
	•	ele-1
	•	dom-3
	•	pat-1

Each invariant includes:
	•	invariant key
	•	severity
	•	declared path
	•	FHIRPath expression
	•	human-readable message

These are evaluated at runtime using the FHIRPath evaluator.

⸻

## BindingDef

A BindingDef represents one generated terminology binding for a resource field.

Each binding includes:
	•	declared path
	•	binding strength
	•	bound ValueSet URL
	•	optional binding name
	•	binding target kind

Binding target kinds currently include:
	•	Code
	•	Coding
	•	CodeableConcept
	•	CodeableReference
	•	Quantity
	•	plus some currently unsupported kinds

The binding dispatcher uses this metadata to:
	•	find matching JSON values
	•	extract instance paths
	•	route each value to the correct validator
	•	attach instance paths to produced issues

⸻

## ValidationIssue

Validation results are returned as ValidationIssue values.

Each issue may include:
	•	severity
	•	issue code
	•	FHIR path
	•	instance path
	•	expression
	•	diagnostics

This allows callers to distinguish:
	•	what rule failed
	•	where it failed logically
	•	where it failed in the concrete resource instance

⸻

### Validation Pipeline

Atrius Validation Engine v1 validates a resource in two main passes:

1. **Invariant validation**

Invariant validation evaluates generated FHIRPath rules against the resource and its child elements.

This includes:
	•	domain resource invariants such as dom-*
	•	element invariants such as ele-1
	•	resource-specific invariants such as obs-*, pat-*, etc.
	•	recursive invariant validation of nested child elements
	•	recursive validation of contained resources

Additional structural checks are also emitted where needed, such as:
	•	arrays must not be present if empty
	•	certain nested structures must obey resource rules

⸻

2. **Binding validation**

Binding validation uses generated BindingDef metadata to validate coded values against their bound ValueSets.

This includes:
	•	locating matching fields in serialized JSON
	•	computing correct instance paths
	•	dispatching based on binding target kind
	•	performing local terminology validation first
	•	falling back to remote terminology when local validation is insufficient

Binding validation currently supports:
	•	primitive code
	•	Coding
	•	CodeableConcept
	•	CodeableReference
	•	Quantity

⸻

### Local-First Terminology Strategy

Atrius Validation Engine v1 uses a local-first terminology approach.

## Local validation

Generated ValueSet code provides best-effort local membership checking.

This includes:
	•	local contains(system, code) checks
	•	implicit-system code validation where possible
	•	coding validation
	•	codeable concept validation
	•	expected display checks for locally known codes
	•	distinction between:
	•	locally known membership success
	•	locally known membership failure
	•	undecidable / remote validation required

## Remote fallback

When local validation cannot decide membership, the engine can delegate to a remote terminology service.

This is especially important when:
	•	the ValueSet has non-local rules
	•	included ValueSets are not generated locally
	•	filtering semantics cannot be evaluated locally
	•	inline or expanded terminology behavior depends on a terminology server

⸻

### Terminology Architecture

The terminology subsystem is layered so that validation logic stays decoupled from transport and backend-specific client details.

## Terminology layers

```text
Validation engine / generated binding validators
        ↓
TerminologyService / TerminologyServiceSync
        ↓
TerminologyBackend
        ↓
HeliosTerminologyBackend
        ↓
Helios TerminologyClient
        ↓
Remote FHIR terminology server
``` 

⸻

## requests.rs

Defines strongly typed terminology request models.

The key request model is ValidateVsRequest, which represents a typed internal model for remote $validate-code requests.

It centralizes:
	•	request structure
	•	lightweight request validation
	•	FHIR Parameters JSON serialization

This allows the backend layer to evolve without changing validator-facing APIs.

⸻

## service.rs

Defines the validation-facing terminology service layer.

Key traits:
	•	TerminologyService (async)
	•	TerminologyServiceSync (sync)

**Narrow `member_of` path (binding validation)**

`TerminologyService::member_of(valueset_url, system, code, display)` is intentionally minimal: one membership check for a single `(system, code, display)` triple. That matches how generated binding validation works end-to-end:

	•	local evaluation may yield `NeedsRemote` carrying only those fields (see `LocalBindingOutcome` / `RemoteMembershipRequest` in the binding engine)
	•	the adapter builds a `ValidateVsRequest` with `code`, `system`, and `display` set and everything else defaulted
	•	it does **not** send full `Coding` or `CodeableConcept` JSON, `systemVersion`, `context`, `date`, or other `$validate-code` parameters

So “narrow” is aligned with what the validator extracts today, not an accidental omission at the HTTP layer.

**Full `ValidateVsRequest` path (integrations)**

`RemoteTerminologyService::validate_vs(&ValidateVsRequest)` performs a complete `ValueSet/$validate-code`: it delegates to `TerminologyBackend::validate_vs`, then parses the response into `TerminologyMembershipOutcome`. Use this when you need richer parameters than `member_of` provides (for example embedded `coding` / `codeableConcept`, `systemVersion`, `context`, or `date`). Custom callers or future engine features can use this without changing the binding-facing trait.

`member_of` is implemented by building a minimal `ValidateVsRequest` and calling `validate_vs` so both paths share one implementation.

⸻

## backend.rs

Defines the backend abstraction:
	•	TerminologyBackend

This is the seam between:
	•	validation/service layers
	•	concrete backend implementations

It currently exposes remote ValueSet validation through:
	•	validate_vs(&ValidateVsRequest)

⸻

## helios.rs

Implements TerminologyBackend using the Helios terminology client.

This adapter:
	•	receives a typed request
	•	validates it with `ValidateVsRequest::validate`
	•	serializes the full `$validate-code` parameter set via `ValidateVsRequest::to_parameters_json`
	•	posts it through `TerminologyClient::validate_code_with_parameters` (same URL routing as the narrow `validate_vs` helper, including HL7 built-in ValueSet instance paths)
	•	converts backend errors into validation errors
	•	returns raw JSON to the service layer

This isolates Helios-specific transport from the rest of the engine while forwarding every modeled request field the server accepts.

⸻

## types.rs

Defines shared terminology result and error types such as:
	•	TerminologyMembershipOutcome
	•	TerminologyRemoteError

These types allow the service and validator layers to work with structured terminology outcomes instead of raw server payloads.

For the full **`ValidationError` / `ValidationIssue` model**, `std::error::Error` chaining, and **production** guidance (HTTP clients, retries, PII in logs), see **[Errors.md](Errors.md)**.

⸻

### Generated Validators

Each generated resource validator implements version-specific validation traits such as:
	•	R5Validatable
	•	R5ValidatableAsync

These generated implementations are responsible for:
	•	applying resource-level binding definitions
	•	applying resource-level invariants
	•	recursively validating child elements
	•	rebasing child instance paths
	•	recursively validating contained resources

For example, a generated resource validator typically:
	•	calls apply_r5_bindings(...) or apply_r5_bindings_async(...)
	•	calls apply_invariants(...)
	•	validates nested fields recursively
	•	uses rebase_instance_paths(...) so child issues map correctly into the parent resource instance

This means resource-specific structure is generated, while generic dispatch and issue handling are implemented once in the runtime.

⸻

### Binding Dispatch Architecture

The binding dispatcher is the runtime component that applies generated BindingDef metadata to actual serialized resource data.

Dispatcher responsibilities
	•	serialize focus object to JSON
	•	derive relative binding path
	•	locate matching JSON values and instance paths
	•	select correct binding validator by BindingTargetKind
	•	stamp correct instance path on produced issues
	•	accumulate all issues across all matched values

There are both:
	•	synchronous dispatch paths
	•	asynchronous dispatch paths

This allows local-only or legacy validation to remain sync while enabling remote terminology integration where needed.

⸻

### Invariant Evaluation Architecture

Invariant evaluation is driven by generated InvariantDef metadata.

Runtime responsibilities
	•	execute generated FHIRPath expressions
	•	map failures into ValidationIssue
	•	preserve invariant key, severity, message, and path context
	•	recursively validate children and contained resources
	•	add structural validation checks not directly represented by invariants where required

The FHIRPath evaluator is therefore the invariant execution engine, while the generated metadata tells it what to run.

⸻

## Severity Model

The engine currently uses two related but distinct severity dimensions:

Validation issue severity

General validation issue severity:
	•	Fatal
	•	Error
	•	Warning
	•	Information

## Binding strength

FHIR binding strength:
	•	Required
	•	Extensible
	•	Preferred
	•	Example

Binding strength influences how terminology failures are surfaced as validation issues.

In general:
	•	Required bindings are treated most strictly
	•	weaker bindings can produce warnings rather than hard errors

This allows the validator to preserve FHIR semantics while still producing actionable issue output.

⸻

### Design Principles

Atrius Validation Engine v1 is built around the following principles:

1. Generated metadata, generic runtime

- Specification-derived metadata is generated once, while validation logic is implemented generically in the runtime.

 - This avoids hand-coding per-resource validation rules.

⸻

2. Local-first terminology

Local ValueSet validation is preferred whenever possible.

Remote terminology calls are used only when required.

This improves:
	•	performance
	•	determinism
	•	offline capability
	•	testability

⸻

3. Narrow validator-facing terminology API

Generated binding validators use a simple membership-oriented terminology interface (`TerminologyService::member_of`): value set URL plus optional system, code, and display per remote check. They do not need to understand full FHIR `$validate-code` semantics.

The richer `ValidateVsRequest` model lives at the backend and on `RemoteTerminologyService::validate_vs` for callers that need it; see **service.rs** above.

⸻

4. Richer backend request model

The backend layer uses a richer request model (ValidateVsRequest) so that future terminology features can be added without reworking validator code.

⸻

5. Clear separation of concerns

Responsibilities are deliberately separated:
	•	generators produce metadata and validator wiring
	•	runtime dispatches validation
	•	terminology services adapt validation questions into backend requests
	•	backends perform remote operations
	•	parsers interpret remote results

⸻

6. Recursive correctness

Nested elements and contained resources are validated recursively, with rebased instance paths so that issues remain accurate in the final resource context.

⸻

### Current Limitations

Atrius Validation Engine v1 is functional, but some areas are intentionally incomplete or still evolving.

Known limitations
	•	R5 is the primary target; R4 support is not yet fully aligned in all binding paths
	•	remote terminology integration is currently focused on the validation patterns needed by binding enforcement; binding-driven remote calls use the narrow `member_of` shape (not every `ValidateVsRequest` field)
	•	when using `RemoteTerminologyService::validate_vs` or `TerminologyBackend::validate_vs` directly, remote `$validate-code` calls can serialize the full `ValidateVsRequest` shape; whether a given field affects the outcome still depends on the remote server’s FHIR version and capabilities
	•	some advanced $validate-code request fields are modeled but not yet fully exercised end-to-end in automated tests
	•	behavior may differ depending on the capabilities and FHIR version of the remote terminology server
	•	not all future terminology operations ($lookup, $expand, $subsumes, etc.) are implemented yet

⸻

### What v1 Delivers

Atrius Validation Engine v1 already provides a strong foundation for production-grade FHIR validation.

It delivers:
	•	generated invariant validation
	•	generated binding validation
	•	local terminology support
	•	remote terminology fallback
	•	sync and async validation flows
	•	recursive resource validation
	•	issue reporting with precise instance paths
	•	layered terminology architecture
	•	version-aware design with primary R5 focus

⸻

### Planned Evolution Beyond v1

Likely next steps include:
	•	complete R4 parity for binding handling
	•	broader automated coverage of remote `$validate-code` parameter combinations
	•	deeper integration tests against live or containerized terminology servers
	•	support for more terminology operations
	•	stronger request-shape tests and parser tests
	•	improved compatibility handling across remote terminology servers
	•	future dynamic profile-aware validation using meta.profile
	•	loading StructureDefinition resources dynamically
	•	applying profile-specific invariants, tightened bindings, and later slicing rules

⸻

### Summary

Atrius Validation Engine v1 is a generated, layered, local-first FHIR validation engine.

It combines:
	•	specification-derived invariant and binding metadata
	•	a generic validation runtime
	•	recursive generated resource validators
	•	a decoupled terminology subsystem
	•	local-first terminology membership checks with remote fallback

The result is a validation architecture that is already practical for R5-focused validation work today, while still leaving a clean path for expansion into richer terminology and profile-aware validation in later versions.

