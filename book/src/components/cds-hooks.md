# CDS Hooks

The `helios-cds-hooks` crate provides Rust types and traits for building [CDS Hooks](https://cds-hooks.hl7.org/) clinical decision support services.

## What Is CDS Hooks?

CDS Hooks is an HL7 standard that allows clinical decision support (CDS) logic to be invoked at specific points in clinical workflows (e.g., when a patient is selected, when an order is signed). Services expose a discovery endpoint and a set of hook endpoints that return actionable cards.

## What This Crate Provides

- **Complete protocol types** — discovery, requests, responses, cards, suggestions, and feedback structs
- **Strongly-typed hook contexts** — context structs for all 10 hooks in the CDS Hooks Library
- **`CdsHooksService` trait** — async trait compatible with any Rust web framework (Axum, Actix, etc.)

## Usage

Add the crate to your `Cargo.toml`:

```toml
[dependencies]
helios-cds-hooks = { path = "path/to/helios-cds-hooks" }
```

Implement the `CdsHooksService` trait and wire it up to your web framework's router.

> Full API documentation is available via `cargo doc --no-deps --open`.
