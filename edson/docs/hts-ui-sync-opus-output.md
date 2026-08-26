# Sub-Opus findings for HTS UI design doc sync

**Summary:** 5 findings across §2/§4/§8/Appendix A, breakdown: P0=4 P1=1 P2=0.

All findings target `edson/docs/hts-ui-design.md` and address drift item D2
(§7.6.1 F1 = D locked batch transport to client-side polling; the four
call-out sections still describe it as `hx-swap-oob` streaming). Authoritative
sources: §7.6.1 F1 = D (design doc L1337, L1545, L1642), `crates/hts-ui/src/operations.rs`
(`BatchJobs` fan-out), `crates/hts-ui/src/upstream.rs` L2806
(`HTS_UI_BATCH_FANOUT_CONCURRENCY = 8` compile-time const), and
`crates/hts-ui/templates/partials/hts-op-batch-*.html` (per-row `hx-trigger="load"`
skeleton rows, `hx-swap="outerHTML"`, self-terminating `<progress>` endpoint,
zero `hx-swap-oob` attributes).

---

## §2 — Locked scope decisions

### F-Sync-1 — §2.5 batch transport still describes OOB streaming

- **Section**: §2.5 "Scale — designed in" (line ~175-176)
- **Problem**: The last sentence still commits to `hx-swap-oob` per-row streaming, directly contradicting the §7.6.1 F1 = D decision (client-side polling, no OOB, no SSE, no vendored htmx extension).
- **Current text (verbatim, ≤3 lines)**:
    "Batch validation streams per-row results via `hx-swap-oob` rather than blocking on the full response."
- **Proposed text (verbatim, ≤5 lines)**:
    "Batch validation fans out per-row `$validate-code` calls upstream (semaphore-bounded by `HTS_UI_BATCH_FANOUT_CONCURRENCY = 8`) and delivers per-row results via client-side htmx polling — skeleton rows self-fetch with `hx-trigger=\"load\"` and swap `outerHTML`, plus a self-terminating progress endpoint — rather than blocking on the full response. See §7.6.1 F1 = D for the transport rationale."
- **Priority**: P0
- **Rationale**: §7.6.1 F1 = D (L1337, L1545) and `crates/hts-ui/src/operations.rs::BatchJobs`; contract-breaking for any Discussion reader who anchors on §2.5.

### F-Sync-2 — §2.1 describes `helios-hts-ui` as future work while the crate already ships

- **Section**: §2.1 "UI home — shared chrome (`helios-ui-chrome`) + `helios-hts-ui`" (line ~114-120)
- **Problem**: The Decision statement uses new-crate language for `helios-hts-ui`, but `crates/hts-ui/` is already on disk and Slice E (batch, closure, translate) has landed in it. The `helios-ui-chrome` extraction is still legitimately future work, but conflating the two hides that `helios-hts-ui` itself is not pre-implementation.
- **Current text (verbatim, ≤3 lines)**:
    "**Decision:** Extract shared chrome (layout, CSS tokens / layered stylesheet, theme, i18n scaffolding, htmx helpers, fragment/full-page dual-mode render) from `crates/ui` into a new `helios-ui-chrome` crate. Both `helios-ui` (HFS) and a new `helios-hts-ui` crate depend on it. The `hts` binary mounts `helios-hts-ui` under `/ui` the same way `hfs` mounts `helios-ui`."
- **Proposed text (verbatim, ≤5 lines)**:
    "**Decision:** Extract shared chrome (layout, CSS tokens / layered stylesheet, theme, i18n scaffolding, htmx helpers, fragment/full-page dual-mode render) from `crates/ui` into a new `helios-ui-chrome` crate (pending #543 / Phase 0). Both `helios-ui` (HFS) and `helios-hts-ui` (already at `crates/hts-ui/`, Slice E onwards) depend on it. The `hts` binary mounts `helios-hts-ui` under `/ui` the same way `hfs` mounts `helios-ui`."
- **Priority**: P1
- **Rationale**: `crates/hts-ui/Cargo.toml` exists; `crates/hts-ui/src/operations.rs` (Slice E) is in-tree. Clarifies "pre-implementation" language flagged by the scope brief without stepping into §12 phasing.

---

## §4 — Convergent patterns + design gaps

### F-Sync-3 — §4.2 item 3 still frames the batch differentiator as OOB streaming

- **Section**: §4.2 "Gaps that HTS should fill", item 3 (line ~508-509)
- **Problem**: The list frames HTS's differentiator as `hx-swap-oob` streaming — the exact transport §7.6.1 F1 = D rejected. Discussion readers scanning "what HTS does that benchmarks don't" get a directly wrong answer.
- **Current text (verbatim, ≤3 lines)**:
    "3. **`$batch-validate-code` per-row streaming UI** via `hx-swap-oob` — no benchmark offers this; HTS already has the route."
- **Proposed text (verbatim, ≤5 lines)**:
    "3. **`$batch-validate-code` per-row progressive UI** via client-side htmx polling (skeleton rows self-fetch with `hx-trigger=\"load\"`; semaphore-bounded fan-out at `HTS_UI_BATCH_FANOUT_CONCURRENCY = 8`; §7.6.1 F1 = D) — no benchmark offers this; HTS already has the route."
- **Priority**: P0
- **Rationale**: §7.6.1 F1 = D and `crates/hts-ui/src/upstream.rs` L2806; item is a headline differentiator claim, so must match shipped transport.

---

## §8 — HTMX interaction patterns

### F-Sync-4 — §8 pattern table lists an OOB variant as the batch exemplar

- **Section**: §8 pattern table (line ~1890)
- **Problem**: Row "Per-row stream" advertises `hx-swap-oob` as the mechanism for `$batch-validate-code` and points at an out-of-tree skill (`ui-design-map §7`). The shipped implementation has zero `hx-swap-oob` attributes; the row is actively misleading for anyone using this table as an HTMX cheatsheet.
- **Current text (verbatim, ≤3 lines)**:
    "| Per-row stream | `hx-swap-oob` for `$batch-validate-code` | new; see ui-design-map §7 |"
- **Proposed text (verbatim, ≤5 lines)**:
    "| Per-row polling | skeleton row `hx-trigger=\"load\" hx-get=\".../row/{i}\" hx-swap=\"outerHTML\"` + self-terminating `<progress>` poll | `crates/hts-ui/src/operations.rs::BatchJobs`, `templates/partials/hts-op-batch-*.html` (§7.6.1 F1 = D) |"
- **Priority**: P0
- **Rationale**: `crates/hts-ui/templates/partials/hts-op-batch-*.html` and §7.6.1 F1 = D; matches shipped exemplars and removes the dead OOB reference.

---

## Appendix A — Open-question traceability

### F-Sync-5 — Appendix A "Scale?" answer still cites "OOB batch rows"

- **Section**: Appendix A, "Scale?" row (line ~2167)
- **Problem**: The last bullet in the answer cell is "OOB batch rows (§2.5, §8)" — the very phrasing §7.6.1 F1 = D superseded during Slice E. Appendix A is meant to be the traceability landing page; leaving the OOB anchor here fossilises the drift for anyone answering the #551 open questions from the top of the doc.
- **Current text (verbatim, ≤3 lines)**:
    "| Scale? | Click-to-load paging, expand filters, too-costly hatch, OOB batch rows (§2.5, §8) |"
- **Proposed text (verbatim, ≤5 lines)**:
    "| Scale? | Click-to-load paging, expand filters, too-costly hatch, polled per-row batch validation (§2.5, §7.6.1 F1 = D, §8) |"
- **Priority**: P0
- **Rationale**: §7.6.1 F1 = D (L1337, L1545, L1642); this row is the canonical answer to #551's "Scale?" open question, so it must cite the resolution and F-number the way F-Sync-1/2/4 do.

---

## Structural / cross-section notes for parent triage

- **§11.2 L2087** still says "Batch validation streaming rows" as a Playwright case — same D2 drift, but §11 is off-limits per this sub-scope. Flag to the §11 subagent (or Wave 2) so the outer ring bullet ends up saying "Batch validation per-row polling rows" and cross-refs §7.6.1 F1 = D.
- **F-Sync-5 (Appendix A)** does *not* require renumbering — it is a single-row edit inside the existing table and preserves the "#551 open question → Answer" contract.
- **§4.1 "Convergent patterns worth copying"** is clean: it does not name any benchmark's OOB/SSE mechanism, so no drift there despite the scope brief pre-flagging it.
- **F-Sync-2 (§2.1)** deliberately does *not* touch the "**Phase 0 of this UI is #543 first** (see §9 and §12)" sentence on L131, since Phase 0 sequencing is §12 territory.
