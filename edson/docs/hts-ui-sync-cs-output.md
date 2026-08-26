# Sub-CS output — HTS UI design sync

Draft, APPEND-ONLY edits for `edson/docs/hts-ui-design.md` covering the
three gaps requested by the parent (Alt E composite-id workaround,
Grupo C Metadata-slot placeholder, Grupo D too-costly count-cleared
semantics). Reviewer applies with `StrReplace`; `old_string`s are
verbatim from `hts-ui-design.md` as of this draft.

Source anchors:

- `crates/hts-ui/src/upstream.rs` L948-980 (`read_code_system` Alt E
  doc comment), L1835-1848 (`read_value_set`), L2608-2621
  (`read_concept_map`) — all three docstrings cross-reference
  `read_code_system` for the two-hop rationale.
- `crates/hts-ui/templates/pages/cs-detail.html` L160-169 and
  `crates/hts-ui/templates/pages/vs-detail.html` L131-137 — empty
  `<div id="hts-workbench-input" hidden></div>` swap target.
- `crates/hts-ui/e2e/tests/value-sets.spec.ts` L132-157 — too-costly
  spec clears `count` via `page.locator('input[name="count"]').fill("")`
  before submitting.
- `crates/hts/src/backends/sqlite/value_set.rs` L261, L322, L1116,
  L1149, L1230 and `crates/hts/src/backends/postgres/value_set.rs`
  L672-678, L5137-5143 — `HTS_MAX_EXPANSION_SIZE` ceiling is guarded
  by `if req.count.is_none()` (or by unbounded-count code paths).

---

## Edit 1: M1 Alt E in §7.3.1 (append-only bullet)

- **Location.** End of §7.3.1's implementation-notes list; append a new
  bullet immediately after the "Test-only upstream timeouts" bullet
  (`hts-ui-design.md` L906-912) and before the `### 7.4` heading (L914).
- **Rationale.** `read_code_system` / `read_value_set` /
  `read_concept_map` all execute a summary-search hop first, then a
  canonical-URL hop, to sidestep HTS's composite `{fhir_id}|{version}`
  storage id. This is the first place the divergence appears (§7.3.1
  is CS-detail), so the substantive bullet lives here; §7.4.1 gets a
  short cross-ref (Edit 1b) and §7.5.1 will get one from sub-CM.
- **old_string:**

````text
- **Test-only upstream timeouts.** `UpstreamClient::new_with_timeouts`
  exposes shorter (100 ms connect / 250 ms request) values so the
  test rings finish in seconds against a closed loopback port;
  production still uses the default 2 s / 5 s pair from
  [`UpstreamClient::new`]. Rationale: Windows' reqwest stack does not
  return `WSAECONNREFUSED` fast enough on `127.0.0.1:1` to keep the
  30-request matrix under a minute at the production defaults.

### 7.4 ValueSet browser + detail (with expand)
````

- **new_string:**

````text
- **Test-only upstream timeouts.** `UpstreamClient::new_with_timeouts`
  exposes shorter (100 ms connect / 250 ms request) values so the
  test rings finish in seconds against a closed loopback port;
  production still uses the default 2 s / 5 s pair from
  [`UpstreamClient::new`]. Rationale: Windows' reqwest stack does not
  return `WSAECONNREFUSED` fast enough on `127.0.0.1:1` to keep the
  30-request matrix under a minute at the production defaults.
- **Two-hop-search reads (Alt E, Phase 3b, `b639ed858`).**
  `UpstreamClient::read_{code_system, value_set, concept_map}` first
  runs a summary search (`_count=1000`) filtered by the URL-safe base
  id, then re-GETs by canonical URL to sidestep HTS's composite
  `{fhir_id}|{version}` storage id — the UI never exposes composite
  ids in routes or templates. Root cause: `POST /import` populates
  the normalized terminology tables but not `helios-persistence`, so
  `GET /{Type}/{id}` 404s for imported resources; the two-hop lookup
  keeps `/ui/hts/*/{id}` working without touching the HTS backend.
  First-hop NotFound still surfaces as `UpstreamError::NotFound`,
  honoring invariant #5's outcome-in-shell contract. See
  `crates/hts-ui/src/upstream.rs::read_code_system` (L948-980) for
  the anchoring doc comment; `read_value_set` and `read_concept_map`
  reuse the same helper (`resolve_canonical_url` +
  `fetch_by_url`).

### 7.4 ValueSet browser + detail (with expand)
````

---

## Edit 1b: M1 Alt E cross-ref in §7.4.1 (append-only bullet)

- **Location.** End of the "Implementation notes discovered while
  landing Slice C" list in §7.4.1 (`hts-ui-design.md` L1095-1105)
  and before the `### 7.5` heading (L1107).
- **Rationale.** Parent prompt: "Add a short cross-ref in §7.4.1 …
  (Sub-CM handles §7.5.1's cross-ref separately)." VS reads share
  the pattern documented in §7.3.1 verbatim, so the cross-ref is
  intentionally light-touch — one bullet pointing at §7.3.1.
- **old_string:**

````text
- **Mock-upstream ready-probe.** `tests/value_sets.rs` uses an in-
  process axum mock (bound on `127.0.0.1:0`) for the flows that pin
  HTTP-level behavior of the outgoing request (tree/flat parameter
  mapping, `X-TOO-COSTLY-THRESHOLD` header, 422 too-costly). On the
  Windows current-thread `#[tokio::test]` runtime the spawned
  `axum::serve` task can trail the first client request by several
  milliseconds; `start_mock` therefore includes a `/__mock_ready` probe
  route and polls it (10 ms interval, 2 s deadline) before returning
  the base URL, so client-side timeouts stay tight without producing
  phantom `Connect` failures. Closed-loopback tests (127.0.0.1:1) keep
  the tight 100 ms / 250 ms envelope from §7.3.1.

### 7.5 ConceptMap browser + detail (with translate)
````

- **new_string:**

````text
- **Mock-upstream ready-probe.** `tests/value_sets.rs` uses an in-
  process axum mock (bound on `127.0.0.1:0`) for the flows that pin
  HTTP-level behavior of the outgoing request (tree/flat parameter
  mapping, `X-TOO-COSTLY-THRESHOLD` header, 422 too-costly). On the
  Windows current-thread `#[tokio::test]` runtime the spawned
  `axum::serve` task can trail the first client request by several
  milliseconds; `start_mock` therefore includes a `/__mock_ready` probe
  route and polls it (10 ms interval, 2 s deadline) before returning
  the base URL, so client-side timeouts stay tight without producing
  phantom `Connect` failures. Closed-loopback tests (127.0.0.1:1) keep
  the tight 100 ms / 250 ms envelope from §7.3.1.
- **Composite-id workaround (cross-ref §7.3.1 Alt E).** VS reads share
  the two-hop-search pattern documented in §7.3.1's last bullet;
  `read_value_set` sidesteps the composite `{fhir_id}|{version}`
  storage id the same way `read_code_system` does, so detail URLs
  stay `/ui/hts/value-sets/{fhir_id}` without ever surfacing the
  composite id in the URL bar or in template context.

### 7.5 ConceptMap browser + detail (with translate)
````

---

## Edit 2a: M7 Metadata slot in §7.3

- **Location.** §7.3 (CS detail) bullet list, between the **HTMX**
  bullet (`hts-ui-design.md` L821-823) and the **States** bullet
  (L824-826). Adjacent to HTMX because the placeholder is a direct
  consequence of the `hx-swap="outerHTML"` contract for the tab
  targets.
- **Rationale.** `cs-detail.html` L160-169 keeps an empty
  `<div id="hts-workbench-input" hidden></div>` inside the
  `CsTab::Metadata` arm; without it, the Lookup / Validate / Subsumes
  tab clicks (all `hx-target="#hts-workbench-input"`,
  `hx-swap="outerHTML"`) have no target on the Metadata landing and
  the swap silently no-ops. The template itself calls this out in a
  comment referencing "Playwright group C tab-click failures".
- **old_string:**

````text
- **HTMX** — full page on hard nav; each tab body `hx-get`s the workbench
  input partial with `?op=<op>&resource={id}`. Submit swaps the result
  region; `_format=json` echo panel is a sibling fragment.
- **States** — 404 soft-deleted → 200 rendering with an explanatory
  OperationOutcome partial; redirected canonical URL surfaces a
  "supersedes/superseded-by" note when the resource carries those extensions.
````

- **new_string:**

````text
- **HTMX** — full page on hard nav; each tab body `hx-get`s the workbench
  input partial with `?op=<op>&resource={id}`. Submit swaps the result
  region; `_format=json` echo panel is a sibling fragment.
- **Metadata slot placeholder (Grupo C, `8d56eac6a`).** The Metadata
  landing tab includes an empty
  `<div id="hts-workbench-input" hidden></div>` placeholder inside
  the `CsTab::Metadata` arm of `templates/pages/cs-detail.html`.
  Operation-tab clicks (Lookup / Validate / Subsumes) all fire
  `hx-target="#hts-workbench-input"` + `hx-swap="outerHTML"`;
  without this element on the Metadata landing the swap has no
  target and silently no-ops, breaking first-click tab activation.
- **States** — 404 soft-deleted → 200 rendering with an explanatory
  OperationOutcome partial; redirected canonical URL surfaces a
  "supersedes/superseded-by" note when the resource carries those extensions.
````

---

## Edit 2b: M7 Metadata slot in §7.4

- **Location.** §7.4 (VS detail) bullet list, inserted directly
  before the **States** bullet (`hts-ui-design.md` L985-990). §7.4
  has no dedicated **HTMX** bullet, so pre-States is the closest
  analogue to the §7.3 insertion; keeps ordering parallel across
  detail sections and safely avoids overlap with Edit 3's rewrite
  of the too-costly bullet just above it.
- **Rationale.** `vs-detail.html` L131-137 mirrors CS-detail: an
  empty `<div id="hts-workbench-input" hidden></div>` sits inside
  the `VsTab::Metadata` arm so the Expand tab's
  `hx-target="#hts-workbench-input"` + `hx-swap="outerHTML"` has
  something to swap. Same Grupo C fix / same commit.
- **old_string:**

````text
- **States**
  - Loading — spinner in results panel; controls stay enabled.
  - Empty expansion — neutral "no members" (not an error).
  - Filter-no-match — keeps expansion metadata, empties row region.
  - 422 too-costly — banner (`role="status"`) + per-request threshold
    form (see above).
````

- **new_string:**

````text
- **Metadata slot placeholder (Grupo C, `8d56eac6a`).** The Metadata
  landing tab includes an empty
  `<div id="hts-workbench-input" hidden></div>` placeholder inside
  the `VsTab::Metadata` arm of `templates/pages/vs-detail.html`.
  The Expand tab fires `hx-target="#hts-workbench-input"` +
  `hx-swap="outerHTML"`; without this element on the Metadata
  landing the swap has no target and silently no-ops, mirroring
  the §7.3 CS-detail contract.
- **States**
  - Loading — spinner in results panel; controls stay enabled.
  - Empty expansion — neutral "no members" (not an error).
  - Filter-no-match — keeps expansion metadata, empties row region.
  - 422 too-costly — banner (`role="status"`) + per-request threshold
    form (see above).
````

---

## Edit 3: M8 too-costly count-cleared in §7.4

- **Location.** Extends the existing `- **`too-costly` control.**`
  bullet in §7.4 (`hts-ui-design.md` L975-984); appended as an
  inline "HTS-side gate semantics" clause at the tail of the same
  bullet so the divergence stays with the UI-side threshold rules
  rather than living in a separate bullet.
- **Rationale.** `HTS_MAX_EXPANSION_SIZE` (default 3500) is
  enforced only when `$expand` omits `count`
  (`crates/hts/src/backends/sqlite/value_set.rs` L261, L322,
  L1116, L1149, L1230 all gate on `if req.count.is_none()`;
  Postgres in `crates/hts/src/backends/postgres/value_set.rs`
  L672-678 / L5137-5143 uses the same-message unbounded-count
  path). The workbench form defaults `count=50`, so submitting
  without editing bypasses the ceiling — the e2e too-costly spec
  (`crates/hts-ui/e2e/tests/value-sets.spec.ts` L132-157) clears
  the input via `fill("")` before submit and calls this out in a
  block comment. Current doc mentions the threshold but does not
  spell out this bypass.
- **old_string:**

````text
- **`too-costly` control.** Both the banner action ("Raise") and the
  Advanced `<details>` numeric input write to a single per-request hidden
  form field named `threshold`; the value echoes on the next Expand
  submit. There is no cookie or session store. Values above the build-
  time ceiling `HTS_UI_MAX_EXPANSION_SIZE_HINT` render a warning and are
  NOT attached as the `X-TOO-COSTLY-THRESHOLD` request header — the
  operator sees the ceiling in a `<details>`-hover tooltip and can edit
  operator config to raise it. See §7.6 for the shared threshold
  contract; the "session-scoped" language in that section is superseded
  by this per-request rule.
````

- **new_string:**

````text
- **`too-costly` control.** Both the banner action ("Raise") and the
  Advanced `<details>` numeric input write to a single per-request hidden
  form field named `threshold`; the value echoes on the next Expand
  submit. There is no cookie or session store. Values above the build-
  time ceiling `HTS_UI_MAX_EXPANSION_SIZE_HINT` render a warning and are
  NOT attached as the `X-TOO-COSTLY-THRESHOLD` request header — the
  operator sees the ceiling in a `<details>`-hover tooltip and can edit
  operator config to raise it. See §7.6 for the shared threshold
  contract; the "session-scoped" language in that section is superseded
  by this per-request rule.

  **HTS-side gate semantics.** `HTS_MAX_EXPANSION_SIZE` (default 3500)
  is enforced ONLY when the `$expand` request omits `count` — every
  in-memory and cached-implicit expansion path in
  `crates/hts/src/backends/sqlite/value_set.rs` (L261, L322, L1116,
  L1149, L1230) and the analogous branches in
  `crates/hts/src/backends/postgres/value_set.rs` (L672-678,
  L5137-5143) gate on `if req.count.is_none()`. The workbench form
  defaults `count=50`, so on a normal submit the request is
  page-bounded and the ceiling is bypassed. To trip the guard from
  the UI a user must clear the `count` input (or the request path
  must omit the parameter entirely); the e2e too-costly spec
  (`crates/hts-ui/e2e/tests/value-sets.spec.ts` L132-157) does
  exactly this — `page.locator('input[name="count"]').fill("")`
  before clicking Run. Only then does HTS return the 422 with the
  too-costly `OperationOutcome`, which renders the banner + Raise-
  threshold form that re-issues an expand call with an explicit
  `count`.
````

---

## Summary

| Edit | Section | Kind             | Bullet lines (new_string body) |
|------|---------|------------------|--------------------------------|
| 1    | §7.3.1  | Append           | ~13                            |
| 1b   | §7.4.1  | Append cross-ref | 6                              |
| 2a   | §7.3    | Insert           | 8                              |
| 2b   | §7.4    | Insert           | 8                              |
| 3    | §7.4    | Extend           | +17 (added paragraph)          |

**Apply order note (for the reviewer):** Edit 2b and Edit 3 both live
in §7.4 but their `old_string`s are disjoint — Edit 3 rewrites the
too-costly bullet (ending at "…by this per-request rule."), Edit 2b
inserts before "- **States**" using the States sub-list as its
anchor. Either can be applied first.

**Sizing note:** Edit 1 and Edit 3 exceed the 8-line soft guidance
because the underlying finding is substantive (Alt E covers three
methods, a rationale, and a NotFound contract; the too-costly gate
needs the source cross-refs, the default-count observation, and the
e2e spec citation). Trim on review if desired; further shortening
would drop source coordinates the reviewer likely wants.

**No concerns.** All `old_string` blocks were copied verbatim from
the current `hts-ui-design.md` line ranges cited above. The template
placeholders exist in the repo today (Grupo C fix already landed);
the two-hop-search implementation is live in `upstream.rs` with an
in-source doc comment that anchors the Alt E language; the HTS-side
`if req.count.is_none()` gate is present in both SQLite and Postgres
backends. No writes were made to `hts-ui-design.md` itself.
