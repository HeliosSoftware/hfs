# POC — Native SQL-on-FHIR query notebook (#650)

The **recommended** approach from the [#650 evaluation](../../docs/sql-on-fhir-analytics-evaluation.md):
close the loop *inside HFS*, with no new runtime and no external assets. This is the
**Stage 1 + a slice of Stage 2** thin-slice — write a ViewDefinition, run it against `$sql-run`,
and chart the result without leaving the server.

## What it is

A single self-contained page served by the existing `helios-ui` crate at **`/ui/sql/notebook`**.
It is one handler returning inline HTML/CSS/JS — no template, no npm, no CDN. The page's JavaScript
talks only to the **same-origin `POST /$sql-run`** endpoint, so it inherits the server's auth,
tenant, and row/timeout limits for free ("data, not credentials").

What you can do:
- Edit a ViewDefinition (a Patient example is prefilled).
- **Run** it → the result renders as a table.
- **Group by** any column → an inline **SVG bar chart** of the distribution (defaults to `gender`).

## Files changed

- `crates/ui/src/lib.rs`
  - new route `.route("/ui/sql/notebook", get(sql_notebook_page))` (beside the other `/ui/sql/*` routes)
  - new handler `sql_notebook_page()` returning `Html<&'static str>`
  - the inline page in the `SQL_NOTEBOOK_POC_HTML` constant

No new dependencies; no new asset files (nothing added to `crates/ui/assets/`, so the no-CDN
Playwright guard and the raw-embed asset story are untouched).

## Run it

```bash
# 1. Build & start HFS with the UI (default features include `ui`)
cargo run --bin hfs
#   server comes up on http://localhost:8080 (SQLite in-memory by default)

# 2. Seed a few Patients so the query returns rows, e.g. POST a Bundle or use
#    the /ui/batch page to upload Patient resources (any gender mix).

# 3. Open the notebook
#    http://localhost:8080/ui/sql/notebook
#    → click Run → see the table + the gender bar chart.
```

If auth or URL-path multi-tenancy is enabled, put the tenant id in the **X-Tenant-ID** field.

## What it proves (and what it deliberately doesn't)

**Proves:** the #650 loop is achievable natively in the existing Rust/HTMX UI with essentially zero
footprint — no Python, no WASM, no external requests — and the browser only ever sees governed
`$sql-run` output.

**POC shortcuts (see the evaluation's Open Questions):**
- **Aggregation is client-side here.** Fine for a bounded preview, but `$sql-run` *silently truncates*
  at 100k rows, so for a production Stage 2 the group-by must run **server-side** (in a SQLView's SQL)
  to be correct on large views. This POC intentionally shows the browser-side version to make the
  tier problem concrete.
- **No identity / persistence.** There is no per-user auth on `/ui` yet (#320) and no saved-notebook
  object — this page is stateless. Those are the Stage-1/Stage-3 design questions, not solved here.
- **Table capped at 200 rows** for display only.
