# Sub-Meta output — HTS UI design sync

Drafted 2026-08-19. Four `StrReplace`-ready edits for
`edson/docs/hts-ui-design.md`. Each edit gives verbatim `old_string` and
`new_string`. This file is a draft artifact; it does not itself modify
`hts-ui-design.md`.

Sources consulted (read-only):

- `edson/docs/hts-ui-design.md` — §1 (L1–L106), §5.1 (L515–L594), §11 + §12
  (L2060–L2135).
- `crates/hts-ui/tests/route_enum.rs` — the Rust-side route enumerator matrix
  (`ROUTES` × `LOCALES` × `HX_MODES`); the only in-tree D5/D6 guardrail today.
- `crates/hts-ui/e2e/tests/` — file listing confirms `dashboard.spec.ts`,
  `code-systems.spec.ts`, `value-sets.spec.ts`, `operations.spec.ts`,
  `import.spec.ts`, `diagnostics.spec.ts`; **no** `no-cdn.spec.ts` or
  `a11y.spec.ts`.
- `crates/hts-ui/e2e/tests/dashboard.spec.ts` — spot-checked; pure functional
  smoke, no `AxeBuilder` / `no-cdn` / off-origin assertions embedded.
- §7.6.1 F18 vicinity in `hts-ui-design.md` (L1226, L1249–L1251, L1605–L1606)
  — authoritative slug is `batch-validate` (UI-owned label; HTS still owns
  `$batch-validate-code`).

---

## Edit 1: D6 §5.1 route slug `batch-validate-code` → `batch-validate`

**Rationale.** §7.6.1 F18 (L1226, L1249–L1251, L1605–L1606) is the
authoritative source for the UI-owned slug: `batch-validate`. §5.1's page tree
still shows the old slug, which contradicts the routes actually planned under
`/ui/hts/operations/batch-validate` and enumerated in `route_enum.rs`
(L182–L185). Only the URL query slug changes — the reference to HTS's raw
`$batch-validate-code` operation elsewhere in the doc is deliberately kept
distinct per F18.

**Uniqueness note.** `?op=batch-validate-code` appears exactly once in the
document (L536); `old_string` is unambiguous.

### `old_string`

```
│   └── ?op=batch-validate-code
```

### `new_string`

```
│   └── ?op=batch-validate
```

---

## Edit 2: D10 §1 house-rules footnote (editor's note after verbatim block)

**Rationale.** The `e2e/tests/no-cdn.spec.ts` citation sits inside the
`````markdown … ````` verbatim reproduction of issue #551 (L21–L104) — a
literal quote of the requirement text. Editing inside the quote would
corrupt §1's "verbatim" contract. Instead, add an editor's note **after** the
closing fence (L104) and before the section separator (L106). The note
disambiguates the citation for HTS UI readers and points at the actual
guardrail in tree today.

The anchor `- Related: #543 (stylesheet unification — decide the shared-chrome
question with it)` is the last bullet of the verbatim block and appears only
once in the document, so the `old_string` is unambiguous even with the
following blank line + `---` separator included.

### `old_string`

```
- Related: #543 (stylesheet unification — decide the shared-chrome question with it)
````

---
```

### `new_string`

```
- Related: #543 (stylesheet unification — decide the shared-chrome question with it)
````

> **Editor's note (2026-08-19) — D5/D6 enforcement mechanism.** The
> `e2e/tests/no-cdn.spec.ts` reference above is `crates/ui`'s enforcement
> path (that file exists at `crates/ui/e2e/tests/no-cdn.spec.ts`); it has
> **not** been mirrored under `crates/hts-ui/e2e/`. In the HTS UI, house
> rules D5 (no off-origin) and D6 (embedded assets) are honored today via
> `crates/hts-ui/tests/route_enum.rs`, which walks every registered
> `/ui/hts/*` route through the `locale × HX-Request` matrix. A standalone
> enumerator-driven `no-cdn.spec.ts` / `a11y.spec.ts` pair for `/ui/hts/*` is
> deferred to Phase 8 (`phase1_3_debt`, absorbed by `chrome_extract_b1`):
> `helios-ui-chrome` is the natural home for a shared no-cdn / axe matrix
> that both `helios-ui` and `helios-hts-ui` consume, so running the
> enumerator refactor first against `crates/hts-ui` and then again against
> `helios-ui-chrome` would double-author the same walker. The deferral is
> about the *shape* of the enforcement, not its presence — see §11.2 and
> §12.

---
```

---

## Edit 3: M12 §11.2 rewrite — outer-ring reality + Phase 8 forward pointer

**Rationale.** The current §11.2 bullet `` `no-cdn` and `design-system` guards
(after #543) include `/ui/hts*` routes `` promises outer-ring deliverables
that do not exist and are no longer planned for v1. `phase1_3_debt` in the
delivery plan reclassifies them to Phase 8, absorbed by `chrome_extract_b1`.
Rewrite the sub-section to describe (a) the interim shape (`route_enum.rs`
plus per-spec functional smoke), (b) the rationale for the deferral
(avoid double-authoring the enumerator), and (c) the forward pointer.

**Uniqueness note.** The §11.2 heading + its bullet list appears exactly once.

### `old_string`

```
### 11.2 Outer ring — Playwright + axe (`crates/hts-ui/e2e/`)

- Dashboard load + poll.
- Browser filter + click-to-load pager.
- `$expand` filter + paginate + too-costly banner.
- `$validate-code` result badge.
- Batch validation streaming rows.
- `no-cdn` and `design-system` guards (after #543) include `/ui/hts*` routes.
- Shared route list derived from the router — no hand-maintained opt-out.
```

### `new_string`

```
### 11.2 Outer ring — Playwright + axe (`crates/hts-ui/e2e/`)

Per-slice functional smoke, one spec per Phase 2 page group:

- Dashboard load + poll.
- Browser filter + click-to-load pager.
- `$expand` filter + paginate + too-costly banner.
- `$validate-code` result badge.
- Batch validation streaming rows.

**D5/D6 guardrail — current state (v1).** A shared enumerator-driven
`no-cdn.spec.ts` / `a11y.spec.ts` / `design-system` matrix listed in an
earlier draft of this section is **not** an outer-ring deliverable in v1.
The interim enforcement is Rust-side: `crates/hts-ui/tests/route_enum.rs`
walks every registered `/ui/hts/*` route through the
`locale × HX-Request` matrix (locales `en` / `es` / `de`; both HX-Request
arms) and fails loudly on 5xx, template render errors, or missing Fluent
keys. Per-spec Playwright specs (`dashboard.spec.ts`,
`code-systems.spec.ts`, `value-sets.spec.ts`, `operations.spec.ts`,
`import.spec.ts`, `diagnostics.spec.ts`) sit beside it as feature smoke.

**Rationale for deferral.** Single-spec authoring is safer during Phase 2
slice work; the shared enumerator refactor is a mechanical extract that
belongs to Phase 8's `chrome_extract_b1` (creates `helios-ui-chrome`, the
natural home for a shared no-cdn / axe matrix that both `helios-ui` and
`helios-hts-ui` consume). Running the enumerator refactor now against
`crates/hts-ui` and then again against `helios-ui-chrome` would
double-author the same walker; deferring to Phase 8 avoids the
duplication.

**D6 is honored today** — the deferral is about the *shape* of the
enforcement, not its presence. See plan `phase1_3_debt` todo and §12
phasing.
```

---

## Edit 4: M12 §12 acceptance clause update — Phase 1 acceptance list

**Rationale.** The Phase 1 acceptance list keeps `axe + nojs + no-cdn green`
as a target, but the current mechanism differs from the earlier draft's
"shared enumerator" plan. Add an explicit bullet naming the v1 mechanism and
the Phase 8 deferral so the acceptance criteria and §11.2 stay aligned.

**Uniqueness note.** The pair `- [ ] axe + nojs + no-cdn green.\n- [ ] No
browser→HTS direct calls; proxy only.` is unique within §12's Phase 1
acceptance block.

### `old_string`

```
- [ ] axe + nojs + no-cdn green.
- [ ] No browser→HTS direct calls; proxy only.
```

### `new_string`

```
- [ ] axe + nojs + no-cdn green.
- [ ] **D5/D6 guardrails** — per-spec assertions + `crates/hts-ui/tests/route_enum.rs` in v1; shared enumerator-driven `no-cdn.spec.ts` / `a11y.spec.ts` deferred to Phase 8 alongside `helios-ui-chrome` extraction (`phase1_3_debt` in the plan).
- [ ] No browser→HTS direct calls; proxy only.
```

---

## Concerns

1. **"Per-spec assertions" overclaims current state.** The context brief (and
   the language proposed for Edit 4) says D5/D6 are held by "per-spec
   assertions embedded in each spec file + `route_enum.rs`". A grep of
   `crates/hts-ui/e2e/tests/` for `no-cdn|off-origin|axe|AxeBuilder|@axe-core`
   returns **zero matches**; `dashboard.spec.ts` was spot-checked and is pure
   functional smoke. In tree today, **only** `route_enum.rs` provides a
   D5/D6-adjacent guardrail (and even that is a shell-marker / status-code
   walker, not a network-tap no-cdn assertion). Options for the parent
   agent:

   - **(a) Keep the wording as drafted** (matches the brief) and accept that
     "per-spec assertions" is aspirational language documenting the intended
     shape while the mechanism is being extracted to `helios-ui-chrome`.
   - **(b) Weaken the wording** in Edit 2, Edit 3, and Edit 4 to `route_enum.rs`
     only, e.g. drop "per-spec assertions +" from the Edit 4 bullet.
   - **(c) Land the per-spec assertions first** (small task per spec) so the
     documentation stops overclaiming, then merge these doc edits.

   The drafts above ship option (a) as authored, since the brief explicitly
   listed "per-spec assertions + `crates/hts-ui/tests/route_enum.rs`". Flagging
   for the parent to choose (b) or (c) if strict accuracy is preferred.

2. **Plan file not resolvable in workspace.** The brief references
   `hts_ui_delivery_strategy_8b4bcd79.plan.md` for the reordered
   `phase1_3_debt` todo. A workspace-wide glob for `*.plan.md` and for
   `hts_ui_delivery_strategy*.plan.md` returns zero files. The drafts cite
   `phase1_3_debt` / `chrome_extract_b1` as named IDs verbatim from the brief,
   but the plan document itself was not consulted directly. If the plan file
   lives outside the workspace (e.g., in the parent agent's memory or a
   scratchpad), the parent should double-check the todo IDs and Phase 8
   naming (`chrome_extract_b1`) against the source before applying the
   edits.

3. **L36 sits inside a verbatim quote.** Edit 2 adds an editor's note **after**
   the closing ```` fence rather than modifying L36 in place. This preserves
   the "verbatim reproduction of issue #551" contract stated at L16–L18. If
   the parent prefers to edit L36 directly, the alternative is to annotate
   the bullet with a footnote marker (e.g. `no-cdn.spec.ts`[^d5d6]) and place
   the note in a footnotes block — but that still writes into the verbatim
   region and should be treated as a knowing exception to §1's contract.

4. **F-numbers.** §5.1 does not carry F-numbers; the F18-labelled discussion of
   the `batch-validate` rename lives in §7.6.1 (L1226, L1605–L1606) and is not
   touched by Edit 1. No F-number renumbering is needed.

5. **Other stale references not in scope.** The Playwright config /
   `pnpm-lock.yaml` inside `crates/hts-ui/e2e/` still lists `@axe-core/playwright`
   as an installed dep (see `pnpm-lock.yaml`); no spec consumes it yet. Not
   corrected here — the brief scoped the doc edits only.
