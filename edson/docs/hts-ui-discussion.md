# HTS UI — GitHub Discussion draft (ready to paste)

> **Superseded in part (2026-08-27).** The **Operations workbench page and the Home Quick-links
> strip have been removed** from the HTS UI. References to either below are historical. The
> removal orphans `$closure`, `$batch-validate-code` and ValueSet `$validate-code`; see
> [hts-ui-improvement-plan.md](hts-ui-improvement-plan.md) §1.5 for the re-homing decision.


> **Not yet published.** This file is the Phase 5 deliverable of the
> `hts_ui_delivery_strategy_8b4bcd79.plan.md` delivery plan (D1 of
> [#551](https://github.com/HeliosSoftware/hfs/issues/551)). Publishing
> is Phase 6b — a manual step performed by the maintainer **after** the
> Phase 6a branch push (`git push -u origin feat/551-hts-ui`) and
> **before** the Phase 6c PR opens. Publishing before the PR opens
> matches the #551-stated intent that the requirements write-up
> publishes "for comment before implementation" (same pattern as
> [#215](https://github.com/HeliosSoftware/hfs/discussions/215) and
> [#223](https://github.com/HeliosSoftware/hfs/discussions/223)). The
> body links to the pushed branch — no PR URL substitution is required
> at publish time.

## Publisher instructions

1. Confirm the Phase 6a branch push has completed — the branch
   [`feat/551-hts-ui`](https://github.com/HeliosSoftware/hfs/tree/feat/551-hts-ui)
   should be visible on the remote. The body below links to it directly;
   no waiting for a PR.
2. Open [github.com/HeliosSoftware/hfs/discussions/new](https://github.com/HeliosSoftware/hfs/discussions/new)
   and pick the category the maintainer uses for research /
   architecture write-ups (the reference discussions for this repo are
   [#215](https://github.com/HeliosSoftware/hfs/discussions/215) and
   [#223](https://github.com/HeliosSoftware/hfs/discussions/223); use
   whichever category those live in).
3. Set the title to the "Suggested title" line below (verbatim).
4. Copy the body verbatim into the Discussion editor. No placeholder
   substitution is required — the body links to the pushed branch and
   notes that the implementation PR is forthcoming.
5. Apply the suggested labels: `ui`, `enhancement`, `terminology`.
6. Publish the Discussion. Comment on #551 with the Discussion URL and
   tick deliverable D1 in the issue description.
7. **Optionally**, after the Phase 6c PR opens, edit the published
   Discussion to add the PR URL to the Links section for
   cross-navigation. This is optional — the Discussion is complete
   without it.

The design doc [`edson/docs/hts-ui-design.md`](hts-ui-design.md) stays
the canonical reference; this Discussion is the operator-facing summary
that surfaces the shipping shape without asking readers to read a
3,400-line document.

---

## Suggested title

```
HTS: administrative UI — requirements, design, and v1 implementation
```

## Suggested labels

`ui`, `enhancement`, `terminology`

## Body (copy everything below verbatim, then substitute the PR placeholders)

---

## TL;DR

- The Helios Terminology Server ([`crates/hts`](../../crates/hts)) shipped
  a v1 administrative console (`/ui/hts/*`) that surfaces the browse and
  operation-exercise half of `#551`'s research inputs. It follows the same
  house rules as `crates/ui` (server-rendered Askama + htmx, no SPA, no
  CDN, no browser-facing JSON API) and lives in a new sibling crate
  `helios-hts-ui` at [`crates/hts-ui`](../../crates/hts-ui).
- v1 covers §7.1 Home + §7.2/§7.3 CodeSystem + §7.4 ValueSet + §7.5
  ConceptMap + §7.6 Operations workbench (7 ops) + §7.7 Import + §7.9
  Diagnostics.
- v1 deliberately does **not** ship CRUD editors, in-app auth, a
  multi-FHIR-version selector, or the Bootstrap ledger (§7.8 — deferred
  as its own future mini-issue; needs a new HTS admin HTTP route the
  UI cannot ship without).
- **Reviewer note:** CS / VS / CM `name` / `title` search is still
  exact `=` on HTS — **not** FHIR R4 string prefix (starts-with).
  See the dedicated section below and design §7.2.1.0 for HL7
  evidence + code pointers. Out of #551 UI scope.
- The `helios-ui-chrome` extraction that was originally paired with
  [#543](https://github.com/HeliosSoftware/hfs/issues/543) is **deferred
  out of #551 scope entirely** — visual parity between HTS and HFS was
  achieved by sharing `crates/ui/assets/*` via a single `RustEmbed`
  mount instead of extracting a new crate.
- Implementation lives on branch
  [`feat/551-hts-ui`](https://github.com/HeliosSoftware/hfs/tree/feat/551-hts-ui);
  the implementation PR is opened shortly after this Discussion
  (publish-before-PR order matches the #551-stated intent that the
  requirements write-up publishes for comment before implementation).
  Full design lives at
  [`edson/docs/hts-ui-design.md`](../../edson/docs/hts-ui-design.md) (3,400+
  lines; §§ 7.1–7.9 are the per-page specs).

## Origin

Requirement from [#551](https://github.com/HeliosSoftware/hfs/issues/551),
verbatim: research what a FHIR terminology-server UI needs to offer,
then build one for HTS. Two phases — a requirements write-up informed
by the FHIR specification, tx.fhir.org, and Ontoserver, then
implementation in the same style and with the same technologies as the
existing HFS web UI. The five open questions in #551 (where the code
lives, read-only vs read/write, authentication, scale, multi-version)
are answered in §2 of the design doc; each is locked as a scope
decision before code landed.

## Locked scope decisions (from §2 of the design doc)

- **Where the UI lives (§2.1).** New crate `helios-hts-ui` at
  `crates/hts-ui/`, mounted by the `hts` binary under `/ui`. HTS and
  HFS share `crates/ui/assets/*` (CSS, theme JS, htmx, logo, Figtree
  fonts) via a `RustEmbed` mount rooted at `../ui/assets`. Askama
  templates and inline-SVG icons are copied by-value into
  `crates/hts-ui/templates/`. The original design (extract a shared
  `helios-ui-chrome` crate paired with #543) was deferred on
  2026-08-21 (§9.0) — visual parity was met by a different path
  (§14) so the extraction is now future work, not v1 blocking.
- **Read-only v1 (§2.2).** Browse + exercise. CRUD REST already exists
  on HTS; v1 does not expose write forms. Keeps the first delivery
  focused on discoverability and live documentation — the Ontoserver
  half of #551's research inputs — without shipping half-finished
  editors.
- **Authentication (§2.3).** v1 assumes deployment-level gating
  (reverse proxy, private network, mTLS) consistent with HTS's
  current zero-auth posture. No in-app login. SMART/OAuth/basic gate
  deferred to v2.
- **Multi-version (§2.4).** HTS's FHIR version is compile-time; the
  console inherits whatever version the binary was built with. No
  R3/R4/R5 selector in v1. tx.fhir.org's per-version selector is
  documented in the design doc as a valid v2 pattern.
- **Scale (§2.5).** Click-to-load pagination is the default for search
  and `$expand`. `$expand` exposes `count` / `offset` / `filter`,
  tree-vs-flat, and an `X-TOO-COSTLY-THRESHOLD` escape hatch on 422
  `too-costly`. `$batch-validate-code` is a UI-fabricated per-row
  fan-out over `POST /ValueSet/$validate-code` (semaphore-bounded by
  `HTS_UI_BATCH_FANOUT_CONCURRENCY = 8`) with client-side htmx polling
  and a self-terminating progress endpoint — no OOB, no SSE, no
  vendored htmx extension.

Explicit v1 non-goals (§2.6): no SPA / CDN / npm browser dependency /
new browser-facing JSON API; no CRUD; no direct SQL against HTS
tables; no in-app auth; no multi-version; no chrome-extraction crate.

## Information architecture (§5 of the design doc)

```text
/ui/hts                             # Home (renamed from Dashboard, §14)
/ui/hts/code-systems                # CodeSystem browser
/ui/hts/code-systems/{id}/lookup    # CS detail — operation-first landing
/ui/hts/value-sets                  # ValueSet browser
/ui/hts/value-sets/{id}/expand      # VS detail — operation-first landing
/ui/hts/concept-maps                # ConceptMap browser
/ui/hts/concept-maps/{id}/translate # CM detail — operation-first landing
/ui/hts/operations                  # Unified operation workbench
    ?op=lookup | validate-code | subsumes | expand | translate | closure | batch-validate
/ui/hts/import                      # Import (paste + file upload)
/ui/hts/diagnostics                 # /metadata, /health, /metrics
```

Detail pages **embed** the operations workbench partials — Lookup /
Validate / Subsumes on CS, Expand / Validate on VS, Translate on CM.
The standalone `/ui/hts/operations` route is the entry point for
"start from an arbitrary system/code" flows reached from the sidebar.

`/ui/hts/bootstrap` (§7.8, Bootstrap ledger) is documented in the
design doc but **not shipped in v1** — see "What v1 does not ship"
below.

## Component reuse strategy (§6 of the design doc)

Three unifications collapse what would otherwise be duplicated
templates across CS/VS/CM and seven operations:

1. **Unified operation workbench.** One page template
   `hts-operation.html` + shared submit/result fragments; the input
   panel is a partial swapped by `hx-get` per op, the result panel
   swapped on submit, and every op also renders a request URL + raw
   JSON echo panel so the console doubles as live documentation
   (#551 draft-scope item 4).
2. **Shared browser/detail scaffolds.** `hts-browser.html` and
   `hts-detail.html` render CS/VS/CM lists and detail pages from a
   single template; per-resource specialization is a small partial
   plus a Fluent-key namespace.
3. **Shared result partials.** OperationOutcome inline banners,
   degraded-upstream banners, "no results" states, and the composite
   pager are shared between the workbench, detail pages, and browsers.

## Shared-chrome extraction (§9) — deferred out of #551 scope

The design originally paired #551 with
[#543](https://github.com/HeliosSoftware/hfs/issues/543) ("unify the
stylesheet approach so pages can't diverge") and scheduled a
`helios-ui-chrome` crate extraction as a hard prerequisite. On
2026-08-21 the extraction was deferred out of #551 scope entirely
(§9.0):

- The visual-parity goal was already met via the in-place
  `RustEmbed` share of `crates/ui/assets/*` under both `/ui/*` (HFS)
  and `/ui/hts/*` (HTS) — no duplicated bytes on disk for shared
  assets.
- #543's own scope (cascade layers, one canonical component
  vocabulary `.btn` / `.data-table` / `.page-head__title`, CI
  guards against class drift) remains in-scope for `crates/ui` on
  its own timeline; both consoles inherit through the shared
  `app.css`.
- The extraction is still valid future work — layered CSS post-#543,
  slotted `{% block nav_items %}` / `{% block topbar_end %}` /
  `{% block topbar_version %}` in a shared `layouts/base.html`,
  feature-flagged `RequestTenant` / `RequestVersion` — but ships as
  its own dedicated issue when someone picks it up, with an
  Opus-advised diff and Playwright screenshot diffs across all
  `/ui` routes as prerequisites.

## Implementation phasing (§12 of the design doc)

- **Phase 0 — Shared chrome (#543) — WAIVED 2026-08-21.** Originally
  a prerequisite; waived by the §9.0 defer decision.
- **Phase 1 — HTS console v1 — SHIPPED 2026-08-20.** Home, CS/VS/CM
  browsers + detail pages, unified operation workbench (all seven
  ops), `/import` with paste + file upload, Diagnostics. Acceptance
  criteria met: all Phase 1 routes render page + fragment modes;
  strings in en/es/de with key parity; axe + nojs + no-cdn green;
  no browser→HTS direct calls (proxy only); read-only (no
  create/update/delete forms).
- **Phase 1.5 — Bootstrap ledger — DEFERRED (own future mini-issue).**
  Needs a new HTS admin HTTP route (design invariant: the UI never
  opens the DB directly). The `bootstrap_imports` table already
  persists everything the UI needs (path, content hash, size,
  mtime, languages, imported-at) — what is missing is the read
  route + feature gate + `BootstrapReader` trait method +
  optional Rehash POST. Not blocking #551 closure.
- **Phase 2 — Deferred backlog.** CRUD editors for CS/VS/CM; in-app
  auth; per-FHIR-version selector; root batch workbench;
  multi-tenant surface if HTS ever grows one; compare-versions and
  export affordances inspired by VSAC.

## Notable adjustments during implementation

Three design deviations landed as Phase 3.5 polish while the
maintainer did the manual UI walk-through (Phase 4 of the delivery
plan). Reviewers who read the design doc before Phase 3.5 will notice
these do not match §7.1 / §7.3-5 / §7.7 as originally written; the
design doc has been patched (§14 + §7.7/§7.7.1) but §7.3-5 still read
as if a Metadata tab exists:

- **Dashboard → Home (§14).** The landing route is now labeled "Home"
  (en) / "Inicio" (es) / "Startseite" (de) with a single collapsed
  `hts-nav-home` Fluent key backing both the sidebar entry and the h1,
  mirroring HFS's `nav-home` pattern.
- **Metadata tab dropped on detail pages (supersedes §7.3/§7.4/§7.5).**
  Each detail page now redirects `/{id}` → its default operation tab
  (Lookup for CS, Expand for VS, Translate for CM). Resource facts
  (id, url, version, publisher, jurisdiction, purpose, copyright)
  moved to an always-visible header block above the operation tabs.
  Rationale: the extra Metadata tab duplicated data already visible
  in the header and cost users a click on the most common workflow.
- **Import file upload shipped (§7.7 rewritten).** The original v1
  scope deferred file upload; the visual-parity pass reversed that
  and shipped a Batch-style client-side `FileReader.readAsText()`
  sink into the paste textarea, with the wire format staying
  `application/x-www-form-urlencoded` so the existing POST handler
  is untouched. Effective JSON cap ~7.5 MiB (URL-encoding overhead).

Two smaller close-out fixes landed 2026-08-21 as user-driven Phase 4
walk-through outcomes:

- **`crates/ui/assets/import.js` — file-mode submit unblocked.**
  Root cause: `applyMode()` was doing
  `textarea.disabled = isFile` when the operator picked **Upload
  file**. HTML5 skips disabled inputs on form submission, so the
  `FileReader`-populated `bundle` value never reached the server
  and every file-mode submit tripped the UI-owned pre-flight 400
  ("Paste a JSON Bundle before submitting"). Fix: switch to
  `readOnly` (blocks manual edits but preserves submission; the
  parent field `hidden` toggle already keeps it out of the
  operator's way). No backend change; the `import_run` handler
  still reads `bundle` from the urlencoded body.
- **Operations §7.6 batch-validate demo unblocked.** The seed
  had no ValueSet composing both example CodeSystems
  (`http://example.org/cs` + `http://example.org/cs/source`),
  and the demo walk-through never named a Target URL — the
  input's HTML5 `required` attribute fired the browser tooltip
  before Submit reached the server. Fix: add
  `ex-vs-batch-mixed` (`http://example.org/vs/batch-mixed`) to
  the E2E seed and rewrite `edson/docs/hts-demo.md` §3.6 to
  name it explicitly.

Full audit trail with commit SHAs lives in the delivery plan
(`hts_ui_delivery_strategy_8b4bcd79.plan.md` — Phase 3.5 section).

## Known HTS backend gap — `name` / `title` string search (for reviewers)

Call-out so reviewers do not assume the CS / VS / CM browsers already
implement FHIR string search. **This is a `crates/hts` conformance
gap, not a UI bug** — the console forwards the same query params and
renders whatever HTS returns (design invariant #5).

**FHIR expectation (HL7 R4):**

- On CodeSystem, ValueSet, and ConceptMap, `name` and `title` are
  SearchParameters of type **string**
  ([CodeSystem](https://hl7.org/fhir/R4/codesystem.html#search),
  [ValueSet](https://hl7.org/fhir/R4/valueset.html#search),
  [ConceptMap](https://hl7.org/fhir/R4/conceptmap.html#search)).
- String search **defaults to case-/accent-insensitive prefix
  (equals or starts-with)**. `:contains` matches anywhere;
  `:exact` is whole-string including case/accents
  ([FHIR R4 Search — string](https://hl7.org/fhir/R4/search.html#string);
  modifiers summary:
  [search.html#modifiers](https://hl7.org/fhir/R4/search.html#modifiers)
  — default described as “partial matches at the start of the
  string”).

**HTS today:**

- `ResourceSearchQuery` documents `name` / `title` as **exact match**
  (`crates/hts/src/types.rs`).
- CS / VS / CM SQLite (and Postgres) `search` use
  `name = ?` / `title = ?` — not `LIKE 'value%'` / starts-with, and
  not case-insensitive.
- No `:contains` / `:exact` support; modifier keys are dropped by
  the typed query extractor.

**UI stance:** browsers expose plain `?name=` / `?title=` with no
match-mode toggle, because a UI-only toggle cannot change backend
SQL (§7.2.1.1 of the design doc). Full write-up with code pointers:
[`hts-ui-design.md`](hts-ui-design.md) §7.2.1.0 and Phase 2 known
backend limitations. Fix belongs to a dedicated `helios-hts`
mini-issue, then optional UI affordances.

## What v1 does not ship

- **Bootstrap ledger (§7.8).** Needs a new HTS admin HTTP route. The
  `bootstrap_imports` SQLite/Postgres table already carries the data
  (`path`, `content_hash`, `size_bytes`, `mtime_unix`, `languages`,
  `imported_at`); what is missing is the HTTP surface. Ships as its
  own paired backend + UI mini-issue.
- **CRUD editors for CS/VS/CM.** CRUD REST already works on HTS; v1
  intentionally does not expose write forms.
- **In-app authentication.** Deployment-level gating only in v1.
- **Multi-FHIR-version selector.** HTS is single-version at
  compile time.
- **`helios-ui-chrome` crate extraction.** Deferred as future work
  (§9.0); visual parity met via the shared-assets arrangement in §14.
- **`phase1_3_debt` residual** (standalone Playwright `a11y.spec.ts`
  route-enum matrix + unifying the hand-maintained `no-cdn.spec.ts`
  ROUTES list with `crates/hts-ui/tests/route_enum.rs`). Micro-refactor;
  no external dependency; can ship as a one-file PR whenever anyone
  picks it up.

## Test rings

- `cargo test -p helios-hts-ui`: 80/0 (Rust HTTP tests — router,
  browsers, detail pages, workbench, upstream contract regressions,
  visual-parity guards, Prometheus-text parser unit tests).
- `cargo test -p helios-hts`: 30+/0 (mount regression, verifies
  `HTS_UI_ENABLED` still boots the `hts` binary clean).
- `pnpm --filter helios-hts-ui-e2e test` (Playwright + axe):
  75 passed / 0 failed / 3 skipped (39.7 s). The 3 skips are
  intentional `test.skip` calls with in-file rationale: 207
  PartialSuccess and 413 TooLarge on Import are covered end-to-end
  by the Rust ring's canned mock, and 5xx-isolation on Diagnostics
  can't be forced from a browser against a real running HTS.

## Links

- Requirement: [#551](https://github.com/HeliosSoftware/hfs/issues/551)
- Companion issue on stylesheet unification:
  [#543](https://github.com/HeliosSoftware/hfs/issues/543)
- Reference discussions (matching handling pattern):
  [#215 Validation](https://github.com/HeliosSoftware/hfs/discussions/215)
  and
  [#223 Clustered deployment](https://github.com/HeliosSoftware/hfs/discussions/223)
- Canonical design doc:
  [`edson/docs/hts-ui-design.md`](../../edson/docs/hts-ui-design.md)
- Companion API truth:
  [`edson/docs/hts-details.md`](../../edson/docs/hts-details.md)
- Booted-server walk-through:
  [`edson/docs/hts-demo.md`](../../edson/docs/hts-demo.md)
- Implementation branch:
  [`feat/551-hts-ui`](https://github.com/HeliosSoftware/hfs/tree/feat/551-hts-ui)
- Implementation PR: forthcoming — opened shortly after this Discussion
  publishes; the maintainer may edit this Discussion after PR-open to
  link the PR here for cross-navigation
- Delivery plan (Cursor user-scope):
  `hts_ui_delivery_strategy_8b4bcd79.plan.md`
- Repo skills consulted during design:
  [`.claude/skills/work-with-hts/SKILL.md`](../../.claude/skills/work-with-hts/SKILL.md),
  [`.claude/skills/work-with-ui/SKILL.md`](../../.claude/skills/work-with-ui/SKILL.md),
  [`.claude/skills/hts-api-skill/SKILL.md`](../../.claude/skills/hts-api-skill/SKILL.md).

---

*End of Discussion body. Everything below is publisher context and is
not part of the paste.*

## Publisher checklist (do this after publish)

- [ ] Discussion URL captured somewhere durable (paste into #551 as a
      comment).
- [ ] #551 deliverable **D1 — Discussion write-up** ticked in the
      issue description.
- [ ] Pass the Discussion URL back to the delivery plan owner so
      Phase 6c (`gh pr create`) can reference the Discussion URL in
      the PR body.
- [ ] After Phase 6c opens the PR (optional): edit the published
      Discussion to add the PR URL under "Implementation PR" in the
      Links section for cross-navigation. The Discussion is complete
      without this edit.
- [ ] Optionally: link the Discussion from
      [`edson/docs/hts-ui-design.md`](hts-ui-design.md) header status
      block for cross-navigation.

## When this file has to be regenerated

- After the 2026-08-21 Phase 6 restructure, this file no longer carries
  `<<PR_URL>>` / `<<PR_NUMBER>>` placeholders — the body links directly
  to the pushed branch `feat/551-hts-ui`. If the branch is renamed
  between draft and publish, update both the TL;DR bullet and the Links
  section (two occurrences) to the new branch name; do not regenerate
  the whole file.
- If a Phase 3.5-style deviation lands **after** this file was drafted
  and **before** the Discussion is published, extend the "Notable
  adjustments during implementation" section with the new bullet(s)
  before pasting. Do not silently ship a Discussion that omits a
  design-doc deviation reviewers might catch from the doc.
- If Bootstrap ledger (§7.8) or the `helios-ui-chrome` extraction
  ships before this Discussion is published (both are deferred future
  work as of the draft date), update the "What v1 does not ship"
  bullet accordingly and mention them in the TL;DR.
- After Phase 6c (PR open), the publisher **may** optionally edit the
  published Discussion to add the PR URL to the Links section
  (immediately below the "Implementation branch" link) for
  cross-navigation. This is an edit to the published Discussion, not to
  this file; the file itself stays branch-linked so it can be re-used
  as a regeneration source.
