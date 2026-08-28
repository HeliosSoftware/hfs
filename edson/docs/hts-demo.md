# HTS UI — demo guide (Phase 4 walk-through)

**Audience**: reviewer running the HTS admin UI locally against the `hts`
binary before we open the single PR for #551.

**Scope**: everything shipped in `crates/hts-ui` v1 — §7.1 Home, §7.2
CodeSystem browser, §7.3 CodeSystem detail (Lookup / Validate / Subsumes),
§7.4 ValueSet browser + `$expand`, §7.5 ConceptMap browser + `$translate`,
the concept information plane, §7.7 Import, and Capability & Conformance.
§7.8 Bootstrap ledger is deferred to Phase 8+ and is NOT part of this demo.

**Two pages that used to be in this guide are gone.** The standalone
Operations workbench (`/ui/hts/operations`) was **deleted** — that path now
returns `404`. `$closure`, `$batch-validate-code` and ValueSet
`$validate-code` are **API-only**: they are still routed and still work over
HTTP (see §4.3), they just have no UI page. And `/ui/hts/diagnostics` was
renamed to **Capability & Conformance** at `/ui/hts/capability-statement`;
the old path `308`s to the new one.

**Exit criterion** (from plan `hts_ui_delivery_strategy_8b4bcd79.plan.md`
Phase 4): reviewer says "UI OK" on every page below, or files findings
that a follow-up iteration addresses before Phase 5.

---



## 0. Prerequisites (one-time)


| Thing          | Value                                                                                             | Why                                                                                                                                    |
| -------------- | ------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| Rust toolchain | `stable-x86_64-pc-windows-gnu` (portable mingw-w64 at `C:\Users\tercere\tools\mingw64\`)          | The `stable-*-pc-windows-gnullvm` linker is not installed on this host; Phase 3b confirmed builds on `-gnu` clean.                      |
| Cargo profile  | `dev` is fine — release not needed for the demo                                                   | UI logic is not perf-sensitive.                                                                                                        |
| Seed data      | `crates/hts/terminology-data/` — 17 files, **151 MB** on disk                                     | The official terminology seed set. This is what the whole guide walks through; there is no fixture bundle any more.                     |
| Disk           | ~1 GB free for `./data/hts.db`                                                                    | The seed set expands to a persistent SQLite file, not an in-memory store.                                                               |
| Ports          | HTS listens on `HTS_SERVER_PORT` (default **8090**), host `HTS_SERVER_HOST` (default `127.0.0.1`) | Every URL below assumes `http://127.0.0.1:8090`.                                                                                       |
| Locale packs   | `locales/{en,es,de}/main.ftl` at the workspace root, compiled into the binary                     | No filesystem lookup at runtime.                                                                                                       |
| curl           | Any curl. Every command below quotes its URL in **single** quotes                                 | Single quotes keep `$lookup` from being read as a shell/PowerShell variable. On Windows PowerShell 5.1 spell it `curl.exe` — plain `curl` there is an alias for `Invoke-WebRequest`. |


If you are inside the corporate VPN and off-VPN switches, remember: HTS
itself does not talk to the internet at runtime (all data is in the local
SQLite file + embedded core packs). The `HTTP_PROXY` env in your shell does
not affect the HTS binary for `/ui/hts` requests.

---



## 1. Boot the server


### 1.1 Boot against the official seed set

```powershell
$env:HTS_BOOTSTRAP_DIR = "./crates/hts/terminology-data"
$env:HTS_UI_ENABLED    = "true"      # NOT "1" — the binary flag parses as bool
cargo run --bin hts
```

One-liner equivalent (bash / WSL):

```bash
HTS_BOOTSTRAP_DIR=./crates/hts/terminology-data HTS_UI_ENABLED=true cargo run --bin hts
```

**First boot takes a while.** It imports ~150 MB of terminology
distributions (ICD-9-CM, ICD-10-CM, MeSH, NCI Thesaurus, NDC, NUCC, UCUM,
the HL7 R4 core + terminology packages, US Core, IPS, PHIN VADS and VSAC)
into `./data/hts.db`. Let it finish — the log prints one line per file.

**Later boots are fast.** Every imported file is recorded in the
`bootstrap_imports` ledger table keyed on path, with `size_bytes` +
`mtime_unix` and a content hash as the fallback check. Unchanged files are
recognised by the cheap stat alone and skipped without being re-read; a new
file, an updated release, or a changed `HTS_IMPORT_LANGUAGES` re-triggers
import of just the affected files.

Once it is up, open [http://127.0.0.1:8090/ui/hts](http://127.0.0.1:8090/ui/hts).
Sanity-check the shell before you go anywhere:

- **Sidebar** — brand *Helios Terminology Server* + `hts v0.2.1`; a
  *Terminology* group (Home, Code Systems, Value Sets, Concept Maps), a
  *Tools* group (Import), a *Server* group (Capability & Conformance), and
  a display-only **FHIR R4** version selector pinned to the sidebar foot.
- **Topbar** — a language switcher (`English` / `Spanish` / `German`, real
  `?lang=` links so it works without JS), a light/dark theme toggle, and the
  `K` avatar.
- **Home tiles** — `Status = OK`, `backend sqlite · FHIR R4`; Uptime;
  **Loaded code systems = 1977** with `150 MiB bundled on disk`; Requests
  with an average latency read from `/metrics`.

> **The "dialect" chip is gone.** It was removed from the topbar on
> 2026-08-28 — it had shipped non-functional (a `<details>` with no options
> and no form, and the `hts_dialect` cookie its comment promised never
> existed). Real `displayLanguage` control lives in the per-operation form
> fields on the workbenches. If you still see a dialect chip, you are
> running a stale binary — rebuild.

### 1.2 Optional overrides worth knowing


| Env var                   | Default                         | What it does                                                                                                                                        |
| ------------------------- | ------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| `HTS_UI_ENABLED`          | `false`                         | Master switch that mounts `/ui/hts`. Bool-typed — use `"true"` / `"false"`, not `"1"` / `"0"`.                                                     |
| `HTS_BOOTSTRAP_DIR`       | (unset)                         | Directory of terminology distributions auto-imported on boot. Unset ⇒ empty store **and** the Home "bundled on disk" tile shows an em-dash.        |
| `HTS_SERVER_PORT`         | `8090`                          | TCP port. `HTS_SERVER_HOST` (default `127.0.0.1`) pairs with it.                                                                                    |
| `HTS_DATABASE_URL`        | `./data/hts.db`                 | The SQLite file. Delete it to force a full re-import on the next boot.                                                                              |
| `HTS_STORAGE_BACKEND`     | `sqlite`                        | `sqlite` \| `postgres`.                                                                                                                             |
| `HTS_MAX_EXPANSION_SIZE`  | `3500`                          | HTS-side ceiling applied when a `$expand` request omits `count`. Lower it to reproduce the too-costly path (§4.4).                                   |
| `HTS_UI_UPSTREAM_URL`     | `http://127.0.0.1:{port}`       | The UI always talks to HTS over HTTP; by default that is this same binary on loopback. Point it elsewhere to demo the degraded banner (§4.5).        |
| `HTS_BOOTSTRAP_BATCH_SIZE`| (see `config.rs`)               | Import batching knob for the bootstrap pass.                                                                                                         |
| `HTS_IMPORT_LANGUAGES`    | (unset)                         | BCP-47 filter applied at import. Recorded in the ledger, so changing it re-imports affected files.                                                   |


---



## 2. What the seed set gives you

Nothing to seed by hand — §1.1 did it. This section is the map you need to
navigate 20k+ resources, plus two behaviours that will otherwise look like
bugs.

### 2.1 Verify the inventory

```bash
curl -s 'http://127.0.0.1:8090/health'
curl -s 'http://127.0.0.1:8090/CodeSystem?_count=5000' | grep -o '"resourceType":"CodeSystem"' | wc -l
curl -s 'http://127.0.0.1:8090/ConceptMap?_count=5000' | grep -o '"resourceType":"ConceptMap"' | wc -l
```

On the reference box that produced this guide:

```
{"status":"ok","service":"hts","version":"0.2.1","backend":"sqlite", ... }
1977
80
```

ValueSets are the big one — **20 689** of them (`_count=100000` to count
them all). CodeSystems **1 977**, ConceptMaps **80**.

> **`Bundle.total` is the page size, not the store size.** `GET
> /CodeSystem?_count=1` answers `"total":1`; `_count=1000` answers
> `"total":1000`. Count entries yourself, or read the Home tile — the
> **Loaded code systems** number comes from
> `TerminologyCapabilities.codeSystem[]` and is the honest 1977.

> **Instance reads do not work on seeded resources.** `GET
> /CodeSystem/icd9cm` and `GET /CodeSystem/icd9cm%7C2015` both return
> `404` + an `OperationOutcome`. `_id` is ignored by search. To fetch a
> resource from the API, filter on its canonical URL — that is the path
> the UI uses too:
>
> ```bash
> curl -s 'http://127.0.0.1:8090/CodeSystem?url=http://hl7.org/fhir/narrative-status'
> ```

### 2.2 Two id shapes — and which one to use

`GET /CodeSystem` projects composite ids: `icd9cm|2015`,
`nci-thesaurus|current`, `narrative-status|4.0.1`. The browser table's row
links use the **short** form:

```
href="/ui/hts/code-systems/icd9cm"
href="/ui/hts/code-systems/nci-thesaurus"
href="/ui/hts/code-systems/narrative-status"
```

Both shapes resolve (`/ui/hts/code-systems/icd9cm%7C2015/lookup` is a 200
too), but **document and demo the short form** — that is what a reader
clicking a row actually gets.

### 2.3 Detail pages 308 to their default workbench tab

Every detail base URL permanently redirects to its first operation tab:

| You open                                        | You land on                                              |
| ----------------------------------------------- | ---------------------------------------------------------- |
| `/ui/hts/code-systems/icd9cm`                   | `/ui/hts/code-systems/icd9cm/lookup`                       |
| `/ui/hts/value-sets/immunization-status`        | `/ui/hts/value-sets/immunization-status/expand`            |
| `/ui/hts/concept-maps/sc-encounter-status`      | `/ui/hts/concept-maps/sc-encounter-status/translate`       |
| `/ui/hts/diagnostics`                           | `/ui/hts/capability-statement`                             |
| `/ui/hts/`                                      | `/ui/hts`                                                  |

All five are `308`. That is the design (§8.3 — the URL bar always names the
active operation), not a bug. The steps below give the URL you land on.

### 2.4 The fixtures this guide uses

Every id, URL and code in this table was verified live against a seeded
server before it was written down.

The **UI label** column is the row's link text, which is the resource
`name` — not its title, and never the id.

| Resource   | Route id             | UI label (Name) — link text     | Canonical URL                                                | Why the demo uses it                                                                       |
| ---------- | -------------------- | ------------------------------- | -------------------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| CodeSystem | `v3-EntityRisk`      | `v3.EntityRisk`                 | `http://terminology.hl7.org/CodeSystem/v3-EntityRisk`          | 11 concepts, `hierarchyMeaning = is-a`, `IFL → EXP` and `INF → BHZ`. Drives Lookup **and** Subsumes on §3.3. Browser row 4. |
| CodeSystem | `narrative-status`   | `NarrativeStatus`               | `http://hl7.org/fhir/narrative-status`                         | Four flat codes with definitions. The cleanest `$lookup` / `$validate-code` demo.            |
| CodeSystem | `icd9cm`             | `ICD-9-CM`                      | `http://hl7.org/fhir/sid/icd-9-cm`                             | Browser row 1. Metadata-only in this seed — good for showing the facts block, useless for `$lookup`. |
| CodeSystem | `nci-thesaurus`      | `NCIt`                          | (browser row 2)                                                | Browser row 2.                                                                               |
| CodeSystem | `mesh`               | `MeSH`                          | (browser row 3)                                                | Browser row 3.                                                                               |
| ValueSet   | `immunization-status`| `ImmunizationStatusCodes`       | `http://hl7.org/fhir/ValueSet/immunization-status`             | 3-code expansion. The smallest complete `$expand` result. Browser row 3.                     |
| ValueSet   | `v3-EntityRisk`      | `v3.EntityRisk`                 | `http://terminology.hl7.org/ValueSet/v3-EntityRisk`            | 11 members, 2 of them nested — the readable **tree mode** demo.                              |
| ValueSet   | `languages`          | `CommonLanguages`               | `http://hl7.org/fhir/ValueSet/languages`                       | 56 members ⇒ the flat pager fires at `count=50`.                                             |
| ValueSet   | `v3-ActCode`         | `v3.ActCode`                    | `http://terminology.hl7.org/ValueSet/v3-ActCode`               | 1 302 members. Use it to see tree mode refuse to page.                                       |
| ValueSet   | `contract-assettype` | `ContractResourceAssetTypeCodes`| `http://hl7.org/fhir/ValueSet/contract-assettype`              | Browser row 1.                                                                               |
| ValueSet   | `bodysite-laterality` / `timing-abbreviation` | `Laterality` / `TimingAbbreviation` | `http://hl7.org/fhir/ValueSet/{bodysite-laterality,timing-abbreviation}` | 3 and 16 members — quick contrast against the 56-member pager case. Browser rows 4 and 5. |
| ConceptMap | `sc-encounter-status`| `EncounterStatusCanonicalMap`   | `http://hl7.org/fhir/ConceptMap/sc-encounter-status`           | `encounter-status` → `resource-status`, `equivalent`. Forward **and** reverse `$translate`.  |
| ConceptMap | `sc-appointmentstatus` / `sc-episode-of-care-status` | `AppointmentStatusCanonicalMap` / `EpisodeOfCareStatusCanonicalMap` | `http://hl7.org/fhir/ConceptMap/sc-*` | Browser rows 2 and 3.                                                    |

Note that two of these canonical URLs resolve to **two** stored versions
(`v3-EntityRisk` as CodeSystem *and* as ValueSet each have a pair). Both
rows link to the same route id, and the detail page picks one — see the
`Version` red flag in §3.3.

### 2.5 Detail pages resolve the whole catalog — fixed 2026-08-28

Worth knowing about, because a demo against an older build will hit it.

Detail pages resolve a route id to a canonical URL by search-listing the
resource type and matching on the base id. Until 2026-08-28 that scan read a
single `?_count=1000` page, so anything past position 1 000 could not open
**even though the browser linked to it** — 977 of 1 977 CodeSystems and
19 689 of 20 689 ValueSets. It failed quietly: HTTP 200, with a not-found
outcome rendered inside the page shell.

It now pages with `_offset` until it finds a match. Check any of the systems
that used to be unreachable:

```bash
# ICD-10-CM sits at position 1 968 in the default order.
curl -s -L 'http://127.0.0.1:8090/ui/hts/code-systems/icd10cm' | grep -c 'not found'
# → 0

# And a ValueSet near the end of 20 689.
curl -s -L 'http://127.0.0.1:8090/ui/hts/value-sets/2.16.840.1.113762.1.4.1190.25' | grep -c 'not found'
# → 0
```

Cost scales with depth, and only for resources deep in the catalog: ~140 ms
for the first page (unchanged), ~1.35 s for the very last ValueSet of 20 689.

Two related backend behaviours are worth knowing when you write your own
queries — neither is a UI issue:

```bash
# `_id` is silently ignored: this returns an unfiltered page, not one match.
curl -s 'http://127.0.0.1:8090/CodeSystem?_id=icd10cm' | grep -o '"total":[0-9]*'

# `?url=` is the search parameter that actually narrows.
curl -s 'http://127.0.0.1:8090/CodeSystem?url=http://hl7.org/fhir/sid/icd-10-cm' | grep -o '"total":[0-9]*'
```

Also avoid a very large `_count` on ValueSet — `?_count=100000` resets the
connection on a seeded store. Page with `_offset` instead.

---



## 3. Per-page walk-through

Follow these in order. Each block is one page: what to click, what to
observe, what a green vs. red result looks like.

### 3.1 §7.1 Home — `/ui/hts`

**Steps**

1. Land on `/ui/hts` (`/ui/hts/` 308-redirects to the canonical path if you
   get the trailing slash wrong).
2. Wait 15 seconds without clicking. Watch the tiles.
3. Click the `1h` and `6h` range links under the chart, then a series chip
   (`2xx` / `4xx` / `5xx`).
4. Toggle theme via the topbar buttons. Toggle back.
5. Click **Spanish** in the topbar switcher (or append `?lang=es`).

**Expected**

- Four tiles: **Server status** (`OK`, `backend sqlite · FHIR R4`),
  **Uptime** (`hts v0.2.1 · no restarts since HH:MM UTC`), **Loaded code
  systems** (`1977`, `150 MiB bundled on disk`), **Requests** (count +
  average latency `· from /metrics`).
- Below them, a **Requests per minute** chart. Its caption says
  "Sampled while this page is open. Excludes this page's own 15 s refresh
  and /metrics scrapes." Range links are `15m` / `1h` / `6h`; the series row
  is `All` / `2xx` / `4xx` / `5xx` with live counts. These are plain links
  (`/ui/hts?window=1h&series=all`), so they are deep-linkable and work
  without JS.
- Poll after 15 s: `hx-trigger="every 15s"` refetches
  `/ui/hts/home/cards?window=…&series=…` and swaps the tiles in place. No
  layout shift, focus stays put.
- Theme toggle: instant switch. No off-origin asset load — open DevTools →
  Network; every request should be `127.0.0.1:8090`. The only four assets
  are `/ui/hts/assets/{app.css,htmx.min.js,theme.js,logo.png}`.
- `es`: tiles read *Estado del servidor* / *Tiempo activo* / *Sistemas de
  códigos cargados* / *Solicitudes*. `de` on the browser pages reads
  *Codesysteme*, *Suchen*, *Zurücksetzen*, status chips *aktiv / Entwurf /
  zurückgezogen / unbekannt*.

**Red flags**

- A poll erases focus or scrolls the page.
- Any request in DevTools points at `cdn.*`, `jsdelivr`, or `unpkg`.
- A locale key renders as `hts-dashboard-something-something` — a Fluent key
  was added on one side of the code but not the other. File it.
- The tiles show em-dashes plus a **"Terminology backend not fully
  available — The terminology server did not respond in time"** banner.
  This *does* happen transiently on a cold or loaded server: the UI's
  upstream client has a 5 s request timeout and the first
  `/metadata?mode=terminology` fetch after idle can exceed it. One refresh
  should clear it. If it does not clear, that is §4.5 territory.



### 3.2 §7.2 CodeSystem browser — `/ui/hts/code-systems`

Layout note: the filter form is a **horizontal toolbar** above the table —
four `type="search"` inputs (*Name*, *Title*, *Canonical URL*, *Version*), a
**Search** button and a **Reset** link, with a **Status** facet chip row
beneath (`Any status` / `active` / `draft` / `retired` / `unknown`, plain
links). The form fires on `input changed delay:300ms, change, submit`, so
typing is debounced at 300 ms. The results table renders
**Name · Title · URL · Version · Status** so every filter has a visible
column.

String filters map 1:1 to FHIR search parameters, so matching is **exact and
case-sensitive** — inherited from the HTS backend, no match-mode toggle.

> **UI-vs-route naming.** `icd9cm` is the route id; it does not appear as
> visible text. The link text is the resource `name` (`ICD-9-CM`), and the
> id lives only in the row's `href`. Scan the Name / Title / URL columns, or
> deep-link. See §2.2 and §2.4.

**Steps**

1. Click **Code Systems** in the sidebar (or open `/ui/hts/code-systems`).
   You should see 25 rows and a **Load more** button. Row 1 is **ICD-9-CM**,
   row 2 **NCIt**, row 3 **MeSH**, row 4 **v3.EntityRisk**.
2. Type `ICD-9-CM` into the *Name* box. Wait 300 ms — the table narrows to
   one row and the **Load more** button disappears.
3. Type `icd-9-cm` instead (lowercase). You get **no** rows and the empty
   state **"No CodeSystems match these filters"**. Case sensitivity, not a
   bug.
4. Click **Reset**. Paste `http://hl7.org/fhir/narrative-status` into
   *Canonical URL*. One row: **NarrativeStatus**.
5. Reset again. Click the **draft** status chip, then **retired**, then
   **Any status**. Each is a full page load (they are links, not JS).
6. Click **Load more** once. Rows grow to 50 and the footer's next request
   advances to `_offset=50`.

Same thing from the command line if you want to see the fragments:

```bash
curl -s 'http://127.0.0.1:8090/ui/hts/code-systems/rows?name=ICD-9-CM'
curl -s 'http://127.0.0.1:8090/ui/hts/code-systems/rows?_count=25&_offset=25'
```

**Expected**

- Debounced filter: typing does not fire a request per keystroke; only after
  300 ms of quiet does the tbody re-render.
- Load-more appends the next page below the current rows (no full re-render,
  no scroll jump). The footer is OOB-swapped (`hx-select-oob="#hts-cs-rows-foot"`)
  so the button's `_offset` advances; at the terminal page the button is
  omitted entirely.
- The footer counts what is on screen: `Showing 25 CodeSystems`,
  `Showing 1 CodeSystems`.

**Red flags**

- Load-more scrolls to the top, double-renders rows, or keeps offering
  **Load more** after every row is already visible.
- Filter clears the tbody to a spinner instead of a skeleton row.
- A visible match-mode `<select>` next to URL/Name/Title (those were rolled
  back; must not reappear without a backend plan).
- `Showing 1 CodeSystems` — the footer does not pluralise. Cosmetic; file it
  if you care.



### 3.3 §7.3 CodeSystem detail — `/ui/hts/code-systems/v3-EntityRisk/lookup`

Design doc §8.3: the resource summary is a **facts block always visible at
the top**; below it a **tab strip lists operations only** — Lookup, Validate,
Subsumes. There is no "Metadata" tab. `/ui/hts/code-systems/v3-EntityRisk`
308-redirects to `…/lookup`, so the URL bar always names the active
operation.

The facts block shows the title, a version pill, a status pill, the
description, a **Facts** row (Version · Status · Content mode · Publisher),
the **Canonical URL**, and a foldable **All CodeSystem facts** (Canonical
URL, Name, Publisher, Jurisdiction, Content mode, Concept count).

**Steps**

1. Open
   [http://127.0.0.1:8090/ui/hts/code-systems/v3-EntityRisk](http://127.0.0.1:8090/ui/hts/code-systems/v3-EntityRisk).
   You land on `/ui/hts/code-systems/v3-EntityRisk/lookup` with **Lookup**
   active. Confirm the facts block reads
   `v3 Code System EntityRisk` · `v2018-08-12` · `active`, Content mode
   `complete`, Publisher `HL7, Inc`, Canonical URL
   `http://terminology.hl7.org/CodeSystem/v3-EntityRisk`.

2. **Lookup.** Fill in:

   | Field              | Value        |
   | ------------------ | ------------ |
   | Code               | `IFL`        |
   | Version            | `2018-08-12` (pre-filled from the facts block) |
   | Display language   | *leave empty* |
   | Properties         | leave `*` checked |

   Submit. You should get:

   ```
   inflammable   IFL   [Open concept]
   system      http://terminology.hl7.org/CodeSystem/v3-EntityRisk
   Name        v3.EntityRisk
   Version     2018-08-12
   Definition  Material is highly inflammable and in certain mixtures (with air)
               may lead to explosions. Keep away from fire, sparks and excessive heat.
   Designations  en → inflammable
   Properties    child → EXP        inactive → false
   ```

   plus a foldable **Raw request and response** showing
   `Request URL http://127.0.0.1:8090/CodeSystem/$lookup` and the
   `Parameters` body.

3. Uncheck `*` and check **parent** + **child** instead. Re-run. The
   Properties panel drops `inactive` and keeps only `child → EXP`. That is
   the `property` filter reaching the server.

4. **Validate** tab. Leave mode on **code**, enter `EXP`. Submit →
   green **valid** badge, display `explosive`, `Code EXP`, `system` and
   `Version`.
   Switch mode to **coding**, leave *Coding system* at its pre-filled
   canonical URL, enter *Coding code* `EXP`. Submit → the same **valid**
   result through the `Coding` shape.

5. Still on Validate, enter `NOPE`. Submit → red **invalid** badge and
   `Message: Unknown code 'NOPE' in the CodeSystem
   'http://terminology.hl7.org/CodeSystem/v3-EntityRisk' version '…'`.

6. **Subsumes** tab. Three runs:

   | Code A | Code B | Expected badge  | Sentence                              |
   | ------ | ------ | --------------- | ------------------------------------- |
   | `IFL`  | `EXP`  | `subsumes`      | *Code A subsumes code B.*             |
   | `EXP`  | `IFL`  | `subsumed-by`   | *Code A is subsumed by code B.*       |
   | `IFL`  | `POI`  | `not-subsumed`  | *Neither code subsumes the other.*    |

   Cross-check any of them against the API:

   ```bash
   curl -s 'http://127.0.0.1:8090/CodeSystem/$subsumes?system=http://terminology.hl7.org/CodeSystem/v3-EntityRisk&codeA=IFL&codeB=EXP'
   # {"resourceType":"Parameters","parameter":[{"name":"outcome","valueCode":"subsumes"}]}
   ```

7. Second CodeSystem for contrast: open
   `/ui/hts/code-systems/narrative-status/lookup`, code `generated`. You get
   display **Generated**, `Name NarrativeStatus`, `Version 4.0.1`, the
   definition, and `inactive → false`. Same result over the API:

   ```bash
   curl -s 'http://127.0.0.1:8090/CodeSystem/$lookup?system=http://hl7.org/fhir/narrative-status&code=generated'
   ```

**Expected**

- Tab clicks swap ONLY the region under the facts block — the facts block
  stays visible above. Region-wrap contract (§8.1).
- The URL bar updates to `/{id}/{op}` on each tab click
  (`hx-push-url="true"`).
- The UI resolves the canonical URL first and then calls the **type-level**
  operation — the raw panel says `.../CodeSystem/$lookup`, not
  `.../CodeSystem/{id}/$lookup`. That is deliberate: the instance route
  misses on composite ids (§2.2).

**Red flags**

- Any "Metadata" tab visible in the tab strip (retired in §8.3; file it).
- Clicking a tab reloads the whole page (Askama base + topbar re-render).
- The facts block disappears when a different operation tab is clicked.
- **Outcome codes that read like keys.** Fixed 2026-08-28. The catalog
  carries sentences for only four issue codes (`not-found`, `invalid`,
  `too-costly`, `unknown`) and the template builds the key from whatever
  code the server sent, so anything else used to render literally as
  `hts-outcome-code-business-rule`. It now falls back to the code itself —
  you should see `business-rule`, never `hts-outcome-code-…`. If you do see
  a key, that is a regression worth filing.
- **`Concept count —`.** The facts block shows an em-dash for every seeded
  CodeSystem because HTS does not populate `CodeSystem.count`. Expected
  today; not a render bug.
- **`Version` echoed back different from what you typed.** `v3-EntityRisk`
  is stored twice (`2018-08-12` and `4.0.0`); a Validate run pinned to
  `2018-08-12` can answer `Version 4.0.0`. Backend version-selection
  behaviour, not the UI. Worth a note on the plan.



### 3.4 §7.4 ValueSet browser + `$expand` — `/ui/hts/value-sets`

The browser is the same toolbar + facet layout as §3.2, columns
**Name · Title · URL · Version · Status**. Row 1 is
**ContractResourceAssetTypeCodes**, row 2 **v3.PaymentTerms**, row 3
**ImmunizationStatusCodes**, row 4 **Laterality**, row 5
**TimingAbbreviation**.

The detail page has exactly one tab, **Expand** — ValueSet
`$validate-code` is API-only (§4.3).

**Steps**

1. Click **Value Sets** in the sidebar. Confirm 25 rows + **Load more**.
2. Open the **ImmunizationStatusCodes** row. You land on
   [http://127.0.0.1:8090/ui/hts/value-sets/immunization-status/expand](http://127.0.0.1:8090/ui/hts/value-sets/immunization-status/expand).
   The facts block reads `Immunization Status Codes` · `v4.0.1` · `draft`,
   Publisher `FHIR Project team`, Canonical URL
   `http://hl7.org/fhir/ValueSet/immunization-status`.
3. **Expand** with the defaults — *Filter* empty, *Count* `50`, *Offset*
   `0`, mode **flat**. Click Run. You should see:

   ```
   Expansion  [Flat]  urn:uuid:…
   completed          Completed           http://hl7.org/fhir/event-status
   entered-in-error   Entered in Error    http://hl7.org/fhir/event-status
   not-done           Not Done            http://hl7.org/fhir/event-status
   total 3 · offset 0
   ```

   plus an **Echoed parameters** panel: `count 50`, `offset 0`,
   `excludeNested true`, `used-codesystem
   http://hl7.org/fhir/event-status|4.0.1`, `warning-draft …`.

   Same over the API:

   ```bash
   curl -s 'http://127.0.0.1:8090/ValueSet/$expand?url=http://hl7.org/fhir/ValueSet/immunization-status'
   ```

4. **Pager.** Open `/ui/hts/value-sets/languages/expand` and Run with
   `count=50`. 56 members exist, so 50 render (`ar Arabic`, `bn Bengali`,
   `cs Czech`, `da Danish`, `de German`, …) and the footer offers a next
   page carrying `offset=50`. Click it; the last 6 render and the pager
   retires.

5. **Tree mode.** Open `/ui/hts/value-sets/v3-EntityRisk/expand`, switch the
   mode radio to **tree**, Run. 11 rows render with `EXP` indented under
   `IFL` and `BHZ` under `INF`. Footer:
   *showing full tree · 9 leaves — Tree mode returns the whole hierarchy;
   the pager is flat-mode only.* Echoed parameters swap `excludeNested true`
   for `hierarchical true`.

6. **Tree mode at scale.** Same thing on
   `/ui/hts/value-sets/v3-ActCode/expand`: 1 302 rows arrive in one
   response, footer reads *showing full tree · 1301 leaves*, and setting
   `count=25` changes nothing. That is the documented contract, but it is
   also 1 302 `<tr>` in one swap — worth knowing before you demo it on a
   projector.

7. **Filter.** Back on `languages`, type `Eng` into *Filter* and Run. The
   expansion narrows server-side to `total 9` — `en`, the eight `en-*`
   regional variants, and `bn Bengali` (the match is a case-insensitive
   substring on the display, so *B-eng-ali* qualifies).

**Expected**

- Tree rendering is an **indented `.data-table`**, not a `<ul role="tree">`.
  Each row's Code cell carries `padding-left: calc(14px + depth * 20px)`.
  Do not go looking for `role="tree"` — it was replaced.
- Flat mode paginates; tree mode does not, and says so in the footer.
- The advanced fieldset (`displayLanguage`, `activeOnly`,
  `includeDesignations`, `useSupplement`, `date`, `property`,
  `tx-resource`, `system-version`, `check-system-version`,
  `force-system-version`, `default-valueset-version`, `threshold`) is
  present and collapsed. Everything in it is echoed back in the parameters
  panel when the server honours it.

**Red flags**

- Tree/flat toggle causes the whole page to reload.
- Pager button disappears in flat mode when it should be shown (rule: hidden
  only when `expansion.total ≤ rendered rows`).
- Any ValueSet you pick expands to `total 0` — several seeded value sets
  (`observation-codes` / LOINC, `clinical-findings` / SNOMED CT,
  `all-languages`, `condition-code`) are stored as metadata only because
  their code systems are not in this seed set. Expected, not a bug. Pick a
  fixture from §2.4 instead.



### 3.5 §7.5 ConceptMap browser + `$translate` — `/ui/hts/concept-maps`

The CM browser shares the toolbar layout but has **three** text filters
(*Name*, *Title*, *Canonical URL*) — no Version — and a 5-column table:
**Name · Title · URL · Mapping · Status**. The Mapping cell stacks the
source and target on two aligned lines:

```
S: http://hl7.org/fhir/ValueSet/encounter-status
T: http://hl7.org/fhir/ValueSet/resource-status
```

There are no *Source system* / *Target system* filter inputs any more — the
earlier build advertised them while axum's `Query` extractor silently
dropped them. They were removed rather than left lying. The Mapping column
is how you eyeball direction.

**Steps**

1. Click **Concept Maps**. 80 maps exist; row 1 is
   **EncounterStatusCanonicalMap**, row 2 **AppointmentStatusCanonicalMap**,
   row 3 **EpisodeOfCareStatusCanonicalMap**.
2. Open the **EncounterStatusCanonicalMap** row → you land on
   [http://127.0.0.1:8090/ui/hts/concept-maps/sc-encounter-status/translate](http://127.0.0.1:8090/ui/hts/concept-maps/sc-encounter-status/translate).
   Facts block: `Canonical Mapping for "EncounterStatus"` · `v4.0.1` ·
   `draft`, **Groups 1**, Publisher `HL7 (FHIR Project)`, and under **All
   ConceptMap facts** a `Source` / `Target` pair naming the two ValueSets.

3. **Forward translate.** Direction stays **Forward**. Fill in:

   | Field         | Value                                    |
   | ------------- | ---------------------------------------- |
   | Source system | `http://hl7.org/fhir/encounter-status`   |
   | Source code   | `planned`                                |

   Submit. Expected:

   ```
   1 matches   [Forward]
   Code     System                                  Display  Equivalence  Origin
   planned  http://hl7.org/fhir/resource-status     —        equivalent   http://hl7.org/fhir/ConceptMap/sc-encounter-status|4.0.1
   ```

   Same over the API:

   ```bash
   curl -s 'http://127.0.0.1:8090/ConceptMap/$translate?url=http://hl7.org/fhir/ConceptMap/sc-encounter-status&system=http://hl7.org/fhir/encounter-status&code=planned'
   # … {"name":"result","valueBoolean":true}
   ```

4. **Reverse.** Click the **Reverse** radio. The input region re-renders:
   the source fields are replaced by *Target code* and *Target system*.
   Enter target system `http://hl7.org/fhir/resource-status`, target code
   `planned`. Submit. Expected: `1 matches` with a **Reverse** chip, plus an
   explanatory note — *"In reverse mode HTS omits originMap, so a match
   cannot be attributed to a specific concept map."*

5. **No match.** Back on Forward, source system
   `http://hl7.org/fhir/encounter-status`, source code `NOPE`. Submit.
   Expected: HTTP 200 with **"No matches for this source."** and the
   outcome *No mapping found for the provided code*. No equivalence table.

**Expected**

- The direction toggle is a `hx-get` on the input region with
  `hx-target="#hts-workbench-input"`, so only the form re-renders.
- No duplicate `direction` in the URL — this was the CM:139 bug; the fix
  (`hx-params="none"`) is pinned by a Rust ring test.

**Red flags**

- Reverse click leaves the URL bar with `?direction=reverse&direction=reverse`.
- The target-side fields do not re-label after a direction toggle.
- A no-match run renders an empty table instead of the outcome banner.



### 3.6 Concept plane — `/ui/hts/concepts?system=…&code=…`

Every other surface is resource-first: find a CodeSystem, then ask it a
question. This one inverts that. The address is `system` + `code`, and the
page answers the three questions an operator actually has about a code they
were handed. It is query-shaped on purpose — a canonical system URI in a
path segment would need double-encoding, and proxies reject it.

You reach it from the **Open concept** link on any `$lookup` result, or by
hand.

**Steps**

1. On §3.3 step 2's Lookup result, click **Open concept**. You land on:

   ```
   /ui/hts/concepts?system=http%3A%2F%2Fterminology.hl7.org%2FCodeSystem%2Fv3-EntityRisk&code=IFL&version=2018-08-12
   ```

   Or type it directly:
   [http://127.0.0.1:8090/ui/hts/concepts?system=http%3A%2F%2Fterminology.hl7.org%2FCodeSystem%2Fv3-EntityRisk&code=IFL](http://127.0.0.1:8090/ui/hts/concepts?system=http%3A%2F%2Fterminology.hl7.org%2FCodeSystem%2Fv3-EntityRisk&code=IFL)

2. **Identity** renders server-side in the shell, so it is there on first
   paint:

   ```
   inflammable                                    [Active]
   System          http://terminology.hl7.org/CodeSystem/v3-EntityRisk
   Code            IFL
   Display         inflammable
   CodeSystem name v3.EntityRisk
   Version         2018-08-12
   Selectability   Selectable
   Definition      Material is highly inflammable and …
   Hierarchy neighbours   Child EXP
   Designations    inflammable / en / —
   Properties      status → active
   ```

   Drop the `&version=` from the URL and the same page answers
   `CodeSystem name EntityRisk` / `Version 4.0.0` — the other stored
   version of the same canonical. Useful to know before you get confused by
   it on stage.

3. **Mappings** and **Subsumption** are lazy — their skeletons self-fetch on
   `hx-trigger="load"`. Watch them fill in. Each skeleton carries a
   `<noscript>` "Open this panel" link to the standalone route
   (`/ui/hts/concepts/mappings?…`), which renders the full page around that
   one panel, so the plane degrades to plain navigation without JS.

4. **Subsumption** on `IFL` shows one row, from the child side:

   ```
   Subsumption
   Relation | Question asked   | Outcome
   Child    | IFL subsumes EXP | Subsumes
   (explosive)
   ```

   Open the child to see the same edge from the other end:

   [`/ui/hts/concepts?system=http%3A%2F%2Fterminology.hl7.org%2FCodeSystem%2Fv3-EntityRisk&code=EXP`](http://127.0.0.1:8090/ui/hts/concepts?system=http%3A%2F%2Fterminology.hl7.org%2FCodeSystem%2Fv3-EntityRisk&code=EXP)

   ```
   Relation | Question asked   | Outcome
   Parent   | IFL subsumes EXP | Subsumes
   (inflammable)
   ```

   The panel's own caption explains the framing: *"Each row is one
   subsumption check. The ancestor candidate is always sent as code A, so a
   hierarchy that agrees with itself answers 'subsumes' every time."*

5. **Compare with code.** Type a bare code into the *Compare* box at the
   bottom of the Subsumption panel — the system is pinned to the concept's,
   so a bare code is all it wants. On `EXP`, comparing with `POI` adds a
   *Compared* row reading `POI subsumes EXP → Not subsumed`.

6. **Mappings on a mapped concept.** The v3 risk codes are not in any stored
   ConceptMap, so their Mappings panel reads *"No ConceptMap maps this
   concept. No mapping found for the provided code"* — the empty state, not
   an error. Use the encounter-status concept to see a real one:

   [`/ui/hts/concepts?system=http%3A%2F%2Fhl7.org%2Ffhir%2Fencounter-status&code=planned`](http://127.0.0.1:8090/ui/hts/concepts?system=http%3A%2F%2Fhl7.org%2Ffhir%2Fencounter-status&code=planned)

   ```
   Mappings — Mappings where this concept is the source, across every stored ConceptMap.
   Mapping vocabulary   equivalence (R4 / R4B)
   Origin map           http://hl7.org/fhir/ConceptMap/sc-encounter-status|4.0.1
   Code     | System                              | Display | Relationship
   planned  | http://hl7.org/fhir/resource-status | —       | equivalent
   ```

   Note the panel found this **without** being told which map to use — it
   calls `$translate` with `url` omitted.

**Expected**

- Identity is server-rendered; the other two panels swap themselves in and
  the returned fragment re-emits its `id` **without** the trigger, so the
  swap terminates (no polling loop).
- Every panel has its own **Raw response** disclosure.
- The permalink survives copy/paste out of a ticket — that is the whole
  point of the route shape.

**Red flags**

- A panel keeps re-fetching itself (trigger not stripped on the response).
- A half-typed permalink returns axum's bare `400` instead of a rendered
  `invalid` OperationOutcome inside the page.
- The Compare box accepts `system|code` or a bare URI and round-trips it as
  a *code*. There is a local pre-flight that is supposed to name that
  mistake; if a pasted `system|code` sails through to a confusing 404, file it.



### 3.7 §7.7 Import — `/ui/hts/import`

The page is three numbered steps: **1 · Choose source**, **2 · Review**,
**3 · Result**. Step 1 has a *Source* radio (**Paste JSON** / **Upload
file**), a `bundle_file` picker and a `bundle` textarea. Step 2 restates the
target (`http://127.0.0.1:8090/import`), the request
(`POST application/fhir+json`), the accepted resources (CodeSystem,
ValueSet, ConceptMap) and the merge rule (*Existing resources are updated in
place when `url` and `version` match*). Step 3 starts at *"No import has
been submitted yet."*

> **This step writes to the store.** The rest of this guide is read-only. If
> you are demoing against a shared seeded server, either skip §3.7 or point
> a throwaway `HTS_DATABASE_URL` at a scratch file first.

**Demo bundle (paste, or save as `bundle-small.json`)**

```json
{
  "resourceType": "Bundle",
  "type": "collection",
  "entry": [
    {
      "resource": {
        "resourceType": "CodeSystem",
        "id": "demo-cs-1",
        "url": "http://example.org/demo/cs",
        "version": "1.0.0",
        "status": "active",
        "content": "complete",
        "concept": [
          { "code": "A", "display": "Alpha" },
          { "code": "B", "display": "Beta" }
        ]
      }
    }
  ]
}
```

Partial-success variant — one good entry plus one entry with an `id` but no
`resourceType`:

```json
{
  "resourceType": "Bundle",
  "type": "collection",
  "entry": [
    {
      "resource": {
        "resourceType": "CodeSystem",
        "id": "demo-cs-ok",
        "url": "http://example.org/demo/cs/ok",
        "version": "1.0.0",
        "status": "active",
        "content": "complete",
        "concept": [{ "code": "X", "display": "Ok" }]
      }
    },
    { "resource": { "id": "broken-no-type" } }
  ]
}
```

**Steps (paste path)**

1. Click **Import** in the sidebar.
2. Paste an empty string. Submit. → pre-flight 400, UI-owned.
3. Paste `{ not json`. Submit. → pre-flight 400, UI-owned.
4. Paste the first bundle. Submit. → 200 success, with the imported-entry
   count in step 3.
5. Paste the partial-success bundle. Submit. → 207 with a per-entry
   breakdown.

**Steps (file path)**

6. Click the **Upload file** radio. The paste textarea hides; the file input
   appears with the hint *"JSON only. The file is read in your browser and
   copied into the Bundle field below; nothing is sent until you submit."*
7. Pick `bundle-small.json`. Submit. → same result strip as the paste path.
8. Click **Paste JSON** again. The textarea reappears with the file contents
   still in place, in case you want to edit before re-submitting.

**Expected**

- Pre-flight errors never hit the backend. The submit button re-enables
  after the error banner renders.
- Success reports how many entries were imported and links back to the
  respective browser.
- 207 shows which entries succeeded and which failed, each failure with its
  `OperationOutcome`.
- 413 (paste > 10 MB) is intentionally not covered by a Playwright test; the
  Rust ring covers it with a canned mock. Skip it in the demo unless you
  want to paste a large fixture by hand.
- **File path caveat.** The file is urlencoded into the same `bundle=…`
  field the paste path uses, so URL-encoding overhead (~33 %) puts the
  effective JSON cap on the file path at ~7.5 MiB before HTS returns 413.
  For anything larger, paste the Bundle directly (paste bytes go on the wire
  verbatim) or split the file.

**Red flags**

- Submit stays disabled after a validation error (means the
  `UpstreamHealth` decode broke — Grupo B bug pattern).
- File picker does nothing when clicked (means `import.js` did not load —
  check `/ui/hts/assets/import.js` returns 200 and the `<script>` tag is
  present in `import.html`).



### 3.8 Capability & Conformance — `/ui/hts/capability-statement`

Formerly §7.9 Diagnostics at `/ui/hts/diagnostics`. That path still works
and answers `308 → /ui/hts/capability-statement`; bookmarks and old links
survive. There are no tabs any more — it is **six stacked cards**, composed
live from `/metadata` and `/metadata?mode=terminology` in one pass.

```bash
curl -s -o /dev/null -w '%{http_code} -> %{redirect_url}\n' 'http://127.0.0.1:8090/ui/hts/diagnostics'
# 308 -> http://127.0.0.1:8090/ui/hts/capability-statement
```

**Steps**

1. Open
   [http://127.0.0.1:8090/ui/hts/capability-statement](http://127.0.0.1:8090/ui/hts/capability-statement).
2. Read down the cards. Expand the last one.
3. Click the **1977** in *Terminology Capabilities* — it links to
   `/ui/hts/code-systems`.
4. Click **View the complete statement** at the bottom — it opens
   `/metadata`.

**Expected**, card by card:

| Card                          | What it shows on the seeded server                                                                                                                                                                                                                     |
| ----------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Server Summary**            | Description `Helios Terminology Server SQLite backend`; Base URL `http://heliossoftware.com/fhir/hts/CapabilityStatement/hts`; FHIR version `4.0.1`; Status `active`; Kind `instance`; Date `2026-04-01`; Formats `application/fhir+json, application/fhir+xml`. |
| **System Interactions**       | **Absent.** HTS declares no `rest[].interaction[]`, and the template omits the whole card rather than rendering an empty one. Its absence is the correct result — do not file it.                                                                        |
| **Operations**                | `$versions`, `$lookup`, `$validate-code`, `$subsumes`, `$expand`, `$translate`, `$closure`, each with its HL7 `OperationDefinition` canonical. Note `$closure` is listed here but has no UI page (§4.3).                                                  |
| **Per-Resource Capabilities** | CodeSystem / ValueSet / ConceptMap, each `read create update delete search-type`, each with `5` search params.                                                                                                                                          |
| **Terminology Capabilities**  | Hierarchical expansion `No`; Expansion paging `Yes`; Incomplete expansions `No`; Validate-code translations `No`; Translation needs a map `Yes`; Closure maintenance `Yes`; **Code systems declared `1977`** (a link); and the `$expand` parameter list — `activeOnly, check-system-version, count, displayLanguage, excludeNested, force-system-version, includeDefinition, includeDesignations, offset, property, system-version, tx-resource`. |
| **Raw CapabilityStatement (JSON)** | A `<details>` disclosure, collapsed by default. Opens to pretty-printed JSON, cut off with: *"Truncated to the first 16384 of 356648 bytes — this server's statement grows with the code systems it loads."* followed by a **View the complete statement** link to `/metadata`. |

**Red flags**

- A "System Interactions" card rendering **empty** rather than being
  omitted.
- The truncation note missing while the JSON is obviously cut — that means
  someone changed the cap without updating the note.
- The raw JSON rendered as HTML (missing `<pre>` escaping).
- *Terminology Capabilities* claiming `Hierarchical expansion: Yes`. It says
  `No` today, which is what the statement declares — even though tree mode
  in §3.4 works. That mismatch is a backend conformance question, not a UI
  one; note it on the plan rather than filing it against `crates/hts-ui`.

---



## 4. Cross-cutting scenarios

Run these across at least two pages each.

### 4.1 Theme (light / dark / system)

Toggle from the topbar buttons on §3.1, §3.4, and §3.8. Confirm:

- No FOUC (Flash Of Unstyled Content) when navigating pages — `theme.js`
  runs before first paint.
- Both themes ship enough contrast for `axe-core` (verified in Phase 3
  Playwright). Visually spot-check the outcome banners on the CodeSystem
  Validate tab — they use accent colors that are easy to underdo.



### 4.2 i18n (en / es / de)

Switch to **Spanish** on §3.1 and confirm the tiles read *Estado del
servidor* / *Tiempo activo* / *Sistemas de códigos cargados* /
*Solicitudes*. Then switch to **German** and load `/ui/hts/code-systems`:
heading *Codesysteme*, buttons *Suchen* / *Zurücksetzen*, status chips
*Jeder Status / aktiv / Entwurf / zurückgezogen / unbekannt*.

The switcher is three plain `?lang=` links, so it works with JS disabled and
the choice is visible in the URL.

Look for:

- Untranslated keys rendered as `hts-something-else`. There should be
  **none**: a sweep of all six pages in all three locales came back clean on
  2026-08-28, and outcome codes now degrade to the code rather than the key
  (§3.3). Anything you spot is a regression — file it.
- Long German compounds pushing the layout. The sidebar and the browser
  toolbar are the tight spots.



### 4.3 Operations with no UI page (API-only)

Three operations are routed and working but deliberately have no page: the
standalone Operations workbench that used to host them was deleted.
`/ui/hts/operations` returns `404`. Demo them with curl.

**ValueSet `$validate-code`** — `GET` or `POST /ValueSet/$validate-code`:

```bash
curl -s 'http://127.0.0.1:8090/ValueSet/$validate-code?url=http://hl7.org/fhir/ValueSet/immunization-status&system=http://hl7.org/fhir/event-status&code=completed'
```

```json
{"resourceType":"Parameters","parameter":[
  {"name":"code","valueCode":"completed"},
  {"name":"display","valueString":"Completed"},
  {"name":"issues","resource":{"resourceType":"OperationOutcome", ... }}, ... ]}
```

**`$batch-validate-code`** — `POST /ValueSet/$batch-validate-code` only.
Each row is a `validation` parameter carrying its **own** `Parameters`
resource, and each of those must carry its own `url`; a `url` at the top
level is not inherited and every row comes back
`Missing required parameter: url`.

```bash
curl -s -X POST -H 'Content-Type: application/fhir+json' \
  -d '{"resourceType":"Parameters","parameter":[
    {"name":"validation","resource":{"resourceType":"Parameters","parameter":[
      {"name":"url","valueUri":"http://hl7.org/fhir/ValueSet/immunization-status"},
      {"name":"coding","valueCoding":{"system":"http://hl7.org/fhir/event-status","code":"completed"}}]}},
    {"name":"validation","resource":{"resourceType":"Parameters","parameter":[
      {"name":"url","valueUri":"http://hl7.org/fhir/ValueSet/immunization-status"},
      {"name":"coding","valueCoding":{"system":"http://hl7.org/fhir/event-status","code":"NOPE"}}]}}]}' \
  'http://127.0.0.1:8090/ValueSet/$batch-validate-code'
```

You get one `validation` output per input, in order, each holding a
`Parameters` (with `result` / `display` / `issues`) or an
`OperationOutcome`.

**`$closure`** — `POST /ConceptMap/$closure`. Listed in the Capability &
Conformance *Operations* card and declared `Closure maintenance: Yes`. It
maintains named server-side closure state, so it is a write: not exercised
in this read-only walk-through. Use a scratch database if you want to demo it.



### 4.4 Expansion ceiling ("too costly")

`HTS_MAX_EXPANSION_SIZE` (default **3500**) is applied only when a
`$expand` request omits `count` — it is a per-request ceiling, not a global
cap.

**You cannot reproduce it on the seeded store as booted.** The largest
expansion in this seed set is `http://terminology.hl7.org/ValueSet/v3-ActCode`
at **1 302** members, comfortably under 3 500; the value sets that would be
huge (`observation-codes` / LOINC, `clinical-findings` / SNOMED CT) expand
to `total 0` because their code systems are not in the bundle.

To see the banner you need a dedicated boot with the ceiling lowered:

```powershell
$env:HTS_BOOTSTRAP_DIR     = "./crates/hts/terminology-data"
$env:HTS_UI_ENABLED        = "true"
$env:HTS_MAX_EXPANSION_SIZE = "5"
cargo run --bin hts
```

Then open `/ui/hts/value-sets/languages/expand`, **clear the `count` input**
(empty, not `0` — HTS only applies the ceiling when `count` is absent) and
Run. Expected: a red banner and a **Raise threshold** form that retries with
an explicit `count` — the same input the workbench normally pre-fills with
`50`. The expand form's advanced fieldset also carries a `threshold` field
for exactly this retry.

Treat this section as **not verified on the shared demo server**. Run it on
your own instance before signing it off.



### 4.5 Degraded state

The UI always talks to HTS over HTTP — by default to
`http://127.0.0.1:{HTS_SERVER_PORT}`, i.e. the same binary on loopback. When
that leg fails or exceeds the 5 s request timeout, the Home cards render a
banner:

> **Terminology backend not fully available** — The terminology server did
> not respond in time. *Some tiles are hidden until HTS becomes reachable
> again. Interactive controls are disabled on affected pages.*

…and the affected tiles collapse to em-dashes while the chart caption
switches to *"/metrics is unreachable — no new samples are arriving."*

**Option A (transient, free).** You will very likely hit this without trying
on a loaded or freshly restarted server — the first
`/metadata?mode=terminology` fetch after idle can exceed the timeout on a
store with 1 977 code systems. Refresh; the tiles fill back in. If you want
to force it, hammer the API in one shell while reloading `/ui/hts` in the
browser.

**Option B (deterministic).** Point the UI at a dead upstream:

```powershell
$env:HTS_UI_ENABLED     = "true"
$env:HTS_UI_UPSTREAM_URL = "http://127.0.0.1:9999"   # nothing listens
cargo run --bin hts
```

Load `/ui/hts`. Expected: the banner, `status` and the inventory tiles as
em-dashes, and the page still rendering — no white screen.

**Option C (up but empty).** Boot without `HTS_BOOTSTRAP_DIR`. The store is
empty, so **Loaded code systems** reads `0` and the bundled-data hint shows
an em-dash rather than a misleading `0 MB`. That is "up but empty", not
degraded, and the UI should say so rather than hide it. If the number is
missing entirely, that is a rendering bug.

---



## 5. Sign-off

You are done when:

- [ ] §3.1 Home — 4 tiles + chart, 15 s poll, range/series links,
      theme + locale switch clean.
- [ ] §3.2 CS browser — 300 ms debounce, case-sensitive filters, status
      facets, Load-more advancing `_offset`, empty state.
- [ ] §3.3 CS detail — 308 to `/lookup`; facts block stays above; Lookup /
      Validate / Subsumes move `aria-current`; `IFL`/`EXP`/`POI` give
      `subsumes` / `subsumed-by` / `not-subsumed`.
- [ ] §3.4 VS browser + `$expand` — 3-code expansion on
      `immunization-status`, 56-member pager on `languages`, indented tree
      on `v3-EntityRisk`.
- [ ] §3.5 CM browser + `$translate` — Mapping column, forward, reverse,
      no-match.
- [ ] §3.6 Concept plane — Identity server-rendered, Mappings and
      Subsumption lazy-load and terminate, Compare box works, permalink
      round-trips.
- [ ] §3.7 Import — pre-flight, 200, 207, file/paste toggle. *(Skip on a
      shared server — it writes.)*
- [ ] §3.8 Capability & Conformance — 5 cards rendered + System Interactions
      correctly absent, `/ui/hts/diagnostics` still 308s, raw JSON truncated
      with the note and the `/metadata` link.
- [ ] §4.3 API-only ops — `$validate-code` and `$batch-validate-code`
      answered over curl; `/ui/hts/operations` confirmed `404`.
- [ ] Themes — light + dark on at least two pages.
- [ ] i18n — `en` + `es` on at least two pages; `de` spot check.
- [ ] Degraded — one of §4.5 options shows the banner without crashing the UI.

Fixed on 2026-08-28, found by writing this guide against the real seed set:

- ~~Detail pages only resolve the first 1 000 rows~~ — they now page with
  `_offset`, so the whole catalog opens (§2.5).
- ~~Outcome codes render as raw Fluent keys~~ — an untranslated FHIR issue
  code now surfaces as itself (`business-rule`), which is what the partial
  always documented.

Known findings still open, so nobody re-files them:

1. **`Concept count` is always an em-dash** — HTS does not populate
   `CodeSystem.count` (§3.3). Analysis and cost:
   `edson/docs/hts-ui-improvement-plan.md` §14.
2. **`Bundle.total` echoes the page size**, not the store size (§2.1).
3. **`GET /{type}/{id}` instance reads 404** on seeded resources, and `?_id=`
   is silently ignored; only `?url=` search narrows (§2.1, §2.5).
4. **`TerminologyCapabilities` declares `Hierarchical expansion: No`** while
   tree mode demonstrably works (§3.8).
5. **`GET /ValueSet?_count=100000` resets the connection** on a seeded store
   — page with `_offset` instead (§2.5).
6. **ICD-9-CM imported one concept** from a 1 MB source file. A silent
   import failure, not a UI issue.

When every box is ticked, respond "UI OK" and I move to Phase 5 (draft
`edson/docs/hts-ui-discussion.md`).

Any red flag above → file it as a comment on the plan or as a new bullet
here; I will loop with a fix and we re-run the affected block before signing
off.
