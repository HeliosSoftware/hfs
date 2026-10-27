# Upstream FHIR Schema conformance fixtures

The `*.json` files in this directory are vendored, unmodified, from the
**FHIR Schema** project's language-agnostic conformance suite:

- Source repository: https://github.com/dougc95/fhir-schema (fork of
  https://github.com/fhir-schema/fhir-schema)
- Vendored from local clone at commit: `dff20652992ee2f27d46122598acdf4356298c92`
  (2026-07-03)
- Upstream path: `tests/*.json`
- License: MIT (Copyright (c) 2018-2023 Nikolai Ryzhikov, Ewout Kramer,
  FHIR Community, Health Samurai, Firely) — see the upstream `LICENSE.md`.

These fixtures are the behavioral contract for the validation engine in this
crate: each file bundles inline schemas, resource instances, and the exact
error objects (`{type, path, message}`) a conforming validator must emit, in
order. The harness in `../../conformance.rs` runs them with **exact ordered
deep-equality** — message strings included.

Do not edit these files. Helios-specific behavior beyond the upstream contract
(primitive value checks, `excluded`, numeric cardinality when absent, slicing
rules, etc.) is pinned by our own fixtures in `../extended/` instead.

Fixture semantics honored by the harness:
- a test without an `errors` key must validate clean (`[]`)
- `skip: true` tests are skipped (known-unimplemented upstream behavior)
- `focus: true` is ignored — every non-skipped test always runs
