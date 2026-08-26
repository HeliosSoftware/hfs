# Sub-CM output — HTS UI design sync

Draft, append-only edits for §7.5 (ConceptMap browser + detail, L1107–1163)
and §7.5.1 (Slice D implementation notes, L1164–1208) of
`edson/docs/hts-ui-design.md`. The design doc itself is read-only in this
sub-task — the applier is responsible for taking the `old_string` /
`new_string` pairs below and running them through StrReplace against the
live design doc.

**Application order matters.** Edits 1 and 2 both touch the direction-toggle
bullet in §7.5.1. Edit 1 is a narrow rename inside that bullet; Edit 2's
`old_string` reflects the **post-Edit-1** state of the bullet (i.e. the id is
already `#hts-workbench-input`). Apply Edit 1 first, then Edit 2. Edit 3 is
independent and can be applied at any time. All three preserve existing text
verbatim except at the substitution sites.

---

## Edit 1: D5 id rename `#hts-cm-workbench-input` → `#hts-workbench-input`

**Where:** §7.5.1, direction-toggle bullet (currently L1183 in the design
doc). Verified via grep: this is the only remaining occurrence of the stale
`#hts-cm-workbench-input` id in `hts-ui-design.md`. The real template
(`crates/hts-ui/templates/partials/hts-cm-translate-input.html`, form root
line ~29 plus the two direction-radio `hx-target` attributes at lines ~77
and ~87) uses `#hts-workbench-input` — the F15 rename during Slice E aligned
CS, VS, and CM detail on the shared id, so the design doc's `-cm-` variant
is a leftover.

**old_string** (single line, unique in the file):

```
  `hx-target="#hts-cm-workbench-input"` swap so flipping the toggle
```

**new_string**:

```
  `hx-target="#hts-workbench-input"` swap so flipping the toggle
```

---

## Edit 2: M2 `hx-params="none"` on direction radios

**Where:** §7.5.1, "Direction-toggle re-render" bullet (currently
L1181–1188 in the design doc). Extends the existing bullet — the current
copy already describes the `hx-get` / `hx-target` swap and the nojs
fallback, but never mentions the load-bearing `hx-params="none"` that
prevents duplicate `direction=…` query params on the wire. The extension
folds in the wire-level rationale, the two Rust ring tests that pin the
contract, and the fix commit. Rationale sourced from the template comment
block at
`crates/hts-ui/templates/partials/hts-cm-translate-input.html`
lines 50–70 and (per the plan file) commit `64889213e`; ring test names
verified in `crates/hts-ui/tests/concept_maps.rs`:

- `translate_tab_htmx_returns_input_partial_only` — line 534
- `translate_input_hx_reverse_direction_renders_target_code` — line 574

**old_string** (whole bullet, post-Edit-1 state — assumes Edit 1 already
renamed the id):

```
- **Direction-toggle re-render.** The direction radios carry an
  `hx-get="/ui/hts/concept-maps/{id}/translate?direction=…"` +
  `hx-target="#hts-workbench-input"` swap so flipping the toggle
  fetches the appropriate source-group partial (forward: system/code/
  display; reverse: targetCode). This keeps the field set A11y-clean
  (no `display: none` toggles on inputs that would still submit) and
  the same URL + query params work as the nojs fallback (hard-nav to
  `/translate?direction=reverse` lands the reverse form).
```

**new_string** (extended — original copy preserved verbatim through
"lands the reverse form.", then a new paragraph adds the `hx-params="none"`
wire contract):

```
- **Direction-toggle re-render.** The direction radios carry an
  `hx-get="/ui/hts/concept-maps/{id}/translate?direction=…"` +
  `hx-target="#hts-workbench-input"` swap so flipping the toggle
  fetches the appropriate source-group partial (forward: system/code/
  display; reverse: targetCode). This keeps the field set A11y-clean
  (no `display: none` toggles on inputs that would still submit) and
  the same URL + query params work as the nojs fallback (hard-nav to
  `/translate?direction=reverse` lands the reverse form).
  Both radios also carry `hx-params="none"` — this is load-bearing:
  without it, htmx serialises the trigger radio's own form value
  (`name="direction"`, currently-checked `value="reverse"`) onto the
  GET URL, which htmx appends to the literal `?direction=reverse`
  already present on `hx-get`. The wire ends up as
  `?direction=reverse&direction=reverse`, which axum's
  `Query<TranslateInputForm>` (serde_urlencoded-derived `Deserialize`)
  rejects as a duplicate scalar field (HTTP 400). htmx's default 4xx
  handler is `swap: false`, so the reverse fieldset never lands in
  the DOM and Playwright times out looking for the `targetCode`
  input. `hx-params="none"` short-circuits htmx's FormData collection
  for these two triggers so the URL is emitted verbatim. Pinned by
  two Rust ring tests in `crates/hts-ui/tests/concept_maps.rs`:
  `translate_input_hx_reverse_direction_renders_target_code` (asserts
  the reverse fetch renders the `targetCode` input) and the widened
  `translate_tab_htmx_returns_input_partial_only` (asserts the tab
  fetch returns only the input partial, not the full detail shell).
  See `edson/docs/hts-ui-cm139-diagnosis.md` for the wire trace and
  htmx-source references; fix landed in commit `64889213e`.
```

---

## Edit 3: M7 Metadata workbench slot cross-ref in §7.5

**Where:** §7.5 top-level bullet list (L1109–1119 in the design doc), added
as a new bullet immediately after the "Detail" bullet (L1110–1113) and
before "States" (L1114). Cross-refs §7.3 rather than duplicating the full
contract — Sub-CS owns the primary explanation of the shared empty-div
placeholder pattern, and CS / VS / CM detail templates all reuse the same
id. Verified in the CM template:

- `crates/hts-ui/templates/pages/cm-detail.html` line 148 —
  `<div id="hts-workbench-input" hidden></div>` inside the `CmTab::Metadata`
  arm, with the comment at lines 144–147 explaining that the Translate tab
  swaps into it via `outerHTML`.

The "Detail" bullet's opening (`- **Detail** — Tabs: Metadata | Translate.`)
is unique to §7.5 (§7.4's counterpart at L918 reads
`Tabs: **Metadata | Expand**`), so anchoring on the "Detail" bullet body
plus the following "- **States**" line is a safe unique match.

**old_string**:

```
- **Detail** — Tabs: Metadata | Translate. Translate embeds the workbench
  input scoped to the map; forward/reverse toggle; match grid columns pick
  `equivalence` **or** `relationship` from the response (never from the FHIR
  version compiled into the UI).
- **States**
```

**new_string**:

```
- **Detail** — Tabs: Metadata | Translate. Translate embeds the workbench
  input scoped to the map; forward/reverse toggle; match grid columns pick
  `equivalence` **or** `relationship` from the response (never from the FHIR
  version compiled into the UI).
- **Metadata workbench slot** — the CM detail template includes the same
  empty `<div id="hts-workbench-input" hidden></div>` placeholder in the
  Metadata landing as CS and VS detail, so the Translate tab can
  `hx-swap="outerHTML"` into it without a full-page nav. See §7.3 for the
  full contract; no CM-specific behavior beyond the shared pattern.
- **States**
```

---

## Concerns

- **Sequential application of Edits 1 and 2.** Both edits touch the
  same physical bullet in §7.5.1. Edit 2's `old_string` deliberately
  reflects the post-Edit-1 state (`#hts-workbench-input`, not
  `#hts-cm-workbench-input`). If the applier runs Edit 2 *before* Edit 1,
  the StrReplace will fail to match; if a single combined pass is
  preferred instead, the applier should either (a) run Edit 1 then
  Edit 2, or (b) collapse both into a single StrReplace whose
  `old_string` is the original stale-id bullet and whose `new_string`
  is Edit 2's extended body.
- **Bullet growth.** Edit 2 roughly doubles the "Direction-toggle
  re-render" bullet's length. The extended copy stays on-format
  (prose paragraph continuation, no new sub-bullets, same wrap width
  as the surrounding §7.5.1 bullets) but any reviewer scanning §7.5.1
  for section length will notice. The reason to keep it here rather
  than pushing it to `hts-ui-cm139-diagnosis.md` is that §7.5.1 is
  the invariant-locking section for Slice D and this is a Slice-D
  invariant. The diagnosis doc is already cross-referenced.
- **`hx-params` note is CM-specific.** The `hx-params="none"` pattern
  as documented is scoped to the CM direction radios (the diagnosis
  investigation was CM-only, per plan todo `tests_all_green` /
  CM:139). If future Sub-CS / Sub-VS work uncovers analogous
  duplicate-query-param bugs on CS or VS forms, this note should be
  generalised or lifted to a shared invariants section — flagged, not
  fixed.
- **Edit 3 wording matches the requested draft closely** ("the CM
  detail template includes the same empty ... placeholder in the
  Metadata landing as CS and VS detail; see §7.3 for the full
  contract") but adds one clarifying half-sentence about *why* the
  placeholder exists (Translate tab hx-swap target) so a reader hitting
  §7.5 first — without §7.3 loaded — still understands the shape. If
  the applier wants a strictly minimal one-liner, drop the "so the
  Translate tab can `hx-swap="outerHTML"` into it without a full-page
  nav" clause.
- **No re-verification of the `TranslateInputForm` type.** The
  extension asserts that the axum `Query<TranslateInputForm>`
  rejects duplicate `direction` params as an HTTP 400. This is
  quoted from the template comment block, not re-derived from the
  handler source — if `TranslateInputForm` has since been widened to
  a `Vec<TranslateDirection>` or wrapped in a tolerant deserializer,
  the wire rationale in Edit 2 would need to be revised. Not
  believed to have changed post-CM:139, but flagged.
