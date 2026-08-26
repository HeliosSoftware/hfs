# Grupo C (Detail-page tab click) — Diagnosis

## Root cause

**Shared root for the three tab-click tests (CS:65, VS:81, CM:91):** On the Metadata landing tab, the workbench region renders only a hint paragraph — there is no `#hts-workbench-input` element in the DOM. Operation tabs (Lookup / Expand / Translate) declare `hx-target="#hts-workbench-input"` + `hx-swap="outerHTML"`, but htmx cannot find that target on the initial page load, so the GET succeeds server-side yet **no client swap occurs** and the Run / Translate button never appears. Direct navigation to `/lookup`, `/expand`, or `/translate` works (those tests pass) because the full-page render includes the input partial with the correct id.

Button labels and Fluent keys are **not** the problem: detail embeds use `hts-workbench-run` → "Run" (CS/VS) and `hts-cm-translate-submit` → "Translate" (CM), matching the Playwright selectors.

## Files touched (list every relevant file)

- `crates/hts-ui/templates/pages/cs-detail.html`: CS tab strip + Metadata workbench slot (missing swap anchor)
- `crates/hts-ui/templates/pages/vs-detail.html`: VS tab strip + Metadata workbench slot
- `crates/hts-ui/templates/pages/cm-detail.html`: CM tab strip + Metadata workbench slot
- `crates/hts-ui/templates/partials/hts-cs-lookup-input.html`: input partial (correct `#hts-workbench-input` + Run button — no change needed)
- `crates/hts-ui/templates/partials/hts-vs-expand-input.html`: input partial (correct — no change needed)
- `crates/hts-ui/templates/partials/hts-cm-translate-input.html`: input partial + direction-toggle radios (separate CM:139 race)
- `crates/hts-ui/src/code_systems.rs`: GET tab handlers return input partial on `HX-Request` (server side OK; client target missing)
- `crates/hts-ui/src/value_sets.rs`: same
- `crates/hts-ui/src/concept_maps.rs`: same + pre-flight validation for reverse `targetCode`
- `crates/hts-ui/e2e/tests/code-systems.spec.ts`: failing tab-click assertion at :65
- `crates/hts-ui/e2e/tests/value-sets.spec.ts`: failing tab-click at :81; unrelated fixture gap at :132
- `crates/hts-ui/e2e/tests/concept-maps.spec.ts`: failing tab-click at :91; direction-toggle test at :139
- `crates/hts-ui/e2e/seed.mjs`: missing `ex-vs-too-costly` fixture (VS:132 only)
- `crates/hts-ui/e2e/tests/operations.spec.ts`: reference — standalone workbench always renders `#hts-workbench-input` in the shell, so the same hx-target contract works there
- `locales/en/main.ftl`: confirms `hts-workbench-run = Run`, `hts-cm-translate-submit = Translate`, `hts-cm-translate-target-code = Target code`
- `playwright-group-a2.log`: failure evidence (Run/Translate not found after tab click; Target code not found after Reverse check)

## Fix strategy

**Pick: Option B — align operation tabs with the Metadata tab’s workbench-level htmx swap**, plus two targeted side fixes.

| Option | Summary | Verdict |
|--------|---------|---------|
| A | Add `href` / `hx-trigger` on tabs | Tabs already have real `<a href>`; trigger is not the issue |
| B | Operation tabs use `hx-target="#hts-{cs\|vs\|cm}-workbench"` + `hx-select` (mirror Metadata tab); optionally stop returning input-only partials for tab GETs | **Chosen** — fixes missing DOM cleanly, removes hint+form overlap, swaps in result slot |
| C | Empty `#hts-workbench-input` placeholder on Metadata tab only | Smaller diff but leaves hint visible beside form; still omits result slot until POST |
| D | Change Playwright to `page.goto(…/lookup)` instead of tab click | Masks the htmx contract bug |

**Justification:** Metadata tab already uses `#hts-cs-workbench` + `hx-select` successfully. Operation tabs should use the same pattern instead of targeting an id that only exists after a hard navigation. The server already returns a full detail page on non-htmx GET; htmx tab clicks should receive the same document and let `hx-select` extract the workbench fragment.

**Additional fixes (not tab-click):**

- **CM:139:** Change direction radios from `hx-trigger="change"` to `hx-trigger="click"`. Both Forward and Reverse radios fire `change` when toggling; the unchecked Forward radio can win the race and re-render the forward partial (no Target code field).
- **VS:132:** Add `ex-vs-too-costly` (+ backing CodeSystem with > `HTS_MAX_EXPANSION_SIZE` concepts, default 3500) to `seed.mjs`. The fixture is documented in `e2e/README.md` and `value-sets.spec.ts` but never seeded; the expand page renders an outcome shell with no workbench form, so Run is absent.

## Exact edits

### 1. `crates/hts-ui/templates/pages/cs-detail.html` — Lookup / Validate / Subsumes tabs

**Current (Lookup tab — Validate and Subsumes are identical apart from URLs):**

```html
      <a role="tab"
         class="hts-cs-detail__tab{% if tab == CsTab::Lookup %} is-active{% endif %}"
         aria-selected="{% if tab == CsTab::Lookup %}true{% else %}false{% endif %}"
         aria-controls="hts-cs-workbench"
         href="/ui/hts/code-systems/{{ id }}/lookup"
         hx-get="/ui/hts/code-systems/{{ id }}/lookup"
         hx-target="#hts-workbench-input"
         hx-swap="outerHTML">
        {{ chrome.i18n.t("hts-cs-detail-tab-lookup") }}
      </a>
```

**Replacement:**

```html
      <a role="tab"
         class="hts-cs-detail__tab{% if tab == CsTab::Lookup %} is-active{% endif %}"
         aria-selected="{% if tab == CsTab::Lookup %}true{% else %}false{% endif %}"
         aria-controls="hts-cs-workbench"
         href="/ui/hts/code-systems/{{ id }}/lookup"
         hx-get="/ui/hts/code-systems/{{ id }}/lookup"
         hx-target="#hts-cs-workbench"
         hx-select="#hts-cs-workbench"
         hx-swap="outerHTML">
        {{ chrome.i18n.t("hts-cs-detail-tab-lookup") }}
      </a>
```

Apply the same `hx-target` / `hx-select` change to the Validate and Subsumes tab anchors (lines ~141–153).

### 2. `crates/hts-ui/templates/pages/vs-detail.html` — Expand tab

**Current:**

```html
      <a role="tab"
         class="hts-vs-detail__tab{% if tab == VsTab::Expand %} is-active{% endif %}"
         ...
         hx-get="/ui/hts/value-sets/{{ id }}/expand"
         hx-target="#hts-workbench-input"
         hx-swap="outerHTML">
```

**Replacement:**

```html
         hx-get="/ui/hts/value-sets/{{ id }}/expand"
         hx-target="#hts-vs-workbench"
         hx-select="#hts-vs-workbench"
         hx-swap="outerHTML">
```

### 3. `crates/hts-ui/templates/pages/cm-detail.html` — Translate tab

**Current:**

```html
         hx-get="/ui/hts/concept-maps/{{ id }}/translate"
         hx-target="#hts-workbench-input"
         hx-swap="outerHTML">
```

**Replacement:**

```html
         hx-get="/ui/hts/concept-maps/{{ id }}/translate"
         hx-target="#hts-cm-workbench"
         hx-select="#hts-cm-workbench"
         hx-swap="outerHTML">
```

### 4. Tab GET handlers — return full detail page for htmx tab loads

Each `*_input` handler currently short-circuits on `HX-Request` and returns only the input partial. Tab clicks (with workbench-level `hx-select`) need the full detail page. **Remove the `if is_htmx { return partial }` block** for tab-route GET handlers; always call `render_detail_with_tab`.

**Example — `crates/hts-ui/src/code_systems.rs` `lookup_input`:**

**Current:**

```rust
    let summary = state.upstream.read_code_system(&id).await.ok();
    if is_htmx {
        return render(
            LookupInputTemplate {
                chrome,
                id,
                summary,
            }
            .render(),
        );
    }
    render_detail_with_tab(&state, chrome, id, CsTab::Lookup, summary).await
```

**Replacement:**

```rust
    let summary = state.upstream.read_code_system(&id).await.ok();
    render_detail_with_tab(&state, chrome, id, CsTab::Lookup, summary).await
```

Repeat for `validate_input`, `subsumes_input` (`code_systems.rs`), `expand_input` (`value_sets.rs`), and `translate_input` (`concept_maps.rs`) **only if** direction-toggle radios are fixed to keep using input-level partial responses (see edit 5). If direction toggle also moves to workbench-level swap, `translate_input` can drop the partial branch entirely.

**Update Rust ring tests** that assert partial-only htmx responses (`lookup_input_hx_renders_input_partial_only`, `expand_tab_htmx_returns_input_partial_only`, `translate_tab_htmx_returns_input_partial_only`) to assert the workbench fragment is present inside a full-page response instead.

### 5. `crates/hts-ui/templates/partials/hts-cm-translate-input.html` — direction toggle race (CM:139)

**Current (both radios):**

```html
             hx-get="/ui/hts/concept-maps/{{ id }}/translate?direction=forward"
             hx-target="#hts-workbench-input"
             hx-swap="outerHTML"
             hx-trigger="change">
```

**Replacement:**

```html
             hx-get="/ui/hts/concept-maps/{{ id }}/translate?direction=forward"
             hx-target="#hts-workbench-input"
             hx-swap="outerHTML"
             hx-trigger="click">
```

Apply `hx-trigger="click"` to **both** Forward and Reverse radios (lines ~63–75). Only the clicked radio fires; the unchecked Forward radio no longer re-fetches the forward partial over the reverse one.

*(Optional hardening: also switch direction radios to `#hts-cm-workbench` + `hx-select` and drop the partial branch in `translate_input`, same as tab fix.)*

### 6. `crates/hts-ui/e2e/seed.mjs` — `ex-vs-too-costly` fixture (VS:132)

**Add after `ex-vs-tree` block (~line 151):**

```javascript
  // -- ex-cs-huge: backs ex-vs-too-costly; default HTS_MAX_EXPANSION_SIZE=3500.
  const hugeConcepts = [];
  for (let i = 1; i <= 3600; i++) {
    hugeConcepts.push({ code: `big-${i}`, display: `Big ${i}` });
  }
  entries.push({
    resource: {
      resourceType: "CodeSystem",
      id: "ex-cs-huge",
      url: "http://example.org/cs/huge",
      version: "1.0.0",
      name: "ExampleHugeCS",
      status: "active",
      content: "complete",
      concept: hugeConcepts,
    },
  });

  entries.push({
    resource: {
      resourceType: "ValueSet",
      id: "ex-vs-too-costly",
      url: "http://example.org/vs/too-costly",
      version: "1.0.0",
      name: "ExampleTooCostlyVS",
      status: "active",
      compose: {
        include: [{ system: "http://example.org/cs/huge" }],
      },
    },
  });
```

## Which tests each edit fixes

| Edit | Tests fixed |
|------|-------------|
| cs-detail.html tab hx-target/hx-select | `code-systems.spec.ts:65` |
| vs-detail.html tab hx-target/hx-select | `value-sets.spec.ts:81` |
| cm-detail.html tab hx-target/hx-select | `concept-maps.spec.ts:91` |
| `*_input` handlers → full page on htmx tab GET | Required companion to edits 1–3 (otherwise `hx-select` finds nothing) |
| cm-translate-input.html `hx-trigger="click"` | `concept-maps.spec.ts:139` |
| seed.mjs `ex-vs-too-costly` | `value-sets.spec.ts:132` |

## Special case: concept-maps.spec.ts:139 (reverse without targetCode)

**Not the same root cause as the tab-click trio.**

- The test navigates directly to `/ui/hts/concept-maps/ex-cm-1/translate` (form is present; `#hts-workbench-input` exists).
- Failure in `playwright-group-a2.log`: `getByLabel('Target code')` not found **after** `getByLabel('Reverse').check()`.
- The forward partial is still showing because **both** direction radios use `hx-trigger="change"`. Unchecking Forward fires a GET `?direction=forward`; checking Reverse fires `?direction=reverse`. Whichever response arrives last wins — Forward often wins, leaving Code/System fields and no Target code label.
- Pre-flight validation (`validate_pre_flight` → `.hts-outcome--error`) is implemented correctly in `concept_maps.rs` and covered by `translate_reverse_without_target_code_renders_inline_validation_outcome_without_posting_to_hts` in the Rust ring; the e2e test never reaches the submit click because the reverse form never renders.

Fix: edit 5 (`hx-trigger="click"`). No change to validation logic or outcome partial.

## Special case: value-sets.spec.ts:132 (too-costly banner)

**Not the same root cause as VS:81.**

- Direct `page.goto("/ui/hts/value-sets/ex-vs-too-costly/expand")` — not a tab click.
- `ex-vs-too-costly` is documented in `e2e/README.md` and the spec header but **absent from `seed.mjs`**. The detail template wraps tabs/workbench in `{% if let Some(summary) = self.summary() %}`; unknown id → outcome partial only → no `#hts-workbench-input`, no Run button → 30 s timeout on click.
- Fix: edit 6 (seed fixture). No template change required once the VS exists.

## Confidence & risks

| Area | Assessment |
|------|------------|
| Tab-click diagnosis | **High.** Metadata workbench HTML lacks `#hts-workbench-input`; Metadata tab uses workbench-level swap; direct-URL tests pass; log shows element not found after tab click. |
| CM:139 diagnosis | **High.** Log pinpoints Target code after Reverse check; dual `change` trigger race is reproducible by inspection. |
| VS:132 diagnosis | **High.** Seed grep confirms fixture missing; same “Run not found” symptom, different entry path. |
| JS ring / htmx timing | Low risk for tab fix — swap target exists before click; server response is synchronous HTML. |
| nojs fallback | Unaffected — tab `<a href>` still hard-navigates to the operation URL. |
| operations.spec.ts | Unaffected — standalone shell always includes `#hts-workbench-input`; op-selector uses `hx-target="body"`. |
| Rust unit tests | **`lookup_input_hx_renders_input_partial_only` and siblings must be updated** if handlers stop returning input-only fragments for tab GETs. Direction-toggle partial contract can remain if radios keep `#hts-workbench-input` target. |
| Seed size | `ex-cs-huge` adds 3600 concepts to the import bundle (~few hundred KB JSON). Acceptable for e2e; alternative is lowering `HTS_MAX_EXPANSION_SIZE` in `boot.mjs` but that risks breaking `ex-vs-1` expand success tests. |
| Tab `aria-selected` after htmx | Still stale after workbench-only swap (pre-existing Metadata-tab behavior). Out of scope for Grupo C; tests do not assert tab active state post-click. |

## Hypothesis checklist

| Hypothesis | Result |
|------------|--------|
| H1. Tab `hx-get` does not fire | **Rejected** — click reaches the `<a role="tab">`; server partial is fine on direct HX-Request tests. Failure is missing swap **target**. |
| H2. Response target/label mismatch | **Partially rejected for labels** (Fluent resolves correctly). **Confirmed for target id** — `#hts-workbench-input` absent on Metadata landing. |
| H3. Playwright clicks non-interactive inner span | **Rejected** — tabs are flat `<a>` text nodes (unlike Grupo A op-selector `role="tab"` issue). |
| H4. Missing seed fixtures | **Confirmed for VS:132** (`ex-vs-too-costly`). **Rejected for tab-click tests** — `ex-cs-1`, `ex-vs-1`, `ex-cm-1` are seeded and Metadata landing tests pass. |
