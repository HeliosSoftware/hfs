---
name: work-with-cds-hooks
description: Work on the HFS CDS Hooks library. Use for CDS Hooks protocol types, strongly-typed hook contexts, the CdsHooksService trait, cards/suggestions/actions/links, and building clinical decision support services.
---

# CDS Hooks

Use this when working in `helios-cds-hooks`. It provides Rust types and an async service trait for building [CDS Hooks](https://cds-hooks.hl7.org/) services conforming to HL7 CDS Hooks **v3.0.0-ballot**. It is a **standalone protocol library** — currently not wired into the `hfs` server — and is framework-agnostic (Axum, Actix, etc.).

## What's in it

- **Complete protocol types** — request, response, discovery, `Card`, `Suggestion`, `Action`, `Link`, `Feedback`, with full serde support (`models.rs`).
- **Strongly-typed hook contexts** (compile-time validated) for the standard hooks (`hooks.rs`): `patient-view`, `order-select`, `order-sign`, `encounter-start`, `encounter-discharge`, `appointment-book`, `order-dispatch`, `allergyintolerance-create`, `medication-refill`, `problem-list-item-create`.
- **Service trait + builders** — `CdsHooksService` async trait and card/response builder helpers (`service.rs`, `lib.rs`).

## Implementing a service

```rust
use async_trait::async_trait;
use helios_cds_hooks::{
    CdsHooksService, CdsService, CdsHooksError,
    hooks::{HookContext, PatientViewContext},
};

struct PatientGreeter;

#[async_trait]
impl CdsHooksService for PatientGreeter {
    type Context = PatientViewContext;

    fn definition(&self) -> CdsService { /* hook, id, title, description */ }
    // request handler returning cards — see crates/cds-hooks/src/service.rs
    // for the exact trait methods and signatures.
}
```

See the full quick-start in `crates/cds-hooks/src/lib.rs`. `CdsHooksError::status_code()` maps errors to HTTP status codes.

## Code map

`lib.rs` (overview, re-exports, builders), `models.rs` (protocol types), `hooks.rs` (hook context structs + `HOOK_NAME` constants), `service.rs` (`CdsHooksService` trait, `CdsHooksError`). Tests are inline in `src/`; integration tests under `crates/cds-hooks/tests/` where present.

## Wiring it in

There is no CDS Hooks HTTP endpoint in `hfs` yet. To expose services, mount `CdsHooksService` implementations in a router (e.g. `GET /cds-services` discovery + `POST /cds-services/{id}` per service).
