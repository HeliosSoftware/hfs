# HTS UI — demo guide (Phase 4 walk-through)

**Audience**: reviewer running the HTS admin UI locally against the `hts`
binary before we open the single PR for #551.

**Scope**: everything shipped in `crates/hts-ui` v1 — §7.1 Home, §7.2
CodeSystem browser, §7.3 CodeSystem detail (Lookup / Validate / Subsumes),
§7.4 ValueSet browser + `$expand`, §7.5 ConceptMap browser + `$translate`,
§7.6 Operations workbench (7 ops, batch-validate polling, closure banner),
§7.7 Import, §7.9 Diagnostics. §7.8 Bootstrap ledger is deferred to Phase 8+
and is NOT part of this demo.

**Exit criterion** (from plan `hts_ui_delivery_strategy_8b4bcd79.plan.md`
Phase 4): reviewer says "UI OK" on every §7 page below, or files findings
that a follow-up iteration addresses before Phase 5.

---



## 0. Prerequisites (one-time)


| Thing          | Value                                                                                                                    | Why                                                                                                                                                        |
| -------------- | ------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Rust toolchain | `stable-x86_64-pc-windows-gnu` (portable mingw-w64 at `C:\Users\tercere\tools\mingw64\`)                                 | The `stable-*-pc-windows-gnullvm` linker is not installed on this host; Phase 3b confirmed builds on `-gnu` clean.                                         |
| Cargo profile  | `dev` is fine — release not needed for the demo                                                                          | UI logic is not perf-sensitive.                                                                                                                            |
| Node / pnpm    | Only needed if you want to reuse `crates/hts-ui/e2e/seed.mjs` to seed via Playwright's harness                           | You can also curl the same bundle by hand — see §1.                                                                                                        |
| Ports          | HTS binary listens on `HTS_SERVER_PORT` (default **8090**); the e2e harness reuses `HTS_E2E_PORT` (also default `8090`). | Both default to the same port, so a booted `hts` binary and the seed script line up out of the box. Swap in another port only if `8090` is already in use. |
| Locale packs   | `locales/{en,es,de}/main.ftl` already bundled into the binary                                                            | No filesystem lookup at runtime.                                                                                                                           |


If you are inside the corporate VPN and off-VPN switches, remember: HTS
itself does not talk to the internet at runtime (all data is in-process
SQLite + embedded core packs). The `HTTP_PROXY` env in your shell does not
affect the HTS binary for `/ui/hts` requests.

---



## 1. Boot the server



### 1.1 Empty-store boot (home sanity)

```powershell
$env:HTS_UI_ENABLED = "true"          # NOT "1" — the binary flag parses as bool
$env:HTS_SERVER_PORT = "8090"         # optional; default is 8090 anyway
cargo run --bin hts
```

Open [http://127.0.0.1:8090/ui/hts](http://127.0.0.1:8090/ui/hts). You should see:

- Topbar with the **HTS** brand, a dialect chip (defaulting to `en`), a
theme toggle, and a help link.
- Status card row: **status = up**, backend = SQLite in-memory (or the
file backend if you set one), uptime counting up, FHIR version = the
compile-time version of the `hts` binary.
- Loaded systems card = **0** and Bundled data card = **0 MB** on a fresh
boot. This is expected on empty storage — proceed to §2 to seed.
- Quick-links row: Browse CS, Browse VS, Browse CM, Operations. Import
status strip below (should say "No imports yet").

If the dashboard renders but the status card says **degraded** with a
reason like "upstream 5xx" or "decode error", stop and share the reason —
that is a bug we want to catch before Phase 6.

### 1.2 Optional overrides worth knowing


| Env var                           | Default                                         | What it does                                                                                                                                                      |
| --------------------------------- | ----------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `HTS_UI_ENABLED`                  | `false`                                         | Master switch that mounts `/ui/hts` under the `hts` binary. Bool-typed — use `"true"` / `"false"`, not `"1"` / `"0"`.                                             |
| `HTS_SERVER_PORT`                 | `8090`                                          | The TCP port `hts` binds. `HTS_SERVER_HOST` (default `127.0.0.1`) pairs with it.                                                                                  |
| `HTS_UI_UPSTREAM_URL`             | (in-process)                                    | When set, the UI proxies to an external HTS instead of its own `AppState`. Use this to demo the degraded-guard banner: point it at a URL that answers 5xx (§4.5). |
| `HTS_MAX_EXPANSION_SIZE`          | `3500`                                          | HTS-side ceiling used when a `$expand` request omits `count`. Setting it low (e.g. `5`) turns `ex-vs-too-costly` into a live too-costly demo (§4.4).              |
| `HTS_UI_BATCH_FANOUT_CONCURRENCY` | `8` (compile-time constant, not env-driven yet) | Max in-flight rows the batch-validate workbench queues at once. Referenced here for §4.3; you cannot override it at runtime in v1.                                |


Note the last row: `HTS_UI_BATCH_FANOUT_CONCURRENCY` is defined as a Rust
`const` in `crates/hts-ui/src/upstream.rs`, not an env lookup. Changing it
requires a rebuild. This is called out on the plan's Phase 8 backlog.

---



## 2. Seed the terminology store

The dashboard is useful empty, but every other page needs data. Two
options — use whichever is more convenient.

### 2.1 Reuse the Playwright seed (recommended)

`crates/hts-ui/e2e/seed.mjs` builds a canonical `Bundle` with 34 CS + 5 VS +
2 CM. Run it against your booted server:

```powershell
cd crates/hts-ui
$env:HTS_E2E_PORT = "8090"    # matches the binary's HTS_SERVER_PORT default
$env:NO_PROXY = "127.0.0.1,localhost"   # keeps node's fetch off the corporate proxy
node -e "import('./e2e/seed.mjs').then(m => m.default()).then(()=>console.log('SEED OK'))"
```

It POSTs the bundle to `http://127.0.0.1:8090/import` and prints
`[seed] import 200 OK: CS=34 VS=6 CM=2 concepts=64` followed by
`SEED OK` on success. If it prints an `OperationOutcome` body instead,
read the diagnostic — usually the port is wrong or the binary is not
running yet.

> **Caveat (Bucket C, HTS backend — out of #551 scope):**
> re-importing the same seed on top of existing data leaves
> `concept_closure` empty for the affected CodeSystems. `$lookup`
> and `$validate-code` keep working (they read `concept_hierarchy`
> directly), but `$subsumes(A, B)` on `ex-cs-1` silently regresses
> from `subsumes` to `not-subsumed` until the next HTS restart —
> at startup, `migrate_concept_closure` rebuilds the closure for
> every system that has hierarchy edges but no closure rows. Root
> cause is inside `crates/hts` (`import::fhir_bundle`) and belongs
> to a separate backend mini-issue; the UI is not touching HTS
> internals.
>
> **Workaround for the demo.** If `§3.3` Subsumes suddenly reports
> `not-subsumed` where §3.3 expects `subsumes`:
>
> 1. Cheapest: stop `hts`, boot it again (no re-seed needed) — the
>    startup migration rebuilds the closure over the existing data
>    in place.
> 2. Fresh start: stop `hts`, delete `$env:TEMP\.hts-e2e-8090.db*`
>    (or the file at `HTS_DATABASE_URL`), boot again, and re-run
>    this seed step once.
>
> The Playwright suite sidesteps this by wiping the ephemeral
> SQLite inside `boot.mjs` on every run, so its subsumes test
> always exercises a fresh-import path.



### 2.2 Curl a minimum bundle (if you want to see the wire format)

```powershell
$body = @'
{ "resourceType": "Bundle", "type": "collection", "entry": [
  { "resource": { "resourceType": "CodeSystem", "id": "ex-cs-1",
      "url": "http://example.org/cs", "version": "1.0.0",
      "status": "active", "content": "complete",
      "concept": [ { "code": "A", "display": "Alpha" },
                   { "code": "B", "display": "Beta"  } ] } }
] }
'@
Invoke-WebRequest -Uri http://127.0.0.1:8090/import -Method POST `
    -ContentType "application/fhir+json" -Body $body
```

Success = HTTP `200` + a `Bundle` echo. HTTP `207` on partial success,
`400` on bad JSON, `413` on payloads over 10 MB — these are the four
Import-page states you will see in §3.6.

### 2.3 What the full seed gives you

After §2.1 the store contains:

> **How to find these in the UI.** Every browser table's link text is the
> resource `name` (with `id` shown only when `name` is empty); the `id`
> ends up in the row's link `href` (e.g.
> `/ui/hts/value-sets/ex-vs-1`). Use the **UI label (Name)** column
> below to spot a specific fixture in the table, or the **URL** column
> — both are painted verbatim. If in doubt, deep-link with the id.

| Resource   | Id                              | UI label (Name) — link text          | Canonical URL                                             | Highlights (why the demo uses it)                                                                                     |
| ---------- | ------------------------------- | ------------------------------------ | --------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------- |
| CodeSystem | `ex-cs-1`                       | `ExampleCodeSystem`                  | `http://example.org/cs`                                   | A → B subsumption; A has a `designation` and a `status=active` property. Powers Lookup / Validate / Subsumes on §7.3. |
| CodeSystem | `ex-cs-limbs`                   | `ExampleLimbsCS`                     | `http://example.org/cs/limbs`                             | 60 flat concepts. Backs `ex-vs-1` (paged expand) and `ex-vs-too-costly` (§4.2).                                       |
| CodeSystem | `ex-cs-source` / `ex-cs-target` | `ExampleSourceCS` / `ExampleTargetCS`| `http://example.org/cs/source` / `.../cs/target`          | Referenced by `ex-cm-1`.                                                                                              |
| CodeSystem | `ex-cs-2` … `ex-cs-31`          | (filler names — one per row)         | `http://example.org/cs/filler-N`                          | Filler rows so the browser Load-more button fires.                                                                    |
| ValueSet   | `ex-vs-1`                       | `ExampleLimbsVS`                     | `http://example.org/vs/limbs`                             | Flat expansion of `ex-cs-limbs`. Pager fires past 25 rows.                                                            |
| ValueSet   | `ex-vs-tree`                    | `ExampleTreeVS`                      | `http://example.org/vs/tree`                              | Hierarchical expansion of `ex-cs-1` — role="tree" mode.                                                               |
| ValueSet   | `ex-vs-batch-mixed`             | `ExampleBatchMixedVS`                | `http://example.org/vs/batch-mixed`                       | Envelope composing `ex-cs` + `ex-cs/source`. Target of choice for the `$batch-validate-code` demo (§3.6) — a single batch job validates codes drawn from both CodeSystems against one URL. |
| ValueSet   | `ex-vs-too-costly`              | `ExampleTooCostlyVS`                 | `http://example.org/vs/too-costly`                        | Reuses `ex-cs-limbs`; when `HTS_MAX_EXPANSION_SIZE=5` boots the binary, its default expand blows past the ceiling.    |
| ValueSet   | `ex-vs-source` / `ex-vs-target` | `ExampleSourceVS` / `ExampleTargetVS`| `http://example.org/vs/source` / `.../vs/target`          | Referenced by `ex-cm-1`.                                                                                              |
| ConceptMap | `ex-cm-1`                       | `ExampleCM`                          | `http://example.org/cm/example`                           | Forward A → T1 with `equivalent` equivalence.                                                                         |
| ConceptMap | `ex-cm-no-match`                | `ExampleCMNoMatch`                   | `http://example.org/cm/no-match`                          | Empty group so a well-formed translate returns HTTP 200 + `result=false`.                                             |


---



## 3. Per-page walk-through

Follow these in order. Each block is one page: what to click, what to
observe, what a green vs. red result looks like.

### 3.1 §7.1 Home — `/ui/hts`

**Steps**

1. Land on `/ui/hts` (`/ui/hts/` 308-redirects to the canonical path if you get the trailing slash wrong).
2. Wait 15 seconds without clicking. Watch the status cards.
3. Toggle theme via the topbar chip. Toggle again.
4. Open the dialect chip; pick `es`. Reload with `?lang=es` in the URL if the chip does not persist.

**Expected**

- Poll after 15 s: the "Requests" / "Latency" / "Uptime" numbers refresh
in place. No layout shift. Focus stays where it was.
- Theme toggle: instant switch. No off-origin asset load (open DevTools →
Network; every request should be `127.0.0.1:8090`).
- `es`: card labels, quick-link labels, and topbar all switch to Spanish.
The FHIR-version chip stays as `R4` (or whatever your binary was built
with) — it is metadata, not a translatable term.

**Red flags**

- A poll erases focus or scrolls the page.
- Any request in DevTools points at `cdn.*`, `jsdelivr`, or `unpkg`.
- A locale key renders as `hts-dashboard-something-something` — that
means a Fluent key was added on one side of the code but not on the
other. File it.



### 3.2 §7.2 CodeSystem browser — `/ui/hts/code-systems`

Layout note (Phase 5): the filter form lives in a sticky rail on the
left; string fields (URL / Name / Title / Version) are plain text
inputs that map 1:1 to FHIR search parameters, so matching is exact
and case-sensitive — inherited from the HTS backend (no match-mode
toggle; see design doc §7.2.1.1). The results table on the right
renders **Name · Title · URL · Version · Status** so every filter
has a visible column.

> **UI-vs-route naming.** `ex-cs-1` is the FHIR `id`; it does not
> appear as visible text in the table. The link text is the resource's
> `name` (`ExampleCodeSystem` for `ex-cs-1`) and the `id` is only in
> the row's link `href`. Scan the URL / Name / Title columns — or
> deep-link with `/ui/hts/code-systems/ex-cs-1`. See §2.3 for the
> full id ↔ Name ↔ URL map.

**Steps**

1. Click **Browse CS** on the dashboard (or open `/ui/hts/code-systems`).
2. Type an exact `name` that exists in the seed (e.g. the seed CS
  `name`) into the *Name* filter. Wait 300 ms — the table should
   narrow. Try the same string in the opposite case; with the current
   backend that usually returns **no** rows (case-sensitive).
3. Reset the filter. You should see 25 rows + a **Load more** button
  (seed has 34 CodeSystems; `_count` default is 25).
4. Click **Load more** once. Rows should grow (≈34 total) and the
  **Load more** button should **disappear** (end of list).
5. (Optional) Hard-refresh and click **Load more** twice quickly —
  you must **not** see duplicate rows; a second click must not be
   possible after the button is gone.

**Expected**

- Debounced filter: typing does not fire a request per keystroke;
only after 300 ms of quiet does the tbody re-render. Status
`<select>` re-fires immediately via `change`.
- Load-more appends the next page below the current rows (no full
re-render, no scroll jump). The footer is OOB-swapped so the
button's `_offset` advances; at the terminal page the button is
omitted.
- Empty state (type gibberish): the tbody shows a "No CodeSystems match"
row — but the header, filters, and buttons stay put.

**Red flags**

- Load-more scrolls to the top, double-renders rows, or keeps
offering **Load more** after every row is already visible.
- Filter clears the tbody to a spinner instead of a skeleton row.
- A visible match-mode `<select>` next to URL/Name/Title (those
were rolled back; must not reappear without a backend plan).



### 3.3 §7.3 CodeSystem detail — `/ui/hts/code-systems/ex-cs-1`

Design doc §8.3: the resource summary (URL, publisher, jurisdiction,
content mode, concept count, status pill) is a **facts block always
visible at the top**; below it, a **tab strip lists operations only**
— Lookup, Validate, Subsumes. There is no "Metadata" tab. The naked
`/ui/hts/code-systems/ex-cs-1` URL 308-redirects to
`/ui/hts/code-systems/ex-cs-1/lookup`, so the URL bar always names the
active operation.

> **UI-vs-route naming.** Same rule as §3.2: `ex-cs-1` is only in the
> route; the browser paints the row as Name **ExampleCodeSystem** /
> URL `http://example.org/cs`. Steps below use the deep-link, but you
> can also reach the same page by clicking that row from §3.2.

**Steps**

1. Open `/ui/hts/code-systems/ex-cs-1` (or click the **ExampleCodeSystem**
   row from §3.2). The URL should resolve to
   `/ui/hts/code-systems/ex-cs-1/lookup`; the **Lookup** tab is active
   (`aria-current="true"`). Above the tab strip, confirm the facts
   block shows `url = http://example.org/cs`, `version = 1.0.0`, the
   status pill, and the concept count.
2. **Lookup**: type code `"A"` in the Code input. Submit.
3. **Validate**: click the Validate tab. Type code `"A"`, then run again
   with `"NONEXISTENT"`. Two runs.
4. **Subsumes**: click the Subsumes tab. Enter code A `"A"` and code B
   `"B"`. Submit. Then swap them and run again.

**Expected**

- Tab clicks swap ONLY the region under the facts block — the facts
  block stays visible above. Region-wrap contract (§8.1).
- The URL bar updates to `/{id}/{op}` on each tab click
  (`hx-push-url="true"`).
- Lookup on `"A"` shows the `Alpha` display, the `en` designation, and
  the `status=active` property panel.
- Validate on `"A"` = result `true`. On `"NONEXISTENT"` = `false` with
  an `OperationOutcome` diagnostic in the outcome banner.
- Subsumes `A subsumes B` = `subsumes`. Swapping = `subsumed-by`.

**Red flags**

- Any "Metadata" tab visible in the tab strip (retired in §8.3; file it).
- Clicking a tab reloads the whole page (Askama base + topbar re-render).
- The facts block above the tab strip disappears when a different
  operation tab is clicked.
- The workbench input has a duplicated `id="hts-workbench-input"` —
  this was a Grupo A bug; the fix is in commit `61bfc4f59`. If it
  reappears file it.
- **Subsumes reports `not-subsumed` where the expected is `subsumes`
  / `subsumed-by`.** Not a UI regression — this is the HTS backend
  bug flagged in §2.1's Bucket C caveat: `concept_closure` was
  wiped by a prior re-import and never rebuilt. Restart `hts` (the
  startup `migrate_concept_closure` migration is the safety net)
  and re-run this step; if it still reproduces on a fresh boot,
  then file it.



### 3.4 §7.4 ValueSet browser + `$expand` — `/ui/hts/value-sets`

> **UI-vs-route naming.** The browser table columns are **Name · Title ·
> URL · Version · Status** (see [`hts-vs-rows.html`](../../crates/hts-ui/templates/partials/hts-vs-rows.html)).
> `ex-vs-1` is the FHIR `id` and appears **only** in the row's link
> `href` (`/ui/hts/value-sets/ex-vs-1`), never as visible text — the
> link text is the `name` (`ExampleLimbsVS`) because the seed provides
> one. To find the row scan the URL column for
> `http://example.org/vs/limbs`, or search by Name (`ExampleLimbsVS`)
> / Title (`Example Limbs Value Set`). If you prefer, deep-link
> directly with `/ui/hts/value-sets/ex-vs-1` — the base URL 308-
> redirects to `/expand` per §8.3. The same rule applies to
> `ex-vs-tree` → **ExampleTreeVS** / `http://example.org/vs/tree`.

**Steps**

1. Click **Browse VS** on the dashboard. Confirm you see 5 rows (or
   Load-more if fewer). If the table is empty, re-run the seed from
   §2.1 — the curl bundle in §2.2 imports only `ex-cs-1` and leaves
   `/value-sets` empty.
2. Locate the **ExampleLimbsVS** row (Title *Example Limbs Value Set*,
   URL `http://example.org/vs/limbs`) and click it. You land on
   `/ui/hts/value-sets/ex-vs-1/expand` (route id `ex-vs-1`). Direct
   deep-link: [http://127.0.0.1:8090/ui/hts/value-sets/ex-vs-1](http://127.0.0.1:8090/ui/hts/value-sets/ex-vs-1).
3. Under the *Expand* tab, click **Run** with defaults (`count=50`, flat).
4. Change *count* to `10`; toggle **tree** mode; click Run.
5. Go back to the browser; open the **ExampleTreeVS** row (URL
   `http://example.org/vs/tree`, route id `ex-vs-tree`). Direct
   deep-link: [http://127.0.0.1:8090/ui/hts/value-sets/ex-vs-tree](http://127.0.0.1:8090/ui/hts/value-sets/ex-vs-tree).
6. On ex-vs-tree, toggle *tree mode*; you should see nested `A > B`.

**Expected**

- Flat mode: table of 50 limb concepts with a Load-more/pager below.
- Tree mode on `ex-vs-1`: the underlying CS is flat (no
`hierarchyMeaning`), so HTS returns a flat expansion and the
workbench silently renders it as a flat table with the pager. There
is **no banner** — tree mode degrades gracefully to flat rather
than surfacing an OperationOutcome. Verified via Playwright at
§3.4 in `value-sets.spec.ts` (`toggling tree mode on a flat CS degrades silently to a flat table`).
- Tree mode on `ex-vs-tree`: `<ul role="tree">` renders `A` as the
root and `B` as its child. Keyboard arrows navigate.

**Red flags**

- Tree/flat toggle causes the whole page to reload.
- Pager button disappears when it should be shown (rule: hidden only
when `expansion.total ≤ rendered rows`).



### 3.5 §7.5 ConceptMap browser + `$translate` — `/ui/hts/concept-maps`

Layout note (Phase 5): CM shares the sticky-rail form with CS / VS
(see §3.2), but the results table is **5-column with a stacked
Mapping cell** — Name · Title · URL · Mapping (`S:` source URI +
`T:` target URI on two aligned lines) · Status. The rail advertises
*Source system* / *Target system* inputs, but they are still
silently dropped by axum's `Query` extractor on the backend — the
`ResourceSearchQuery` struct does not declare `source-uri` /
`target-uri`. The Mapping column still surfaces the values from
each CM resource in the response, so operators can eyeball
direction even though they cannot filter by it. Tracked as a
`helios-hts` bug.

> **UI-vs-route naming.** Same rule as §3.2 / §3.4: `ex-cm-1` and
> `ex-cm-no-match` are FHIR ids and appear only in the row's link
> `href`. In the table look for Name **ExampleCM** and
> **ExampleCMNoMatch**, or deep-link via
> `/ui/hts/concept-maps/ex-cm-1`. See §2.3.

**Steps**

1. Click **Browse CM**.
2. In the rail, type `http://example.org/cs/source` in *Source
  system* — HTS currently ignores the parameter (backend bug, see
   the layout note above), so expect the table not to narrow; the
   `S:` line in the Mapping cell will still show the URI for maps
   that carry it.
3. Open the **ExampleCM** row (route id `ex-cm-1`).
4. On the *Translate* tab, enter source `A`, source system
  `http://example.org/cs/source`. Submit.
5. Toggle direction to **Reverse**. Wait for the input to re-render.
6. Submit reverse with target `T1`, target system
  `http://example.org/cs/target`.
7. Open the **ExampleCMNoMatch** row (route id `ex-cm-no-match`). Repeat forward translate on `A`. Compare.

**Expected**

- Forward on `ex-cm-1`: match found, target `T1` with `equivalent`
equivalence (or `relationship: equivalent` on R5+ — depends on
compile-time FHIR version).
- Reverse: the form's field labels flip from source-* to target-* and
vice-versa. No duplicate `direction` in the URL — this was the CM:139
bug; the fix `hx-params="none"` is pinned by a Rust ring test.
- `ex-cm-no-match`: HTTP 200 with `result = false` and a "no mapping"
outcome. The equivalence panel should not render.

**Red flags**

- Reverse click leaves the URL bar with `?direction=reverse&direction=reverse`.
- The target-side fields do not re-label after a direction toggle.



### 3.6 §7.6 Operations workbench — `/ui/hts/operations`

Standalone workbench. 7 ops in the selector: `$lookup`, `$validate-code`,
`$subsumes`, `$expand`, `$translate`, `$closure`, `$batch-validate-code`.

**Steps**

1. Pick `$lookup`. Enter system `http://example.org/cs`, code `A`. Run.
2. Pick `$validate-code`. Try `A` (valid) and `NONEXISTENT` (invalid) —
  same behaviour as §3.3 but through the standalone workbench.
3. Pick `$closure`. Read the stateless-server banner (v1 does not
  persist closure names).
4. Pick `$batch-validate-code`. First set **Target ValueSet** to
  `http://example.org/vs/batch-mixed` (the seed's `ex-vs-batch-mixed`
  envelope — see §2.3; the input is HTML5 `required`, so submitting
  with it empty just fires the browser tooltip and never reaches the
  server). The form v1 renders exactly 3 rows — enter:
  - `http://example.org/cs` / `A` (valid)
  - `http://example.org/cs` / `X` (invalid — unknown code)
  - `http://example.org/cs/source` / `A` (valid, different CS, still
    inside the envelope)

   Submit.

**Expected**

- The 7-op selector is a `<nav role="navigation">` (NOT `role="tab"` —
that was the Grupo A bug, fix in `61bfc4f59`).
- Batch-validate immediately renders a skeleton table with 3 rows.
Each row loads its own result via HTMX polling, at most 8 in-flight
(§4.3). Rows resolve independently — do not wait for all three.
- After each row resolves, its skeleton is swapped for a completed row
with the result badge (`valid` / `invalid` / `error`). Both `A` rows
end up `valid` (the target envelope includes both CodeSystems), the
`X` row ends up `invalid`.
- A `<progress>` bar at the top of the table climbs from 0 → 3.

**Red flags**

- Any tab swap loses the operation context (workbench should preserve
filled inputs when possible).
- Batch table shows all rows as "pending" forever (means the
self-terminating progress endpoint hung — see §7.6 impl notes).
- Any row shows a raw JSON stack trace instead of an OperationOutcome.
- Submitting the form does nothing and the browser shows a
"Please fill out this field" tooltip on **Target ValueSet** — that is
the HTML5 `required` constraint on `input[name="target"]` in
`hts-vs-batch-input.html`, not a bug. Paste the target URL from step 4
and resubmit.



### 3.7 §7.7 Import — `/ui/hts/import`

**Demo bundles (paste or save as `bundle-small.json`)**

200 success (same shape as §2.2):

```json
{
  "resourceType": "Bundle",
  "type": "collection",
  "entry": [
    {
      "resource": {
        "resourceType": "CodeSystem",
        "id": "ex-cs-1",
        "url": "http://example.org/cs",
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

207 partial success (one good entry + one entry with `id` but no `resourceType`):

```json
{
  "resourceType": "Bundle",
  "type": "collection",
  "entry": [
    {
      "resource": {
        "resourceType": "CodeSystem",
        "id": "ex-cs-ok",
        "url": "http://example.org/cs/ok",
        "version": "1.0.0",
        "status": "active",
        "content": "complete",
        "concept": [{ "code": "X", "display": "Ok" }]
      }
    },
    {
      "resource": {
        "id": "broken-no-type"
      }
    }
  ]
}
```

**Steps (paste path)**

1. Click **Import** from the topbar or dashboard status strip.
2. Paste an empty string. Submit. → pre-flight 400.
3. Paste `{ not json`. Submit. → pre-flight 400.
4. Paste the **200 success** bundle above. Submit. → 200 success.
5. Paste the **207 partial success** bundle above. Submit. → 207
  partial success.

**Steps (file path — added 2026-08-20, see design §14.6)**

6. Click the **file** radio. The paste textarea hides; the file input
   becomes visible.
7. Save the 200-success JSON above as `bundle-small.json`, then pick
   that file. Notice the paste textarea (now hidden) receives the file
   contents via `FileReader.readAsText()`. Submit. → 200 success, same
   summary strip as the paste path.
8. Click the **paste** radio again. The file input hides, the textarea
   reappears with the file contents still in place (in case you want
   to edit before re-submitting).

**Expected**

- Pre-flight errors are UI-owned — they never hit the backend. The
submit button re-enables after the error banner renders.
- Success shows the number of imported entries and a link back to the
respective browser.
- 207 shows a per-entry breakdown: which entries succeeded and which
failed, with each failure's `OperationOutcome`.
- 413 (paste > 10 MB) is intentionally not covered by a Playwright test;
the Rust ring covers it with a canned mock — you can skip it in the
demo unless you want to paste a large fixture by hand.
- **File path caveat.** The file is urlencoded into the same
  `bundle=…` field the paste path uses, so URL-encoding overhead
  (~33 %) means the effective JSON cap on the file path is ~7.5 MiB
  before HTS returns 413. For anything larger, paste the Bundle
  directly (paste bytes go on the wire verbatim) or split the file.

**Red flags**

- Submit stays disabled after a validation error (means the
`UpstreamHealth` decode broke — Grupo B bug pattern).
- File picker does nothing when clicked (means `import.js` did not
  load — check `/ui/hts/assets/import.js` returns 200 and the
  `<script>` tag is present in `import.html`).



### 3.8 §7.9 Diagnostics — `/ui/hts/diagnostics`

Four tabs: CapabilityStatement / TerminologyCapabilities / /health /
/metrics.

**Steps**

1. Click each tab in turn.
2. Between tabs, watch the URL — it should update via `hx-push-url` so
  you can share a deep link.
3. Force one tab into an error state: temporarily rename the underlying
  endpoint on the binary side (or unset `HTS_UI_UPSTREAM_URL` after
   setting it in §4.4). The failed tab's panel turns red; the other
   three stay green — that is the per-tab isolation contract.

**Expected**

- CapabilityStatement renders as a resource summary (rest interactions,
supported types).
- TerminologyCapabilities lists the loaded `codeSystem[]` (0 on a fresh
boot, N after seed).
- `/health` renders as a small key/value table.
- `/metrics` renders as the raw Prometheus text — this is intentional
(design §7.9 F3: no re-parse, no chart).

**Red flags**

- A red panel disables the other tabs. Isolation broken.
- `/metrics` gets rendered as HTML with `<pre>` escaping missing.

---



## 4. Cross-cutting scenarios

Run these across at least two pages each.

### 4.1 Theme (light / dark / system)

Toggle from the topbar chip on §7.1, §7.4, and §7.6. Confirm:

- No FOUC (Flash Of Unstyled Content) when navigating pages.
- Both themes ship enough contrast for `axe-core` (verified in Phase 3
Playwright; visually spot-check the outcome banners on the operations
workbench — they use accent colors that are easy to underdo).



### 4.2 i18n (en / es / de)

Switch to `es` on §7.1. Click Browse CS. Confirm all labels are in
Spanish. Then switch to `de` and load §7.6. Confirm at least the
`hts-operations-*` keys are populated. Look for:

- Untranslated keys (rendered as `hts-something-else`) — file if you see
any. Key parity is enforced in CI but locale drift is possible during
hand-edit.
- Right-to-left or long-word wraps that push the layout. German has some
very long compound words (`Terminologiediagramm`, etc.) that we should
see fitting in the topbar.



### 4.3 Batch-validate polling (§7.6)

Do this if you want to actually see the fan-out mechanism:

1. In the batch panel, enter 20 rows — a mix of valid and invalid codes
  from the seed.
2. Submit. Watch the skeleton table.
3. Open DevTools → Network. Filter on `/ui/hts/operations/batch-validate/`.
4. Confirm at most 8 requests are in-flight at any moment. As one
  resolves the next queued row's request fires.

This exercises `HTS_UI_BATCH_FANOUT_CONCURRENCY = 8` (compile-time
constant). If you want to see it lower or higher you must rebuild.

### 4.4 Too-costly banner (§7.4)

1. Kill the running HTS.
2. Re-boot with `HTS_MAX_EXPANSION_SIZE=5`:
  ```powershell
    $env:HTS_UI_ENABLED = "true"
    $env:HTS_MAX_EXPANSION_SIZE = "5"
    cargo run --bin hts
  ```
3. Re-seed (§2.1). Open `/ui/hts/value-sets/ex-vs-too-costly` (Name
   **ExampleTooCostlyVS** in the browser, URL
   `http://example.org/vs/too-costly`).
4. On the Expand tab, **clear the** `count` **input** (empty, not `0` — HTS
  only applies the ceiling when `count` is absent).
5. Run.

Expected: a red banner with "expansion too costly" and a **Raise
threshold** form that lets you retry with an explicit `count` — the same
input the workbench filled in step 4 with the default `50`. This confirms
the design's clause that the ceiling is *per-request*, not global.

### 4.5 Degraded state (§7.1)

Two ways to force it.

**Option A** — point the UI at a broken upstream:

```powershell
$env:HTS_UI_ENABLED = "true"
$env:HTS_UI_UPSTREAM_URL = "http://127.0.0.1:9999"   # nothing listens
cargo run --bin hts
```

Load `/ui/hts`. The status cards should show:

- `status = degraded`
- `degraded reason = connection refused` (or similar)
- The dashboard still renders — no white-screen — and the quick-links
are disabled with an `aria-disabled="true"` state.

**Option B** — leave `HTS_UI_UPSTREAM_URL` unset, boot normally, but do
NOT seed. The dashboard reports `loaded systems = 0`. That is technically
"up but empty" (not degraded); the UI should say so, not hide it. If
that number is missing entirely, that is a rendering bug.

---



## 5. Sign-off

You are done when:

- [ ] §7.1 Dashboard — poll works, theme + locale switch clean.
- [ ] §7.2 CS browser — debounce + Load-more + empty state.
- [ ] §7.3 CS detail — 308 redirect to `/lookup`; facts block stays
      above; Lookup / Validate / Subsumes tabs move `aria-current`.
- [ ] §7.4 VS browser + `$expand` — flat pager + tree mode + too-costly.
- [ ] §7.5 CM browser + `$translate` — forward + reverse + no-match.
- [ ] §7.6 Operations — all 7 ops, batch-validate fan-out, closure banner.
- [ ] §7.7 Import — pre-flight, 200, 207, (413 skipped by design).
- [ ] §7.9 Diagnostics — 4 tabs, per-tab isolation, deep-linkable URL.
- [ ] Themes — light + dark on at least two pages.
- [ ] i18n — `en` + `es` on at least two pages; `de` spot check.
- [ ] Degraded — one of §4.5 options triggers the banner without
  crashing the UI.

When every box is ticked, respond "UI OK" and I move to Phase 5 (draft
`edson/docs/hts-ui-discussion.md`).

Any red flag above → file it as a comment on the plan or as a new bullet
here; I will loop with a fix and we re-run the affected block before
signing off.