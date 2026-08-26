# Grupo B/C/D — Playwright residuals batch

Timestamp: 2026-08-19 01:44 (Windows local)

## Result

- Before: **60 passed / 15 failed / 3 skipped** (post-Grupo A, commit `61bfc4f59`).
- After : **73 passed /  2 failed / 3 skipped** — wall time **50.9 s**.
- Net progression: **+13** tests, no regressions, wall time still under 1 min.

## Diagnosis inputs

Three read-only `explore` subagents ran in parallel to avoid compilation / port races:

- `edson/docs/hts-ui-grupo-b-diagnosis.md` — Import submit disabled.
- `edson/docs/hts-ui-grupo-c-diagnosis.md` — Detail-page tab-click swap.
- `edson/docs/hts-ui-grupo-d-diagnosis.md` — Five small drifts.

Two diagnoses overlapped:
- Group B's `uptime_seconds` `u64`→`f64` fix also resolves Group D's `/health` tile drift.
- Group C's `ex-vs-too-costly` seed fix also unblocks Group D's too-costly banner test (VS:132).

## Fixes applied

All within one build cycle; no handler surface changes.

| File | Change | Why | Tests unblocked |
|---|---|---|---|
| `crates/hts-ui/src/upstream.rs` | `UpstreamHealth.uptime_seconds`: `u64`→`f64`; `uptime_pretty` floors the float; added `health_deserializes_fractional_uptime_seconds` unit test; updated `uptime_pretty_shapes_units_from_seconds` cases to include a fractional-boot second. | HTS `/health` emits `helios_observability::uptime` as a fractional-second float. The `u64` shape rejected `0.218212` with a JSON decode error, which set `degraded_reason` and left Import's submit `disabled` on server render. | Import ×4, Diagnostics `/health` ×1 |
| `crates/hts-ui/templates/pages/cs-detail.html`<br>`crates/hts-ui/templates/pages/vs-detail.html`<br>`crates/hts-ui/templates/pages/cm-detail.html` | Inserted a hidden `<div id="hts-workbench-input" hidden></div>` inside the Metadata branch of each detail page. | Operation tabs (Lookup / Expand / Translate) declare `hx-target="#hts-workbench-input"`. On the Metadata landing that id was absent, so the htmx swap failed silently and the Run/Translate button never appeared. Chose the placeholder path (Grupo C Option C) over Option B (workbench-level swap + handler rewrite) because it's a 3-line change per template and leaves the Rust ring tests untouched. | CS:65, VS:81, CM:91 |
| `crates/hts-ui/templates/partials/hts-cm-translate-input.html` | Direction radios: `hx-trigger="change"` → `hx-trigger="click"`; expanded the trigger comment to document the intent. | Diagnosis hypothesis for CM:139. Did **not** resolve CM:139 in practice — see "Known residuals" below. Kept the change because clicking is still the more explicit trigger for a two-state toggle. | (none — see residuals) |
| `crates/hts-ui/e2e/seed.mjs` | Added `ex-vs-too-costly` ValueSet that reuses `ex-cs-limbs` (60 concepts). Skipped the `ex-cs-huge` 3600-concept fixture from the diagnosis in favor of reusing the already-seeded limbs CS, keeping bundle size ~unchanged. | The value-sets spec references `ex-vs-too-costly` but the seed never produced it. | VS:132 (once combined with the count-clear below) |
| `crates/hts-ui/e2e/boot.mjs` | Added `HTS_MAX_EXPANSION_SIZE: "5"` to the child env. | Trip the too-costly gate at 60 concepts. Kept `ex-vs-1` flat-expand green because that spec fills `count=50`, and HTS only enforces the ceiling when `count.is_none()` (see `crates/hts/src/backends/sqlite/value_set.rs`). | Same as above |
| `crates/hts-ui/e2e/tests/dashboard.spec.ts` | Sidebar assertion scoped from `getByRole("navigation")` to `#sidebar nav`. | Strict-mode collision with quick-link nav landmarks that duplicated the sidebar labels. | Dashboard sidebar |
| `crates/hts-ui/e2e/tests/code-systems.spec.ts` | Filter debounce: cell locator gained `exact: true` (avoids substring collision with 30 `.../filler-N` URLs). Load-more: replaced the fragile `before + 6` assertion with a progress-not-regression poll that also bounds the count by the seed roster (34). | Two independent CS browser drifts. | CS filter, CS load-more |
| `crates/hts-ui/e2e/tests/value-sets.spec.ts` | Too-costly test clears the count input (`fill("")`) before submit; added a rationale comment referencing the `if req.count.is_none()` gate. | HTS's `HTS_MAX_EXPANSION_SIZE` guard is only active when `count` is omitted from the request. The workbench form defaults `count=50`, which would otherwise bypass the banner even against a 60-concept fixture. | VS:132 |

## Ring gates before Playwright rerun

- `cargo test -p helios-hts-ui --lib` — 13/0 green, including the two new/updated `upstream::tests`.
- `cargo build -p helios-hts` — clean debug rebuild, 1m03s.
- Playwright — 73/2/3 (see top).

## Known residuals (kept out of this batch)

| Test | Root cause | Why deferred |
|---|---|---|
| `concept-maps.spec.ts:139` — reverse direction radio | The Grupo C diagnosis's "unchecked-Forward change race" hypothesis did **not** reproduce: `hx-trigger="click"` did not fix the test. HTML5 spec (and Chromium's behavior) says only the newly-checked radio fires `change`, so a race between two radios wasn't the real cause. Real root cause still open — likely either the swap itself doesn't fire (e.g. Playwright `.check()` semantics on a label-wrapped radio + htmx event listener registration timing) or an unrelated pre-flight in `concept_maps.rs` short-circuits the response. | Needs a targeted network + DOM trace (`page.on("request")` + a screenshot at the point of failure) that isn't safe to interleave with template + backend edits in the same batch. The safe next step is the diagnosis's Option B (workbench-level `hx-target="#hts-cm-workbench"` + `hx-select`), which also requires updating the `translate_input` handler and its ring test. |
| `operations.spec.ts:531` — batch-validate skeleton row | Row locator resolves through the transient htmx `.htmx-swapping.htmx-added.htmx-settling` phase, and the assertion catches the row before its `hx-get` attribute is materialized. This is the `#19 batch skeleton race condition — needs route interception` item already deferred in the todo list after Grupo A. | Requires Playwright `page.route(...)` interception to hold the fan-out response until the skeleton is settled, or an assertion polling helper. Out of scope for this batch. |

## Commit plan

Single squash commit on top of `61bfc4f59`:
- Rust upstream (`u64`→`f64` + tests) — pure library / no handler contract change.
- Three detail templates — inert Metadata placeholder.
- One template — CM direction `click` (documented rationale kept even though CM:139 still red).
- Two harness files (`seed.mjs`, `boot.mjs`) — fixture + env only.
- Three spec files — assertion tightening only.

No API surface changed, no schema changed, no i18n key added, no Rust ring test needed rewriting.
