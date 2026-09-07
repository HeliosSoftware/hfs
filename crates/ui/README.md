# helios-ui — HTMX-first web UI for HFS

A server-rendered, **HTMX-first** web UI for the Helios FHIR Server. It is a thin
Axum library crate that owns templates, static assets, and view handlers, and is
mounted by the `hfs` binary as a sub-router under `/ui`.

This document is both the discussion doc and the **rules of the road**: when we
build UI in this codebase, this is where things go and why. For the operational
summary — routes, config, test commands — see the `/work-with-ui` skill
(`.claude/skills/work-with-ui/SKILL.md`).

---

## Approach: server-rendered HTMX, not a SPA

We render HTML on the server (Rust) and use [htmx](https://htmx.org/docs/) for
partial page updates. Handlers return **full pages** on hard navigations and
**HTML fragments** on `HX-Request`s. State lives on the server; the client stays
thin.

**There is no React, Vue, Svelte, Alpine, or jQuery here — and no bundler, no
npm dependency, and no build step for the browser code, with one narrow,
documented exception** (see "Assets: vendored & embedded" below). Two
third-party scripts are vendored: htmx, and — as a prebuilt, never-built-here
bundle — CodeMirror 6, mounted over the ViewDefinition JSON editor (#753) and
the SQL Queries/SQL Views editor panes (#838). Everything else under
`assets/` is hand-written vanilla JS in an IIFE. Do not introduce a
framework.

Why, over a SPA + JSON API:

- **Stays close to the FHIR logic we already have.** The UI calls into the
  existing workspace crates (`helios-fhir`, `helios-fhir-validator`,
  `helios-persistence`, `helios-observability`) and renders the result. There is
  no second copy of the domain model in a browser client, and no client-side view
  state to keep in sync.
- **No duplicated API surface.** We do not add a browser-facing JSON API.
  htmx consumes HTML fragments, so the FHIR REST surface stays clean and
  UI-agnostic.
- **Progressive enhancement.** Because the server renders real HTML at real
  URLs, the UI degrades to working full-page loads when JavaScript is absent —
  asserted by the `nojs` Playwright project, not just promised. See
  [HATEOAS](https://htmx.org/essays/hateoas/) and
  [Hypermedia Systems](https://hypermedia.systems/).
- **Locality of Behaviour.** Behavior is co-located with markup via `hx-*`
  attributes rather than scattered across JS files. See
  [Locality of Behaviour](https://htmx.org/essays/locality-of-behaviour/).

Keeping the UI in a **separate crate** from `helios-rest` preserves the clean
FHIR REST surface and lets the UI be feature-gated off (`--no-default-features`
or the `headless` feature on `hfs`) for headless deployments.

---

## Templating decision: Askama (finalized)

We use **[Askama](https://docs.rs/askama)** — Jinja2-like templates that are
**compiled and type-checked at build time**, with **auto-escaping** on `.html`
templates.

Trade-offs weighed:

| Engine | Checked at | HTML lives in | Verdict |
|--------|-----------|---------------|---------|
| **Askama** (chosen) | **compile time** | template files | Type-checked, auto-escaping, keeps markup out of Rust. Templates that reference missing fields fail the build. |
| [Maud](https://maud.lambda.xyz/) | compile time | **Rust macros** | Fast and type-safe, but markup lives *in Rust source* — in tension with our "no HTML in Rust" rule. |
| [Minijinja](https://docs.rs/minijinja) | runtime | template files | Flexible / hot-reloadable, but template errors surface at request time, not build time. |

Askama wins because it keeps markup in template files (satisfying the rule
below) **and** fails the build on template errors, matching the rest of this
codebase's compile-time-correctness bias. Auto-escaping gives us
[XSS](https://cheatsheetseries.owasp.org/cheatsheets/Cross_Site_Scripting_Prevention_Cheat_Sheet.html)
protection by default — `{{ value }}` is HTML-escaped unless explicitly marked
safe.

---

## Assets: vendored & embedded, never a runtime CDN

htmx, CSS, fonts, and the per-page scripts are **vendored** under `assets/`
(htmx pinned at `htmx.org@2.0.4`) and **embedded into the binary** at compile
time with [rust-embed](https://docs.rs/rust-embed), served from `/ui/assets/*`
by [axum-embed](https://docs.rs/axum-embed) with br/gzip/deflate negotiation.

**Never hotlink a CDN in production.** A healthcare server may run offline or
air-gapped, and a runtime CDN dependency is a supply-chain risk. Embedding also
keeps `hfs` a single self-contained binary — no asset directory to ship
alongside it. `e2e/tests/no-cdn.spec.ts` makes this executable: it fails if any
page issues an off-origin request.

`debug-embed` is on, so **debug builds embed too** — editing a file under
`assets/` or `templates/` needs a rebuild to take effect. Assets are served with
`Cache-Control: no-cache` plus a content-based `ETag`, so unchanged assets come
back as a cheap `304` while a rebuilt one is always re-fetched rather than served
stale.

To update htmx, replace `assets/htmx.min.js` with the new pinned release and
note the version bump in the commit.

### The one exception: a vendored, prebuilt bundle

"No bundler" (above) is the default, not an absolute: a third-party script that
is a real parser or grammar — not a widget — cannot reasonably be hand-written
in the style every other asset in this crate uses. For that narrow case:

> Third-party browser code may be vendored as a prebuilt single-file bundle produced by a
> documented, checked-in, one-off script under `crates/ui/vendor/`; pinned versions and a
> lockfile are committed alongside it; the script is never executed at build time or in CI;
> the resulting bundle is never loaded from a CDN; and the bundle ships with its license
> banner intact.

CodeMirror 6 is the first, and so far only, case this applies to (#753,
extended by #838): [`crates/ui/vendor/codemirror/`](vendor/codemirror/README.md)
is the vendoring ritual (pinned npm dependencies, a committed lockfile, a
rollup + terser recipe run by hand, never by `cargo build` or CI); its one
output, [`assets/vendor/codemirror.bundle.js`](assets/vendor/codemirror.bundle.js),
is the vendored bundle itself — embedded and served exactly like every other
asset in this crate (above), nothing bundler-specific about how it ships. The
license of every bundled package, including `lezer-fhirpath` (MIT, declared
in its published README rather than `package.json`), is documented with its
citation in the vendor README's "What's bundled" table (#821) and carried in
the bundle's own banner comment. It backs the ViewDefinition JSON editor on
`/ui/sql/view-definitions` and the SQL pane editors on `/ui/sql/queries` and
`/ui/sql/views`; see
[`docs/viewdefinition-editor-evaluation.md`](../../docs/viewdefinition-editor-evaluation.md)
for the full evaluation this amendment is drawn from. The ViewDefinition
editor also talks to `POST /ui/sql/view-definitions/lint` (#753, #820, #821),
the CodeMirror linter's structural + FHIRPath-syntax check — a plain
JSON-in-JSON-out endpoint of its own (no htmx swap), unrelated to the run
preview below. The request body is the raw ViewDefinition JSON; the response
is `{"diagnostics": [...]}`, one element per
`helios_sof::lint::Diagnostic` with two additions the handler itself builds:
`message` is **not** `Diagnostic::message` (that field is always English —
`$sql-run`, `sof-cli`, and `pysof` all use it verbatim) but the request's
negotiated-locale rendering of `code` + `args` against the `vd-lint-*`
catalog in `locales/*/main.ftl` — negotiated exactly like every page (`?lang=`
override → `hfs_lang` cookie → `Accept-Language` → `en`, see
[`i18n.rs`](src/i18n.rs)); and each element of `fixes` (one-click structural
edits — rename/remove a key, set a string value, addressed by RFC 6901
pointer, never a text position, since this handler never sees source text)
carries an additional `label`, translated the same way from the matching
`vd-fix-*` catalog key. `args`, `span`, and every other `Diagnostic`/`Fix`
field pass through unchanged, so a client that wants the raw value behind a
translated sentence still has it.

`vd-editor.js` turns each diagnostic's `fixes` into a `Diagnostic.action`
(#821) — a button in the hover tooltip and, with several diagnostics at once,
the bottom lint panel — whose `name` is the fix's own `label` and whose
`apply` edits the document by the fix's RFC 6901 `pointer`, resolved against
the browser's *live* syntax tree at the moment it is actually clicked (never
against the tree the diagnostic first arrived against): a `pointer` that no
longer resolves to the shape its kind expects — the document changed in
between — makes the fix a no-op rather than mangling unrelated text.
`rename-key` replaces just the content of the named property's key string;
`remove-key` deletes the property together with whichever neighboring comma
would otherwise dangle, never leaving a stray blank line or disturbing any
other property's indentation; `set-string` replaces just the content of the
pointed-at value string, escaping `"`/`\`. Every edit is one transaction
(`userEvent: "lint.fix"`, one `Ctrl+Z` undoes it) that relaunches the lint
(`forceLinting`) once applied. **Ctrl+.** collects the actions of every
diagnostic under the cursor (`@codemirror/lint`'s own `forEachDiagnostic`):
exactly one applies it directly, more than one opens the lint panel
(`openLintPanel`), none falls through to `.`'s normal self-insertion;
`lintKeymap` rides along in the same keymap, adding F8 (next diagnostic) and
Ctrl-Shift-M (open the panel). Submitting `#vd-editor-form` as Save (not
Duplicate) while the most recently *completed* lint pass still has at least
one `error`-severity diagnostic — local JSON syntax errors included — pops a
native, plural-correct `window.confirm` (`data-msg-save-errors-one`/`-other`
on `#vd-editor-grid`, Fluent `vd-save-with-errors-one`/`-other`); cancelling
it keeps the page as it is with focus back on the editor, and warnings alone
(or no lint result yet) never prompt at all. None of this requires anything
beyond `window.HfsCodeMirror` — no JavaScript at all means Save always just
submits, exactly as it does today.

The editor also talks to `POST /ui/sql/view-definitions/complete` (#821),
a sibling of `/lint` following the same "the browser knows syntax, the
server knows FHIR" split: a CodeMirror completion source only has to say
*where the cursor is*, and this endpoint answers *what fits there*. No
tenant, no htmx swap, no locale — plain JSON in, JSON out, and the FHIR
version comes from the request's negotiated `RequestVersion` (`?version=`/
cookie), never the body. Two request shapes, tagged by `kind`:

- `{"kind": "key", "pointer": "...", "present": [...]}` — completion at a
  structural JSON node. The response's `items` is
  `helios_sof::lint::node_keys(pointer)` (the same key model `/lint`'s
  `unknown-key` check is built on) minus whatever `present` already has, each
  one `{"label": <key>, "kind": "key", "detail": <"string"|"number"|
  "boolean"|"string[]"|"object"|"object[]"|"other">, "required": <bool>}`;
  `from` is always `0`. A pointer the key model doesn't recognize answers
  `{"from": 0, "items": []}`, never an error.
- `{"kind": "fhirpath", "pointer": "...", "document": {...}, "expression":
  "...", "cursor": N}` — completion inside a partial FHIRPath expression.
  `expression` is the field's current full text; `cursor` is a **char**
  offset (not a UTF-8 byte offset) into it, and anything after the cursor is
  ignored. `document` is capped at 1 MiB — larger bodies are rejected with
  `400`, never evaluated. The response's `from` is the char offset where the
  token being completed starts (so a client splices its choice in at
  `[from, cursor)`), and `items` mixes up to four candidate kinds depending
  on where the cursor sits:
  - Right after a `.` ("member mode"): `element` candidates (the resolved
    type's own children) plus every `function`.
  - Right after `%` (or the token itself starts with `%`, "constant mode"):
    only `constant` (the document's own `constant[]` entries) and `variable`
    (the FHIRPath environment variables) — `from` backs up to point at the
    `%`.
  - Anywhere else — start of expression, after `(`/`,`/an operator/
    whitespace ("root mode"): `element` (children of the current
    `%context`) plus `function`, `constant`, and `variable` together.
  - Inside an unterminated `'...'` string literal: `items: []` always.

  `element` items carry `detail` as the FHIR type (`"HumanName"`, `"string"`,
  or `"Quantity | CodeableConcept | ..."` for an un-narrowed choice element),
  suffixed `[]` when the element repeats, and `doc` from the schema's `short`
  text when the pack carries one. `function` items carry `detail` as the
  catalog's call signature (`"where(criteria)"`). `constant` items are
  `"%name"`, `detail` the type derived from whichever `value[x]` key is
  present (`valueString` → `"string"`, `"unknown"` if none is). `variable`
  items are `"%name"` with no `detail`. Filtering candidates by whatever
  prefix the user has typed is the browser's job — every list here is
  unfiltered.

  The type a partial expression resolves against is a heuristic, not a
  FHIRPath evaluation (nothing here ever evaluates one): the root type is
  `document.resource` when the request's version registry resolves it,
  narrowed by each ancestor `select`'s own `forEach`/`forEachOrNull`/(first
  element of) `repeat` in order (a `select`'s own iteration expression is
  resolved against the context *above* it, never itself); a member chain
  (`a.b(c).d[0]`) is split into top-level, paren/bracket/string-respecting
  `.`-segments and walked left to right — an identifier looks up that name's
  child type; `ofType(T)`/`as(T)` resolves to `T` outright (narrowing an
  otherwise-ambiguous choice element like `value[x]`); `extension(...)`
  resolves to `Extension`; `%resource`/`%rootResource` resolves to the root
  type and `%context` to the current context type; the type-preserving
  functions `where`, `first`, `last`, `tail`, `skip`, `take`, `single`,
  `exclude`, `distinct`, `union`, `intersect`, and `trace` pass the incoming
  type through unchanged; a trailing `[...]` indexer never changes the type;
  and anything else (`resolve()`, `select(...)`, a literal, an operator, an
  undeclared `%const`) resolves to an unknown type — which drops `element`
  from the response but never `function` (or, in root mode, `constant`/
  `variable`).

All three SQL on FHIR playgrounds — `/ui/sql/view-definitions`,
`/ui/sql/queries`, and `/ui/sql/views` — share one results-card partial,
`partials/sql_run_results.html`, and one `$sql-run` preview contract (#752,
generalized in #839): each page's own render nests the partial as a template
field, and each has its own `POST …/run` fragment endpoint
(`/ui/sql/view-definitions/run`, `/ui/sql/queries/run`, `/ui/sql/views/run`)
that runs the editor's *posted* text — saved or not — through `$sql-run` and
renders that same partial as its whole response, with `hx-swap-oob`
attributes so only the results card and its meta move. View Definitions'
`/run` always answers `200` (htmx does not swap `4xx`/`5xx` by default)
except for a malformed request body; the Library-backed `/run` endpoints
(#841) answer `200` unconditionally instead, since their own by-hand form
parse (below) never fails to extract. Each caller supplies its own surface —
the fragment URL, the id of the form to `hx-include`, the results heading
and failure-prefix i18n keys, and an optional "Export as files" action (SQL
Queries only, once the Library has a saved id) — so the partial itself never
branches on which page is rendering it.

`RunResultsState` (`crate::lib`) has a fourth arm beyond
Success/Failure/Empty: `Waiting(message)` (#841), a SQL Query's own
"a declared, required parameter has no value yet" pause — rendered as a
plain, non-`--warn` notice (no `data-error-line` either, there is no SQL
error to tint), with the same OOB shape `Failure` uses otherwise (the stale-
meta relabel, no `#run-results` swap, so the previous table stays on
screen). Only SQL Query's own `/run` and `?…&saved=1` render can produce it;
View Definitions and SQL View never do.

None of the three pages has a Run button: the editor card is always open,
and the results region below it wires straight to `/run` with plain `hx-*`
attributes — no JavaScript beyond what already ships. The results region's
own empty shell fires one `hx-trigger="load"` request when the page opens
with nothing to show yet (a fresh selection, or the `new` starter document),
and the editor's `json`/`sql` textarea reposts on `hx-trigger="input changed
delay:500ms"` as it changes — CodeMirror's mount already dispatches `input`
on every edit, so this needs no mount-specific wiring. A failed run leaves
the editor's text untouched and the last successful table on screen,
relabelled "last successful run" via an out-of-band swap of just its meta;
when the server's message names a parse error's line (sqlparser's own
`… at Line: N, Column: M`), the notice also carries `data-error-line="N"`
(SQLite execution errors carry no line — those notices go out without the
attribute). On `/ui/sql/queries` and `/ui/sql/views`, `sql-editor.js` reacts
to that same `#run-notice` swap and tints the named line in the mounted
CodeMirror editor (`.sql-editor__error-line`, #839) — a decoration only, it
never edits the document or touches the textarea. With JavaScript disabled
there is no live preview at all: Save's own redirect (`?vd=<id>&saved=1` /
`?lib=<id>&saved=1`) is what renders the just-stored resource's results,
server-side, once.

**Unknown-table lint (#842/04, Library-backed pages only).** `crate::
sql_library_run` decides, in this order, ahead of `$sql-run`: (1) a SQL
View's own non-empty `parameter[]` (#841); (2) the SQL reads a table no
declared `relatedArtifact[depends-on]` label names, case-insensitively
(`sql_libraries::unknown_tables`, `helios_sof::sqlquery::{scan_sql,
undeclared_tables}`) — never calls `$sql-run` at all; (3) a SQL Query's own
declared, required parameter with no value (`Waiting`, above); (4) only once
every check passes does it call `$sql-run`. `?…&saved=1`'s own server-side
run and the `document` endpoint's own no-JS re-render apply the same
unknown-table check (steps 2 and, for `?…&saved=1`, 1 and 3 too) before
deciding what to show in place of results. `crate::LibRunNotice` is the
enum that decision produces — `Standard(RunResultsPartial)` for every other
case, or `UnknownTables(crate::UnknownTablesNotice)` — rendered in place of
`partials/sql_run_results.html` (never edited to add this; see its own
header comment) by `partials/lib_run_unknown_tables.html`, the identical
OOB shape as a `Failure`. The notice text is the first table's own full
sentence ("Unknown table {name} — line {line}. Declare it under Reads from
or fix the name. …") then a short sentence per additional table; `#run-
notice`'s `<p>` carries `data-error-line` (the *first* table's own line) and
`data-diagnostics` — a JSON array of `{from, to, message, table}` objects
(Unicode-character offsets into the posted SQL, from `helios_sof::sqlquery
::TableRef::position`) `sql-editor.js` feeds straight to `@codemirror/lint`'s
`setDiagnostics` (below). The same tables also enter the *Reads from* card's
own signature as `?name` hints (`sql_libraries::tables_signature_with_
unknown`, mirroring the Parameters card's own undeclared-placeholder
hints) and render as their own `.tag--failed` "Unknown table" rows there,
each with a *Declare {name}* button (opens *Add table* and pre-fills its
alias, see "Tables panel" below).

Beside the editor sits a guided-form card (#843, `.editor__grid--stretch` in
`assets/app.css`): the two cards stretch to match each other's height, capped
at 70vh, each scrolling inside its own content area past that (the editor's
own form carries the shared `editor__doc` class, #840, for that sizing — see
"Client-side scripts" below). Both are one document — the editor's — with two
views onto it: the card is built inline, server-side, on the page's own first
paint from the same document the textarea shows (`editor::build_form_pane`,
the `pane=form` engine `POST /ui/editor/render` also serves, called directly
rather than fetched, so there is no post-`load` layout shift); a guided-form
edit posts back through `assets/editor-form.js` (`pane=form`) and lands in
the editor as one minimal transaction (`assets/editor-pair.js`'s
`minimalChange`, a common-prefix/common-suffix diff, #840 — extracted from
`vd-editor.js`'s own original #843 implementation) — undoable, and without
moving the caret or the scroll; an editor change that transaction did not
itself just cause is parsed 600ms after the last keystroke and, if its
canonical JSON actually moved, re-requests the panel alone.
`editor::EditorFormPane`'s `is_view_definition`
flag also selects this page's own single-line legend
(`vd-form-legend-live`) in place of the Resource Editor's two-line one — Save
here stays permissive (#752, above), so "checked on save" would be
misleading. The per-keystroke pass that builds the card's rows is the same
schema-driven validator the Resource Editor runs (`Validator::validate_sync`
over the embedded core packs) — `ViewDefinition` is a resource type in every
enabled version's pack, `select`/`column`/`where`/`constant` and their
bindings included (`crates/fhir-validator/tests/pack_smoke.rs` exercises this
per version, R4 through R6) — folded together with
`helios_sof::lint::lint_view_definition`'s own SQL-on-FHIR-specific
diagnostics a structural check alone cannot see (FHIRPath syntax, undeclared
constants, duplicate column names, a `select` with no output, more than one
iteration directive); see `SOF_ONLY_LINT_CODES` in `editor.rs` for exactly
which lint codes fold in and why the rest are excluded (double-reporting what
the validator already covers).

The two cards also stay linked while you point at them: hovering or focusing
a row paints the lines its node occupies in the editor, and moving the cursor
in the editor (click, selection, or typing) lights the row for whichever node
it now sits in — the same idea as the Resource Editor's own `editor-sync.js`,
reimplemented in `assets/editor-pair.js` (#840, extracted from
`vd-editor.js`'s own original #843 implementation) because this page's JSON
pane is a CodeMirror `EditorView`, not the server-rendered
`.json-line[data-jpath]`
markup that link is built on. Both directions resolve a node by walking the
browser's own CodeMirror syntax tree — a row's dotted path
(`select.0.column.0.path`) down to its node, or a cursor position up through
its ancestors back to a dotted path — never by re-parsing the text by hand;
a CodeMirror `StateField` of line decorations (`cm-line--hit` in `app.css`)
carries the editor-side paint, reset the moment the document itself changes.
Reveal stays inside each pane's own scroll container (`EditorView.
scrollIntoView` for the editor, the same container-only scroll `editor-sync.
js` uses for `.editor-tree`) — never the page's.

**`needs-js`** (`assets/app.css`, `@layer components`): hides an element
until `<html class="js">` — a class `theme.js` sets synchronously, before
first paint (below) — so a page that renders a JavaScript-driven card inline,
server-side, on its own first response (the guided-form card here) does not
show it with no client-side loop wired to it yet. A consumer that needs a
shown-state `display` other than the browser default supplies its own
`html.js …` override next to its own layout rules (`.editor__grid--stretch`'s
own `display: flex` override is the example). Not View Definitions' own: SQL
Query and SQL View's Details section (below) renders its guided-form card
the same inline, server-side way and carries the identical `needs-js`.

### Details (#840): the SQL Query/SQL View Library minus its SQL attachment

`/ui/sql/queries` and `/ui/sql/views` (`pages/sql-library.html`, one template
keyed by the route's own `LibraryKind`) give each stored `Library` a Details
section — the same JSON editor + guided-form pairing described above, over a
different document: the `Library` with its `application/sql` `content[]`
attachment stripped out (`sql_libraries::strip_sql_attachment`), since the
SQL card beside it owns that attachment on its own. `crate::
render_lib_details_pane` calls the shared engine with `hidden: &["content"]`
(so the guided form neither shows nor offers to mutate it) and `legend:
"sql-library"` (its own two-line legend — "checked on save" here names the
Library type coding and the SQL attachment, not the generic constraints/
terminology promise `Legend::Resource` makes, since `HFS_VALIDATION_MODE` is
off by default). `assets/sql-library-details.js` mounts CodeMirror with the
shared JSON language/highlight preset (no injected FHIRPath grammar, unlike
`vd-editor.js`) and hands the mounted view to `editor-pair.js` with `fields:
{ hidden: "content", legend: "sql-library" }` — the same host View
Definitions uses, so the sync, the validity chip, and the row↔editor
cross-highlight behave identically on both pages.

One Save posts both cards; the server fuses them
(`sql_libraries::embed_sql`, replacing the first `application/sql`
attachment or appending one when Details carries none — a SQL attachment
typed by hand into the Details JSON always loses to the SQL card) and
rejects a document whose `type.coding` names the other kind — `sql-view`
saved from SQL Queries, or the reverse — with a warning naming the route's
own expected code, before anything is written. Without JavaScript both
textareas — the Details JSON one carries `form="lib-editor-form"`, HTML5
form-associated even though it lives outside that `<form>` in the DOM — post
together in one plain submit; the guided-form card stays `needs-js`-hidden
and the grid collapses to the JSON card alone, exactly like View
Definitions' own card does.

### Parameters card (#841, SQL Query only)

Between the SQL card and the results region, `/ui/sql/queries` renders one
more card — `partials/sql_parameters_card.html` — that SQL View never shows:
`LibraryKind::declares_parameters` (`true` for SQL Query, `false` for SQL
View, whose SQLView profile fixes `Library.parameter` to `0..0`) is the one
field the template gates on, never an `if` on the route's own `code`. The
card holds one `.field` per declared `Library.parameter[use=in]` entry (the
shared `partials/sql_parameter_fields.html` macro #837's SQL Export builder
also uses, now with an optional `form` argument so its inputs — physically
outside `#lib-editor-form` — still submit and live-preview with it, HTML5
form-associated the same way the Details JSON textarea already does), the
`:name` placeholders the SQL uses but does not declare as one-click
*Declare* hints, and an *Add parameter* `<details>` that writes a fresh
declaration into the Details document.

**Signature and OOB (#841).** The card carries a hidden `params_sig` field —
every declared parameter's `name:type` pair, declaration order, then one
`?name` per undeclared-placeholder hint, comma-joined
(`sql_libraries::params_signature`) — and every `/run` submission echoes it
back. `crate::analyze_params` recomputes the same signature from the posted
document/SQL on every request; only when it differs from what the browser
sent does the response carry the card again, as its own `hx-swap-oob`
companion (`partials/lib_run_fragment.html`, which nests
`sql_run_results.html`'s render next to an optional `LibParamsCard` render —
the same "nest one `Template`'s output inside another's" idiom the rest of
this crate uses instead of concatenating markup by hand). A hint appearing
or resolving changes the signature (so the card refreshes); typing a
parameter *value* never does, so the card is never swapped mid-keystroke and
never loses focus.

**Live-run values and bindings.** A value field's own `input` event bubbles
up to the card's own `hx-post`/`hx-trigger="input delay:500ms"`, which
`hx-include`s the whole `#lib-editor-form`. The `/run` handler
(`crate::sql_library_run`) reads every `param:{name}` off the raw form body
(`crate::parse_lib_run_form` — needed because a `#[derive(Deserialize)]`
struct cannot express an arbitrary set of keys, the same reason
`sql_export::start` parses its own `param:{reference}:{name}` fields by
hand) and, for each declared parameter with a non-empty value, supplies a
`{name, type_code, value}` binding to `$sql-run` via `ConformanceSource::
sql_run`'s own `bindings` parameter (#841's own seam from the SQL scanning
ticket) — a value left blank on a defaulted parameter is omitted so the
server applies its own default, never sent as an empty string. A value that
fails to bind its declared type is sent anyway and left to `$sql-run`'s own
error message; this card never duplicates that validation client-side.

**The "waiting" pause.** A declared, default-less parameter with no
non-empty value blocks the run before `$sql-run` is ever called:
`RunResultsState::Waiting` (above) names every missing `:name` in one
notice. `?…&saved=1`'s own server-side run applies the identical gate
(always against an empty value map — the just-saved redirect never carries
the POST's own `param:*` fields forward), so a parameterized query's first
paint after Save shows "waiting" rather than a stale or empty table.

**`POST /ui/sql/queries/document` and `POST /ui/sql/views/document`**
(`crate::sql_library_document`, both routes exist so SQL View's own
programmatic mutations #842 will add later share the endpoint shape, though
SQL View's Parameters card never renders to trigger one): the *Add
parameter*/*Declare* mutations, applied to the **submitted** Details
document — never a stored resource — and handed back unsaved, matching
Save's own "reject without touching storage" contract. `op=add-parameter`
with `param_name`/`param_type` (the *Add parameter* panel's own submit) or a
`declare_param={name}` field alone (a *Declare :name* button's own name/
value pair, implying type `string`) both reach
`sql_libraries::add_parameter`, which validates the name against the
`sql-name` invariant (`^[A-Za-z][A-Za-z0-9_]*$`), rejects a duplicate
`use=in` name, and checks the type against
`helios_sof::sqlquery::BINDABLE_PARAMETER_TYPES` before appending
`{name, use: "in", type}` to `parameter[]` (creating it if absent). An
`HX-Request` gets the Parameters card alone — `data-document` on its root
carrying the updated (Details-shaped, SQL-attachment-free) JSON on success,
absent on a validation failure (the message lands in the *Add parameter*
panel's own field instead, panel left open); no `HX-Request` gets the whole
page re-rendered around the same (possibly mutated) document, unsaved,
exactly like Save's own validation-error path
(`crate::render_lib_document_page`, shared by both). Both buttons are real
`type="submit"` elements with a `formaction` to this endpoint, so *Add
parameter*/*Declare* work with no JavaScript; their own `hx-post`/
`hx-target="#lib-params"`/`hx-swap="outerHTML"` intercept that same submit
when JavaScript is available. `assets/sql-library-panels.js` is the one
JavaScript-only step neither path handles alone: on `htmx:afterSwap` of a
`#lib-params` carrying `data-document`, it hands that text to
`window.HfsSqlLibraryDetails.host.setDoc()` — the Details JSON pane's own
undo-tracked host, exposed by `sql-library-details.js` (below) rather than
discarded — so the mutation becomes one Ctrl+Z-able transaction that also
refreshes the guided form and re-fires the live run.

### Tables panel (#842, both kinds)

Below the SQL card (and the Parameters card, on SQL Query), both routes
render a two-column `#lib-tables-panel`: `section.card#lib-tables` (left,
`partials/sql_tables_card.html`) and `section.card.table-card#lib-columns`
(right, `partials/sql_columns_card.html`), present whenever a Library is
selected — including `?lib=new`'s starter document. `crate::TablesAnalysis`
(`crate::analyze_tables`) computes everything both cards need from one
(document, already-fetched Library list, already-loaded `$sql-export` job
list) triple, so *Reads from*'s own signature and *Used by*'s own rows can
never disagree about what the document currently declares; `crate::
full_tables_analysis` is the "nothing already fetched" path every mutation
endpoint uses, while the page's own render reuses the rail's Library list
instead of fetching it twice (NF1).

**Reads from.** One row per `relatedArtifact[type=depends-on]`, in document
order: an alias (`label`, or a placeholder when absent) and the target it
resolves to. `crate::resolve_table_reference` mirrors `crates/rest/
.../graph.rs`'s own `StorageArtifactFetcher::fetch` (imitates, never
replaces, the server's own resolution) — `sql_libraries::dependency_lookup`
tells a relative `Type/id` reference from an absolute canonical (optionally
`|version`, compared only when pinned); a `Type/id` is read directly, a
canonical is searched among stored ViewDefinitions first
(`search_page("ViewDefinition", url=…)`, the more common dependency), then
matched against the already-fetched Library list by `url`/`url|version`
(`sql_libraries::matches_reference`) — never a second request. `sql_libraries
::classify_table_artifact` turns what resolved (or didn't) into a
[`TableTarget`]: a ViewDefinition or `sql-view` Library become a chip and a
link (`?vd=`/`?lib=`, the artifact's own `name`); nothing answering to the
reference is `NotFound` (`.tag--failed` "Not found"); a `sql-query` Library
or any other resource type is `NotATable` (`.tag--failed` "Not a table") —
distinct messages, since the first is a bad reference and the second names
something the server itself would refuse to read from. Resolution runs once
per render, is memoized per request by `dep.resource` (a repeated dependency
never triggers a second lookup), and only ever runs again — on `/run` — when
the posted `tables_sig` no longer matches; #842/04 extends that signature
with one `?name` per unknown table the SQL reads
(`sql_libraries::tables_signature_with_unknown`), so a table appearing or
resolving re-renders this card too. *Remove* is a form-associated
`type="submit"` per row (`name="table_label"`, its own exact `label`) —
present means clicked, the same "the field's own presence is the operation"
idiom the Parameters card's *Declare* buttons use.

**Used by.** Two groups, artifacts first then exports, both name-sorted:
`sql_libraries::used_by_artifacts` scans the same in-memory Library list for
any (other) Library whose own `relatedArtifact[depends-on]` names the
selected artifact's identity (`id`, `url`, or `url|version`) — chip `SQL
Query`/`SQL View` from its own type coding, link to its own page;
`sql_libraries::used_by_exports` scans the current user's `$sql-export` jobs
(`sql_export::jobs_for_used_by`, same tenant) for a `subjects[].reference ==
"Library/{id}"`, most recent first, linking to `/ui/sql/export/{jobId}`. A
`?lib=new` document with no `id` can still be found by artifacts through its
own `url`, though never by an export — nothing has been saved yet for one to
reference — and an empty result reads "Nothing uses this yet."

**Add table.** A native `<details>` opened by the head's own button, holding
`combobox.html`'s field in single-value mode (`max="1"`, `form=
"lib-editor-form"` — see "Client-side scripts" below) against `/ui/lookup/
table-options` (see the routes table), and an Alias field
(`table_alias`, `pattern="[A-Za-z][A-Za-z0-9_]*"`) that `sql-library-
panels.js` autofills from the chosen option's own name. `POST
.../document`'s `op=add-table` (`crate::apply_add_table`) resolves `table`
through `crate::resolve_table_target` — the *Add table* field's own extra
gate on top of `dependency_lookup`/`classify_table_artifact`: `table_ref`
must actually resolve, resolve to a ViewDefinition or `sql-view` Library,
and — for a Library — not be the artifact's own id (a `Type/id` reference
alone cannot rule out a self-canonical one) — then validates the alias in
order: present (falling back to the target's own `name`), shape
(`sql_libraries::is_valid_parameter_name`), and uniqueness among the
existing labels, case-insensitively (`sql_libraries::label_declared` — `V`
collides with `v`). On success it appends `{type: "depends-on", label,
resource: R}` (`R` = the target's own `url`, else `Type/id`, never
version-pinned) via `sql_libraries::add_table`; `op`'s validation-error
shape and `remove-table` (`crate::apply_remove_table`, always a silent
no-op for an unmatched label) otherwise mirror the Parameters card's own
`add-parameter`/`document` contract exactly — see "Parameters card" above.

A table the SQL reads but no dependency declares (#842/04, "Unknown-table
lint" above) appends its own row after every resolved one — `<code>` the
name as the SQL spells it, `.tag--failed` "Unknown table", and a *Declare
{name}* button. With JavaScript (`assets/sql-library-panels.js`, event
delegation on `[data-declare-table]`) it opens the *Add table* `<details>`
and fills the Alias field with that row's own name, without submitting —
the target itself still needs picking from the combobox, unlike a
Parameters hint's one-click *Declare*; without it, `crate::build_tables_card`
already opens the panel and pre-fills the *first* unknown table's own name
whenever `options.add` is still its own untouched default. The head's own
`.toolbar__count` still counts only the *declared* rows.

**Columns** (`section.card.table-card#lib-columns`, `partials/sql_columns_
card.html`, filled in for #842/04): the last good run's own column list, in
the result table's own order — `crate::build_columns_card`/`sql_libraries
::analyze_columns`, fed the run's raw JSON rows (`crate::run_sql_preview`
now returns them alongside the stringified table, #842/04) and every
dependency resolved to a ViewDefinition (`crate::
resolve_view_definition_dependencies` — a SQL View dependency never
contributes an origin, out of scope). A column's own name existing in
*exactly one* resolved ViewDefinition's `select[].column[]`
(`helios_sof::sqlquery::TableSchema::from_view_definition`) gives it that
column's own type (`ColumnFhirType::code()`) and an `{label}.{column}`
origin; none or several matching dependencies fall back to inferring a type
from the row values themselves (`boolean`/`integer`/`decimal`/`string`, a
mix or an unrepresentable value becoming `string`, no non-null value at all
"—") with no origin. With no rows yet — every render but a successful
`?…&saved=1` — the card is its own skeleton (heading, a fixed `0` count,
and a per-kind "what the query/view produces" meta and empty body); on
`/run`'s own fragment, `crate::ColumnsFragment` decides between sending the
whole card by OOB (`Filled`, a good run only) or just relabelling
`#lib-columns-meta` to "last successful run" (`Stale`, every other outcome —
NF1: the ViewDefinition resolution above never runs on a request that never
reaches a successful `$sql-run`). Never rendered for View Definitions.

### Client-side scripts

Each is a small, self-contained IIFE. Page-specific scripts load with `defer`;
`json-view.js` and `busy.js` load from the shared layout because their behavior
is used across workspaces — `busy.js` ahead of every page script (defer runs in
document order) because page scripts call into it. `theme.js` loads **without
`defer`**, before first paint, to avoid a flash of the wrong theme. `busy.js`
exports `window.hfsBusy`; the CodeMirror stack (below) exports two more
globals for the same reason — code that exists to be called by other scripts,
not just a closed IIFE.

| Asset | Owns |
|---|---|
| `theme.js` | Light/dark preference: stored choice → OS preference, plus the top-bar toggle. Also marks `<html class="js">` (#843), synchronously, before first paint — the signal `.needs-js` (above) hides against |
| `busy.js` | The shared busy states (#679): `during(buttons, work)` and `region(el, label)` |
| `saved-queries.js` | Saved queries, the visual search builder, the `/_user/settings` read/modify/write cycle, and — on Resources/Search/Saved Queries — writing `rails.<page>` back on an in-page rail click (#754/#755) |
| `editor.js` | The schema-driven editor loop — posts the document to `/ui/editor/render` and swaps in the server's HTML |
| `json-view.js` | Delegated folding and accessibility state for every server-rendered JSON view |
| `combobox.js` | Shared multi-select state, chips, keyboard/ARIA behavior, and progressive fallback upgrade; htmx owns transport and callers own result semantics. `data-combobox-max="1"` (#842, *Add table* only) switches a field to single-value mode — choosing an option replaces the current selection rather than adding to it — and fires `hfs:combobox-select` (`{value, label, name}`, `name` from the option's own optional `data-name`) on every actual choice, for a caller that needs to react to *which* option was picked rather than the whole-list `hfs:combobox-change` every field already emits. `data-combobox-form` (#842) gives every hidden input this field creates the same `form=` attribute its fallback textarea carries — needed only when the field's own fieldset sits outside the `<form>` it submits with, as `sql_tables_card.html`'s *Add table* field does (its siblings are each explicitly form-associated, `form="lib-editor-form"`, rather than DOM descendants of a `<form>` the way every caller before it is). `install()` also runs on every htmx `afterSwap` target (#842/04) — needed the moment the unknown-table lint's own OOB refresh replaces `#lib-tables`, and so its *Add table* field, with a fresh, un-enhanced one straight from the server; `initialize()`'s own `data-combobox-ready` guard makes this safe to call repeatedly |
| `resources.js` | The Resources workspace edit modal and "Create new" |
| `batch.js` | Bundle pick → lazy highlighted previews → execution plan → per-entry outcomes |
| `bulk-export.js` | All Resources, individual resource types, and Since/Custom instant state on the Bulk Export builder |
| `sql-export-form.js` | The SQL Export builder (`/ui/sql/export/new`, #834/#836): the subjects table's type switch, text filter, header select-all, and "n of m selected" count; independently, the CSV header switch's visibility (shown only for `format: csv`, never touching its `checked` state) and the Since custom instant's enabled state and `data-pattern` validation on submit — the same enable-only-for-"custom" rule as `bulk-export.js`'s own Since field, but without its fuller calendar-validity pass, which stays a server-side (`crate::lookup::since_instant`) concern |
| `sql-export.js` | "Copy job id" on Active SQL Exports job cards — reveals the button only when the Clipboard API is available, writes the id, shows "Copied" |
| `history.js` | Version selection and diff requests |
| `nl-search.js` | Natural-language search mode (only loaded when configured) |
| `resource-filter.js` | Shared truncated-name tooltips (type rails and the resource grid) and each rail's scroll-to-selection on arrival — the "Recently used" group itself is server-rendered (#754/#755) |
| `conformance-crud.js` | The conformance viewers' write half (create/edit/delete against the FHIR API) |
| `code-editor.js` | Shared CodeMirror 6 mount helper (#838): textarea-as-source-of-truth sync, aria-label, tabindex, Tab-not-captured, silent degradation — `window.HfsCodeEditor.mount(textarea, options)` — plus the shared JSON token-color preset every JSON-editing page reads (`jsonHighlight()`, #840), exported for `vd-editor.js` and `sql-editor.js` to build their own language/highlight/lint on top of. An `options.completion` array of `CompletionSource` functions (#821) wires `autocompletion({ override, activateOnTyping: true, maxRenderedOptions: 300 })` — the library's own default of 100 silently cut off a real match past the fold (a FHIRPath member chain offers a type's own elements plus the entire function catalog) — plus `completionKeymap` (Ctrl-Space opens the popup manually; Enter/Escape/arrow keys are its own while it is open) ahead of `defaultKeymap` — only `vd-editor.js` passes one; `sql-editor.js` and `sql-library-details.js` are unaffected, and Tab still never indents (no command in `completionKeymap` binds it) |
| `editor-form.js` | The guided-form loop (#843), extracted from `editor.js`'s original: `[data-add]`/`[data-remove]`/`[data-extension]`/`[data-choose]`/`[data-set]`, the add-picker's typeahead, live `$expand` — driven against a caller-supplied `root` and `host` (`{ getDoc, setDoc, renderUrl?, fields? }`) instead of page ids, so it works over any document a host owns; `host.fields` (#840) adds constant extra fields (e.g. `hidden`/`legend`) to every request. `window.HfsEditorForm.attach(root, host)` |
| `editor-pair.js` | The shared host between a CodeMirror/textarea JSON editor and the guided-form card beside it (#840, extracted from `vd-editor.js`'s original #843 implementation): two-way JSON↔form sync (a form-driven change lands as one minimal common-prefix/common-suffix transaction, tagged so the sync listener skips its own echo; an editor change 600ms after the last keystroke re-requests the panel alone when its canonical JSON actually moved, or flips the `.editor-validity` chip to "Invalid JSON" when it does not parse), and the row↔editor cross-highlight (a `StateField` of line decorations for a hovered/focused row, a debounced cursor listener that marks the row for whichever node the caret sits in). Adds its own CodeMirror extensions onto an already-mounted `EditorView` via `StateEffect.appendConfig`, so callers pass nothing pair-specific into `HfsCodeEditor.mount`. Falls back to driving the plain `<textarea>` when no `EditorView` is given. `window.HfsEditorPair.mount({ textarea, view?, grid, fields? })`, consumed by `vd-editor.js` on `/ui/sql/view-definitions` and the Library Details editor (#840) |
| `vd-editor.js` | The ViewDefinition editor on `/ui/sql/view-definitions`: JSON + injected FHIRPath language and highlighting, fold, and the async server lint (#753, generalized onto `code-editor.js` in #838); hands the mounted `EditorView` to `editor-pair.js` (#840) to drive the guided-form card beside it. `vdCompletionSource` (#821, `code-editor.js`'s `completion` option) classifies the cursor against the browser's own syntax tree — inside a `PropertyName` string, or in an `Object` at a "new key" gap (right after `{`/`,`, or right after a `Property` with no comma of its own yet — a JSON-error-recovery state reached mid-edit as often as a genuinely new key) → `POST .../complete` with `kind: "key"`; inside the content of a `String` the same injection rule `nestFhirpath` already applies to (`path`/`forEach`/`forEachOrNull`/a `repeat` element, no `\` in the string) → `kind: "fhirpath"`, `document` the editor's own currently-parsed JSON (a document that does not parse never queries at all); anywhere else, no request. Every request is same-origin, `AbortController`-linked to CodeMirror's own completion `context`, and degrades to no popup (`console.debug`) on any failure. A key item's `apply` re-resolves the same classification fresh against the *live* tree at accept time rather than trusting what the source captured — inserting/renaming just the key's own text, and (only when it has no `:` yet) a `": " + skeleton` alongside it, comma-wrapped as the surrounding gap needs (`classifyObjectGap`/`buildKeyInsertion`, both pure and unit-tested); a `function` item inserts `name()` with the cursor between the parens, or after it for a no-argument signature like `first()`; `element`/`constant`/`variable` items take CodeMirror's own default replace. The required-key marker (`data-msg-required` on `#vd-editor-grid`, Fluent `vd-complete-required`) is the only translated string this file owns for completion — every label/detail/message otherwise comes from the server already localized or, for FHIRPath identifiers, untranslatable by nature. Each lint diagnostic's `fixes` (#821) becomes a `Diagnostic.action` that resolves its own RFC 6901 `pointer` against the live tree at apply time and dispatches one `userEvent: "lint.fix"` transaction (`renameKeyChange`/`removeKeyChange`/`setStringChange`, atop the pure, unit-tested `removeKeyRange`/`stringContentRange`/`escapeJsonStringContent`), then relaunches the lint; **Ctrl+.** applies the single fix under the cursor or opens the lint panel for several, `lintKeymap` adding F8/Ctrl-Shift-M alongside it. `#vd-editor-form`'s own `submit` listener pops a plural-correct `window.confirm` (`data-msg-save-errors-one`/`-other` on `#vd-editor-grid`) when submitting as Save while the most recently completed lint pass still has an error |
| `sql-editor.js` | The SQL pane editor on `/ui/sql/queries` and `/ui/sql/views`: SQLite-dialect SQL language and highlighting; after each `#run-notice` swap, tints the line a parse failure names via `data-error-line` (#839) and, via `@codemirror/lint`'s `setDiagnostics`/`lintGutter()` (#842/04), underlines every unknown table `data-diagnostics` locates (JSON `{from, to, message, table}`, character offsets clamped to the document's current length), each with its own hover tooltip and gutter mark — a missing or unparseable attribute clears whatever the previous notice set. No autocomplete yet (follow-up A of #842) |
| `sql-library-details.js` | The Details JSON editor on `/ui/sql/queries` and `/ui/sql/views` (#840): plain JSON language and the shared highlight preset, no injected grammar or lint; hands the mounted `EditorView` to `editor-pair.js` with `fields: { hidden: "content", legend: "sql-library" }` to drive the guided-form card beside it. Keeps `EditorPair.mount`'s own return value — `{formApi, host}` — instead of discarding it, exposed as `window.HfsSqlLibraryDetails` (#841) so a page script can drive the same pairing programmatically |
| `sql-library-panels.js` | The Parameters (#841), Tables (#842), and Columns (#842/04) cards: on `htmx:afterSwap` of a `#lib-params`/`#lib-tables` that carries `data-document` (the `document` endpoint's own successful `add-parameter`/`add-table`/`remove-table` response), hands that text to `window.HfsSqlLibraryDetails.host.setDoc()` so the mutation lands as one Ctrl+Z-able transaction that also refreshes the guided form and re-fires the live run, then strips the attribute. Falls back to writing the JSON textarea directly and firing `input` when that host is not mounted. Also listens for *Add table*'s own combobox `hfs:combobox-select` (#842) and fills the sibling `table_alias` field with the chosen option's `name` — only while that field is still empty or still holds the *previous* autofill, so a value the person typed themselves is never overwritten — and, event-delegated on `[data-declare-table]` (#842/04), opens the *Add table* `<details>` and fills that same field with an unknown-table row's own name on click, without submitting. The *Add parameter*/*Declare*/*Add table*/*Remove* buttons' own mutation requests are plain `hx-post`/`hx-target`/`formaction` in the template — this file only wires the steps none of those handles alone |

`editor.js` is deliberately thin, and that is the architectural point: it does
not model the resource, know what a choice type is, or understand cardinality.
All of that lives in Rust behind `/ui/editor/render`, where it is tested.

---

## Copy capitalization (#652)

The English catalog (`locales/en/main.ftl`) follows one convention:

- **Title Case** for page titles, section headings, card titles, nav entries,
  tab labels, and action buttons/controls (`Server Dashboard`, `Add Tenant`,
  `Save Changes`). Small words stay lowercase mid-title (`a, an, and, as, at,
  by, for, from, in, of, on, or, the, to, with`); hyphenated compounds
  capitalize both halves (`Per-Action Outcomes`).
- **Sentence case** for anything that reads as a sentence or fragment:
  ledes, help text, hints, subtitles, placeholders, confirmation prompts,
  status values, and error messages — even when the key suffix says `-title`
  or `-heading` (a full-sentence heading stays a sentence).
- FHIR and spec spellings are verbatim, always: `FHIR`, `SQL on FHIR`,
  `ViewDefinition`, `NDJSON`, env-var names.

Spanish and German keep their own capitalization norms (both languages use
sentence case where English uses Title Case); the convention above is for
`en` only.

## Required-field marker (#680)

Required fields get an accent-colored asterisk, matching the marker already
used by the resource editor's add-picker (`.editor-add__name em`). This is
implemented as a single CSS `:has()` rule next to `.field__label` in
`assets/app.css`, not per-field markup: the `required` attribute on the input
is the source of truth, and the rule appends `*` after the field's visible
label whenever it wraps a required input. The one field without a visible
label — the search builder's query URL — is marked on its `query-builder__tag`
chip instead. No template renders `*` for this; the only literal `<em>*</em>`
in the codebase is the editor's own (unrelated) marker. Changing the
convention later (e.g. an `(optional)` suffix instead) means editing only
that one CSS rule.

## Error wording (#677)

Errors follow one convention across the Fluent catalogs
(`locales/{en,es,de}/main.ftl`) and the OperationOutcome diagnostics that
`crates/rest/src/error.rs` renders into them:

- Full-sentence errors end with a terminal period; fragments, labels, and
  status values (`Failed`, `failed`, `unavailable`) take none.
- Operation failures use one shape: a full sentence naming the object, with
  the cause after an em dash — `Could not add the tenant — that ID is
  already in use.`
- Interpolated values are single-quoted: `Content type 'text/csv' is not
  supported.`
- Duality: this convention governs what the user sees — the message catalogs
  and the `diagnostics` field of returned `OperationOutcome`s, which reach
  the browser verbatim. It does not apply to `RestError`'s `Display` impl in
  `crates/rest/src/error.rs`, which stays in its own `Label: value` shape —
  that form is for logs and traces, not the UI, and is intentionally
  distinct.

## Rules of the road — where things go

- `crates/ui/src/` — Axum handlers/routers returning `impl IntoResponse`
  (HTML). **Thin:** parse request → call into the workspace → render a template.
  Modules: `lib.rs` (router, dashboard, prefs, search), `editor.rs`,
  `search_params.rs`, `compartments.rs`, `conformance.rs`, `history.rs`,
  `tenants.rs`, `json_view.rs`, `i18n.rs`.
- `crates/ui/templates/` — `.html` templates:
  - `layouts/` — shared document shells (`base.html`).
  - `pages/` — full documents (extend a layout).
  - `partials/` — HTMX-swappable fragments (no `<html>` wrapper).
  - `icons/` — Figma-exported SVGs, inlined with `{% include %}`.
- `crates/ui/assets/` — vendored, pinned `htmx.min.js`, CSS, JS, fonts, images.
  Embedded; never fetched at runtime.
- `locales/<locale>/main.ftl` — **at the workspace root**, not in this crate.
- Handlers branch on the **`HX-Request`** header to return a fragment vs. a full
  page (progressive enhancement).

## Rules of the road — where things must NOT go

- **No HTML in Rust string literals or `format!`.** All markup lives in
  templates.
- **No business/FHIR logic in templates.** Templates render data; they don't
  compute it. Reuse existing crates for data access — don't re-implement
  persistence or terminology logic here.
- **No new browser-facing JSON API** to feed the UI. htmx consumes HTML
  fragments, not JSON.
- **No SPA framework, no bundler, no npm dependency** for browser code, with
  one documented exception — see "Assets: vendored & embedded" above. (The
  `e2e/` directory also has a `package.json`, but that is test-only and never
  ships.)
- **No inline `<script>` blobs or scattered JS.** Prefer `hx-*` attributes
  (Locality of Behaviour); where JS is truly needed, use small pinned assets.
  Inert `type="application/json"` data carriers are the one allowed exception,
  and `no-cdn.spec.ts` enforces the rest.
- **No off-origin requests** — no CDN, no remote font, no remote image.
- **No user-visible prose in templates** — templates hold Fluent catalog keys.
- **`helios-rest`'s FHIR REST handlers stay UI-agnostic** — the UI depends on
  them, not the reverse.
- **Don't couple templates to a single FHIR version** — go through the
  version-agnostic abstractions already in the workspace.
- **No new component class for a primitive that already exists, and no
  page-local CSS for a shared control.** The vocabulary below is the whole
  list; `.button` next to `.btn` is how the Import page drifted off the design
  (#543). `design-system.spec.ts` fails a page that uses an undefined class,
  restyles a shared primitive, or defines a selector twice.

---

## Component vocabulary

`assets/app.css` is one stylesheet in four cascade layers —
`@layer tokens, base, components, pages` — so a page rule outranks a component
rule by layer, never by accident of specificity or source order. `tokens` holds
the custom properties, `base` element defaults, `components` the shared
vocabulary, `pages` what is genuinely unique to one screen.

These are the shared primitives. Before styling anything, reach for one; add to
`pages` only what no other screen will ever want.

| Class | What it is |
|---|---|
| `.btn`, `.btn--primary`, `.btn--danger`, `.btn--current`, `.btn--icon` | The action button: 30px high, 12px horizontal padding, 12px type, and a 9px radius. Primary, danger, and current change emphasis only; `--icon` makes the control a 30px square with no horizontal padding. |
| `.card`, `.card-head`, `.table-card` | Raised surface; its header row; the padding variant that hosts a table. |
| `details.card > summary.card-head` | The same card header, native-disclosure flavor (SQL Export's "Advanced", #836): a `<summary class="card-head">` opens/closes its `<details class="card">` with no marker and a pointer cursor, working without JavaScript. |
| `.panel` | Padding for a full-width card that hosts detail fields without rail behavior. |
| `.detail__field`, `.detail__field--wide` | One labelled value. The field owns the 5px label/value gap; `--wide` spans all columns when the field is inside a key/value grid. |
| `.detail-stack` | Padding-free vertical composition for detail fields and form actions, with a 12px gap. Direct `.form-actions` children rely on that gap instead of adding their usual top margin. |
| `.kv-grid`, `.kv-grid--flush` | Responsive two-column key/value layout with 14px row and 18px column gaps; it collapses to one column at 1250px. `--flush` removes the grid's trailing margin when its container already provides the bottom inset. |
| `.page-head`, `.page-head__title`, `.page-head__lede`, `.page-head--row` | Page heading block; the only `<h1>` treatment; `--row` puts an action on the right. |
| `.back-link` | In-page return link with theme-safe normal, visited, hover, and focus states. |
| `.table-wrap` > `.data-table`, `.col-num`, `.col-actions`, `.data-table__empty`, `.table-foot` | The table, always in its scroll wrapper: ordinary headers and data align left; `.col-num` uses tabular figures without changing alignment; `.col-actions` aligns right; empty-state rows stay centered; the footer hosts pagination. |
| `.empty-state` | The same centered, muted empty treatment for non-table content. |
| `.field`, `.field__label`, `.field__input`, `.field__hint`, `.field__hint--error` | A labelled form field. |
| `.row--params`, `.param-grid` | A parameterized SQL Query's values row (#837, SQL Export builder): `.row--params` shades the `<tr>` right under a subject with declared `Library.parameter[use=in]` entries; `.param-grid` lays its fields out three per row in the form's own plain `.field` voice (never `.field-row`, which uppercases labels). Rendered by `partials/sql_parameter_fields.html`'s `fields(prefix, params)` macro — shared, unchanged, with the SQL Query page's own parameter form (#841). |
| `.row-toggle`, `.param-summary` | The values row's own expand/collapse chevron and folded chip strip (#837, SQL Export builder), both in the parameterized subject's Subject cell: `.row-toggle` (24px, `.icon` rotates off `[aria-expanded="false"]`, the same convention `.json-line__arrow` uses) toggles the row; `.param-summary` (inline-flex, 6px gap) holds `sql-export-form.js`'s own `.tag--param`/`.tag--danger` chips while folded. Both server-rendered `hidden`, revealed only for a checked query. |
| `.lib-params-hint`, `.lib-params-add__fields` | The Parameters card's own extras (#841, `partials/sql_parameters_card.html`): `.lib-params-hint` is one undeclared-placeholder hint row (text + its own *Declare* button), a quieter informational surface than `.notice--warn` since it is expected while a query is still being written, not an error; `.lib-params-add__fields` lays the *Add parameter* panel's name/type fields side by side (`.editor-add`/`.editor-add__toggle`/`.editor-add__panel` are the shared "+ Add" disclosure chrome, reused unchanged from the guided-form editor). |
| `.combobox`, `.combobox__*` | Shared progressively enhanced multi-select. Render it through `partials/combobox.html`; callers provide localized domain copy and an HTML-fragment endpoint, while `combobox.js` owns selection/keyboard state and repeated hidden inputs. Keep a named textarea fallback usable without JavaScript. |
| `.addbox`, `.addbox--modal`, `.addbox__panel`, `.addbox__head`, `.addbox__x`, `.addbox__actions` | The `<details>` disclosure for create/add flows; `--modal` centers it as a dialog. |
| `.choice-grid`, `.choice-card`, `.choice-card__title`, `.choice-card__hint` | The radio-group treatment: one selectable card per choice, `:has(:checked)` accent (#735). |
| `.progress`, `.progress__bar`, `.progress--complete`, `.progress--failed`, `.progress--cancelled` | Full-width job progress track; terminal states recolor the fill. |
| `.job-card`, `.job-card__head`, `.job-card__name`, `.job-card__actions`, `.job-card__meta`, `.job-card__files` | One async job: name + action row, progress track, one meta line, download pills. |
| `.form-legend`, `.field-row` | Standalone section heading between cards; uppercase-labelled fields side by side. |
| `.menu`, `.menu__panel`, `.menu__heading`, `.menu__option` | The `<details>` dropdown (tenant/version selectors, Recent). |
| `.notice`, `.notice--warn` | Inline banner. |
| `.pill` | Large control chip (chart tools). |
| `.tag`, `.tag--*` | Small status pill in tables and lists. |
| `.chip` | Facet/filter chip row member. |
| `.toolbar`, `.toolbar__title`, `.toolbar__search`, `.toolbar__count` | In-card section header with optional search. |
| `.tabs`, `.tab`, `.tab--on` | Tab strip. |
| `.filter-rail`, `.nav-panel` | Left rails: the filter list inside a page; the type panel flush against the sidebar. |
| `.icon-button`, `.icon-button--danger` | Bare 30px-square icon action (table rows), with the action-button 9px radius. |
| `.busy-status` > `.spinner` | Inline working state: the ring plus a short label, `role="status"` in the markup so it announces. |

Labeled values follow one spacing contract: `.detail__field` owns only its 5px
internal label/value gap, while its direct parent owns the larger external
rhythm. Use `.detail-stack`, `.kv-grid`, `.detail`, or `.tester` as that parent;
do not add outer margins to individual fields or place them directly in a
generic `.card__body`.

```text
Parent owns external rhythm (> 5px)

  Wide metadata (> 1250px)                 Compact metadata (<= 1250px)
  ┌──────────────────────────────────┐      ┌─────────────────────────┐
  │ DESCRIPTION — full width         │      │ DESCRIPTION — full width│
  │   ↕ 5px  value                   │      │   ↕ 5px  value          │
  │              ↕ 14px              │      │          ↕ 14px         │
  │ BASE URL — full width            │      │ BASE URL — full width   │
  │   ↕ 5px  value                   │      │   ↕ 5px  value          │
  │              ↕ 14px              │      │          ↕ 14px         │
  │ FHIR VERSION       ← 18px → STATUS│      │ FHIR VERSION            │
  │   ↕ 5px value          ↕ 5px value│      │   ↕ 5px  value          │
  │              ↕ 14px              │      │          ↕ 14px         │
  │ KIND               ← 18px → DATE │      │ STATUS                  │
  │   ↕ 5px value          ↕ 5px value│      │   ↕ 5px  value          │
  │              ↕ 14px              │      │          ↕ 14px         │
  │ FORMATS             │              │      │ KIND                    │
  │   ↕ 5px value       │              │      │   ↕ 5px  value          │
  └──────────────────────────────────┘      │          ↕ 14px         │
                                            │ DATE                    │
                                            │   ↕ 5px  value          │
                                            │          ↕ 14px         │
                                            │ FORMATS                 │
                                            │   ↕ 5px  value          │
                                            └─────────────────────────┘

  `.detail-stack` uses the same contract vertically: 12px between fields,
  while each field keeps exactly 5px between its label and value.
```

Starting a new page: copy `templates/pages/_scaffold.html` (or crib
`tenants.html`, the smallest real page). Both compose only this vocabulary.

Button emphasis never changes geometry: pair `.btn` with `--primary`,
`--danger`, or `--current` for color and state, and add `--icon` only for a
square icon-only action. Inputs keep their own field scale and do not dictate
button height. The sole fixed-height exception is the open
`.addbox--modal > summary.btn`: while its native `<details>` is open, that
summary becomes the full-viewport backdrop (`height: auto`, zero padding and
radius); its closed state and the actions inside the dialog use the canonical
button scale.

### Busy states

One convention for "this control is doing something" (#679), in two lanes:

- **Fetch-driven scripts** call `window.hfsBusy` (`assets/busy.js`).
  `during(buttons, work)` disables the controls, stamps `aria-busy="true"`,
  and clears when the promise `work()` returns settles — pass a *function*,
  never a promise, so the re-entrancy guard runs before the request exists;
  a `work()` that navigates away returns a promise that never settles.
  `region(el, label)` reveals a pre-rendered `.busy-status` element and
  labels its `[data-busy-label]`; `opts.region`/`opts.label` on `during` tie
  one to the same lifetime. The CSS ring keys off `aria-busy`, so the
  visuals cannot ship without the semantics; reduced motion gets the same
  ring as a static glyph. The `::after` ring must keep `content: ""` — CSS
  generated *text* would join the accessible name.
- **htmx controls** use `hx-disabled-elt` (#581); `hx-indicator` is
  deliberately absent (the tenants tests pin this). A pending state that
  outlives the request belongs in the swapped fragment, like the tenants
  provisioning row.

---

## Design source

The visual design is Brett's Figma file
[`CcLtq79cH2aHv4Ii9aNQTP`](https://www.figma.com/design/CcLtq79cH2aHv4Ii9aNQTP/Untitled?node-id=34-2)
— frames "Dashboard V1.1" (34:2, light), "… - Dark" (34:484), and
"… - Tenant Selector" (34:222). Colors, type scale, radii, and shadows in
`assets/app.css` are the inspected values from that file; change them there
first, then here.

Exported from Figma via the REST API (needs a `FIGMA_TOKEN` with file-content
read access):

- `templates/icons/*.svg` — icon nodes exported as SVG, fills normalized to
  `currentColor` so CSS theming applies; inlined with `{% include %}`.
- `assets/logo.png` — the brand mark, exported at 3×.
- `assets/fonts/figtree-*.woff2` — vendored [Figtree](https://fonts.google.com/specimen/Figtree)
  variable font (OFL), embedded like every other asset.

Light/dark theming is CSS custom properties on `:root` / `[data-theme="dark"]`.
Both themes are held to WCAG 2.2 AA, `color-contrast` included, by the axe gate
in `e2e/tests/a11y.spec.ts`.

## Fragment / partial conventions & progressive enhancement

- A **page** (`pages/`) extends `layouts/base.html` and returns a full document.
- A **partial** (`partials/`) returns just the fragment to be swapped — no
  `<html>`/`<head>` wrapper — and is `{% include %}`d into pages so the initial
  full-page render and the htmx swap render identical markup.
- Handlers that back an htmx swap must also work as a **hard navigation**: when
  the `HX-Request` header is absent, return the full page. `/ui/status` is the
  minimal reference implementation of the pattern. Every control needs a real
  `<a href>` / `<form>` underneath so it works with JavaScript disabled — the
  dashboard's window and series selectors are plain links for exactly this
  reason.
- `AutoVaryLayer` emits `Vary: HX-Request` on handlers that read the header, so
  a cache never serves a fragment for a hard navigation. Being on this router
  gets you this for free; don't hand-roll the header.
- **`needs-js`** (`assets/app.css`'s `.needs-js` utility, `@layer components`):
  for a fragment rendered inline, server-side, on a page's own first paint
  that needs a client-side loop wired to it before it should show — the
  alternative to fetching it after `load`, which costs a visible layout
  shift. Hidden until `<html class="js">`, a class `theme.js` sets
  synchronously, before first paint, so a page whose script never runs (or
  hasn't run yet) never shows a dead card. See "The one exception: a
  vendored, prebuilt bundle" above (View Definitions' guided-form card) for
  the concrete case this backs.

Relevant htmx request/response headers we rely on: `HX-Request` (present on
htmx-issued requests). See the
[htmx patterns](https://htmx.org/examples/) for active-search, click-to-edit,
inline-validation, and infinite-scroll fragment recipes we'll standardize on.

---

## Pages & routes

Mounted under `/ui` when running `hfs` (the `ui` feature is on by default; the
`headless` feature disables it — the mount is
`cfg(all(feature = "ui", not(feature = "headless")))`, so enabling both yields
no UI).

```bash
cargo run -p helios-hfs   # then open http://127.0.0.1:8080/ui
```

| Route | Method | Page |
|---|---|---|
| `/ui` | GET | Dashboard — stat cards and the "FHIR resources over time" chart |
| `/ui/resources` | GET | Resources workspace: type rail with live counts, search, edit modal |
| `/ui/editor` | GET | Standalone schema-driven resource editor |
| `/ui/editor/render` | POST | Applies every structural mutation and re-renders; the document rides with the request. `pane=form` (#843) renders only the guided-form panel — hidden state plus the card, no JSON view — for a host that keeps its own JSON editor (the View Definitions and SQL Query/SQL View Details pages' CodeMirror panes). `hidden` (#840) is a comma-separated list of first-level element names the host neither shows nor lets this endpoint mutate (`content`, for a Library edited as SQL Query/SQL View Details, whose SQL attachment lives in its own card); `legend` (#840) overrides which of the guided-form card's explanatory legends renders (`resource`/`view-definition`/`sql-library`) independently of the document's own `resourceType` — see `editor::Legend` |
| `/ui/json-view/render` | POST | Renders raw `application/json` as a highlighted, foldable HTML fragment; applies no FHIR semantics and retains no payload |
| `/ui/editor/expand` | GET | ValueSet expansion, proxied to `HFS_TERMINOLOGY_SERVER` |
| `/ui/queries` | GET | Saved FHIR queries per resource type (#234) and the visual search builder |
| `/ui/queries/params` | GET | Per-type search-parameter catalog backing the builder's datalist |
| `/ui/search` | GET | Natural-language search — **registered only when NL search is enabled** |
| `/ui/search-parameters` | GET | SearchParameter viewer (#238): rail, facets, paginated table, detail panel, plus the write half |
| `/ui/compartments` | GET | Compartment viewer & route tester (#237): "is this type in this compartment, via which parameters, and what search does the server run?" |
| `/ui/history` | GET | Version rail with from/to selection |
| `/ui/history/diff` | POST | Server-side field-level and word-level diff (`docs/history-diff-rendering.md`) |
| `/ui/batch` | GET | Batch/Transaction workspace (#476) |
| `/ui/tenants` | GET/POST | Tenant maintenance; `/ui/tenants/rows` (GET), `/ui/tenants/{id}` (DELETE) |
| `/ui/status` | GET | System-status read path; the fragment-vs-full-page reference |
| `/ui/version` | POST | Persists the sidebar FHIR-version choice (#343) and redirects back |
| `/ui/tenant`, `/ui/tenant/options` | POST/GET | Tenant selector (#344), options loaded lazily |
| `/ui/sql/queries`, `/ui/sql/views` | GET/POST | SQL Query / SQL View workspaces (#649, #838–#842): rail, Details (JSON editor + guided form), the SQL card, the live `$sql-run` preview, — SQL Query only — the Parameters card, and the Tables panel (*Reads from*/*Used by*/*Columns*). POST is Save/Duplicate. See "SQL on FHIR playgrounds", "Details", "Parameters card", and "Tables panel" above |
| `/ui/sql/queries/run`, `/ui/sql/views/run` | POST | The pages' own live-preview fragment (#839, extended in #841 with the Parameters card's OOB companion and the "waiting"/SQL-View-parameter gates, in #842 with the Tables card's own OOB companion when the posted `tables_sig` no longer matches, and in #842/04 with the unknown-table lint gate — no `$sql-run` call, `data-diagnostics` — and the Columns card's own OOB update on every response) — see "SQL on FHIR playgrounds" above |
| `/ui/sql/queries/document`, `/ui/sql/views/document` | POST | The Parameters card's *Add parameter*/*Declare* mutations (#841) and the Tables card's *Add table*/*Remove* mutations (#842) — see "Parameters card" and "Tables panel" above |
| `/ui/sql/export` | GET/POST | Active SQL Exports — the user's `$sql-export` jobs as cards, most recent first (#833); POST resolves the checked subjects and kicks off the job, optionally naming it |
| `/ui/sql/export/new` | GET/POST | SQL Export builder (#834, #836, #837) — an optional name, a single filterable table of stored ViewDefinitions/Libraries with their status, an optional "Narrow it down" card (patients/groups/since) and "Advanced" disclosure (tracking id, CSV header), and an output format; `?subject=` (repeatable) pre-checks matching rows. Every SQL Query declaring `Library.parameter[use=in]` entries carries an `n parameter(s)` chip and, right under its own row, a values row (`param:{reference}:{name}` fields, one per declared parameter, `partials/sql_parameter_fields.html`) — always rendered, unconditionally visible without JavaScript. `sql-export-form.js` hides an unchecked query's values row, reveals its row-toggle chevron once checked, folds it into a `:name = value`/`:name — required` chip summary, keeps the fields' native `required` in sync with "checked and no default", appends "· k value(s) missing" to the selection count, and blocks a submit with an empty required field — opening the affected row and focusing the field. A resubmission that left a field in error re-renders that row with `data-open`, the script's cue to keep it expanded and focus that field. `POST /ui/sql/export` is the same builder's submit target — validates every checked SQL Query's parameter values by declared type before kick-off — see the row above |
| `/ui/lookup/patient-options`, `/ui/lookup/group-options` | POST | Shared Patient/Group combobox search fragments (#836), htmx-only; `?target=` selects which field's `#{target}-message` slot the response's `hx-swap-oob` lands in — one of `bulk-export-patients` \| `sql-export-patients` \| `sql-export-groups`, a closed list (an unrecognized value is a bare `400`). Group search adds `Group.name` only for R5+ (`data/search-parameters-{r4,r5}.json`); Patient search shares Bulk Export's own runtime id-only downgrade |
| `/ui/lookup/table-options` | POST | *Add table*'s own combobox search fragment (#842), htmx-only; `?target=lib-tables` (the only accepted value, else `400`) and an optional `?exclude=Library/{id}`. Up to 8 results: ViewDefinitions whose `name` contains the posted `q` (`search_page` with `name:contains`, `_sort=name`) and `sql-view` Libraries whose `name` contains it (from the already-fetched rail list — `sql-query` Libraries never appear), minus `exclude`; empty `q` returns the first 8 by name. Each option's `value` is `ViewDefinition/{id}` or `Library/{id}`, its label carries the " — ViewDefinition"/" — SQL View" suffix, and it carries an extra `data-name` — the artifact's bare name, for *Add table*'s own alias autofill — that the Patient/Group options above never set |
| `/ui/sql/export/{id}` | GET | A job's own permalink (#835): header with the contextual action/status/overflow, a failure notice when `failed`, the Job card, and the Output files table — reading the notebook's own persisted record, not the server. The Job card surfaces `filters` (#836) when the job carries them: Tracking id, Since, and one `.tag` chip per Patients/Groups reference, each field skipped entirely when empty; Format gains " · with header row"/" · no header row" for a `csv` job with a known `header` choice. Its Subjects field also carries a `:name = value` chip per parameter (#837) right after any SQL Query subject that supplied one, in submission order — empty for every other subject |
| `/ui/sql/export/{id}/detail` | GET | htmx fragment of the page above (`#job-detail`), polled every 5s while the job is `in-progress`; `404` with no body for an id this user/tenant does not own |
| `/ui/sql/export/{id}/card` | GET | htmx fragment: one job's card, polling `$sql-export` status while the job is in progress |
| `/ui/sql/export/{id}/cancel`, `/retry`, `/rerun`, `/remove` | POST | Per-job actions: cancel an in-progress job, resubmit a failed or terminal job as a new record, or drop a terminal job's record from the list |
| `/ui/sql/files` | GET | `301` → `/ui/sql/export` (the job-id form was replaced by the job page, #835) |
| `/ui/assets/*` | GET | Embedded htmx, CSS, JS, fonts, logo |

The router's `fallback_service` is the FHIR app, so anything not under `/ui`
falls through to the normal REST surface.

### Where the data comes from

- **Conformance resources are not vendored into this crate.** SearchParameter and
  CompartmentDefinition are fetched from the server's *own* FHIR API over HTTP on
  the loopback address — storage is the source of truth, seeded per provisioned
  tenant. `ConformanceSource` is the seam; `StaticConformanceSource` stands in
  for it in tests.
- **Dashboard counts and the chart** come from the storage backend through
  `helios_observability::dashboard`, a provider the server registers at startup.
  That keeps this crate free of any persistence dependency for the read path;
  with no provider registered, the dashboard renders placeholder figures through
  the same rendering path. Counts reflect the **default tenant** only — an
  operator view, never exported to the public Prometheus `/metrics` endpoint.
- **Per-user preferences** (theme, nav state, FHIR version, tenant, saved and
  recent queries, and — since #754/#755 — every sidebar rail's `rails.<page>`
  record of `last`/`recent`, tenant-scoped, see `rail_state`) roam in the
  `/_user/settings` document: weak `ETag`, JSON merge patch, `If-Match` on
  write. Tenant-derived keys live under a reserved `byTenant` map so a tenant
  purge can reach them. The server renders the "Recently used" group from
  `recent` on every rail but Compartments — its 4-5 definitions make a group
  noise, so it remembers only `last`; with no settings store configured, there
  is nothing to render and every rail opens on its page default instead. The
  async export workspaces' job lists live there too —
  `byTenant.<tenant>.bulkExport.jobs` and `byTenant.<tenant>.sqlExport.jobs` —
  one member per job, keyed by a locally-generated id, written with the same
  optimistic-locking (`If-Match`) read-modify-write every other settings write
  uses. A SQL Export job optionally carries the builder's trimmed `name`
  (#834); empty is omitted, and the card falls back to the subjects' own
  names. `name` is a label for this notebook only — it is never sent to
  `$sql-export` itself. A SQL Export job's `filters` (#836) records the
  job-wide `patients`/`groups`/`since`/`header`/`clientTrackingId` the
  "Narrow it down" card and "Advanced" disclosure submitted — resolved and
  canonicalized the same way the request itself was built, so *Retry*/*Run
  again* reproduce it exactly; serialization is all-or-nothing (every field
  present once any one of them is set) and the whole object collapses to
  `null` only when every field is at its empty default, so a round trip
  through a resubmission is never lossy. Records persisted before `filters`
  existed still deserialize, with every field defaulting to empty. Each
  `subjects[]` entry also carries its own `parameters` (#837) — the
  name/type/value of every value submitted for that SQL Query's declared
  `Library.parameter[use=in]` entries, empty for a ViewDefinition, a SQL
  View, or an unparameterized SQL Query. `type` is the declared FHIR type
  code, not a hint: it is what `kickoff` types the value as when it rebuilds
  `subject.parameters` on *Retry*/*Run again*.
- **SQL Export's self-calls carry the caller's own identity.** `$sql-export`
  kick-off, status polling, cancel, and the completion manifest all go through
  `ConformanceSource`'s four `$sql-export` methods with a `Caller`: the
  browser's own `Authorization` bearer when the request carried one, the
  configured outbound credential (`HFS_OUTBOUND_BEARER_TOKEN`) otherwise — so
  an async export is attributable to the person who started it, not a service
  account. Because the Active SQL Exports list is the browser's own notebook
  rather than server state (the one `$sql-export` job controller is in-memory,
  ownerless, and reaped after 24h), **jobs started through the API — outside
  this UI — never appear on the list**, and a job the server has since reaped
  or restarted away from shows as `cancelled` with an explanatory reason
  rather than as an error.
- **A job's detail page (`/ui/sql/export/{id}`, #835) reads the notebook's own
  `outputs`, never the server.** The completion manifest is copied into the
  record the moment a poll sees the job go `Done` (module docs of
  `sql_export.rs`), so the detail page's Output files table survives the
  reaper and a server restart the same way the list does — the one exception
  being the download links themselves, which expire with whatever the storage
  backend's presigned-URL TTL is. Only a job still `in-progress` triggers a
  poll: one when the page (or its htmx fragment) renders, exactly like the
  list. An id belonging to another user or tenant is a `404` rendered by the
  shared `pages/not-found.html`/`render_not_found` helper (`crates/ui/src/
  lib.rs`), indistinguishable from an unknown id — reusable by any future
  route that needs the same "not found, and not why" shape.

### Internationalization

All user-visible text resolves from Fluent catalogs at `locales/<locale>/main.ftl`
(workspace root, embedded at compile time). Negotiation order: `?lang=` → the
`hfs_lang` cookie → `Accept-Language` (RFC 4647 Lookup) → `en`. Supported today:
`en`, `es`, `de`. A key missing from a translation falls back to its English
string, and `catalogs_share_the_same_key_set` fails the build-time test suite if
the catalogs drift apart. See `docs/multi-language.md`.

---

## Tests

Two rings, both required.

**Inner ring — Rust, fast.** `tower::oneshot` against the mounted router,
locale middleware included:

```bash
cargo test -p helios-ui
```

`tests/router_http.rs`, `tests/i18n_http.rs`, `tests/tenants_http.rs` (a real
SQLite store, via the test-only `sqlite` feature), plus `mod tests` in most
`src/` modules.

**Outer ring — Playwright + axe-core.** Behavior only a real browser can
observe. Everything Node lives in `e2e/`; the cargo workspace is untouched. See
[`e2e/README.md`](e2e/README.md) for the full spec inventory — among them,
the ViewDefinition editor's own two specs (#821): `vd-editor-lint.spec.ts`
(gutter marker, hover tooltip, quick fixes by click and by Ctrl+., the lint
panel, Ctrl+Z undo, the save-with-errors confirmation, `?lang=es`) and
`vd-editor-completion.spec.ts` (structural keys, FHIRPath elements, the
`forEach` context, constants, functions, and no popup/request outside a
completion context).

```bash
cargo build -p helios-hfs --features ui
cd e2e && npm ci && npx playwright install chromium
npx playwright test
```

The axe gate is strict: every WCAG 2.2 AA rule, `color-contrast` included, in
both themes, is a hard failure. CI runs `ui-tests.yml` per PR on SQLite, and
`ui-tests-matrix.yml` across every storage backend nightly and on demand.

---

## Known follow-ups

- **Outbound service token.** When auth is enabled, the conformance self-call
  relies on an operator provisioning a non-expiring bearer via
  `HFS_OUTBOUND_BEARER_TOKEN`; without one the fetch is rejected and the pages
  degrade to a warning (`pages/compartments-degraded.html`). The fix is a
  short-lived, auto-refreshed token minted through the planned
  `JwtAssertionOutboundAuthProvider` (SMART Backend Services
  `client_credentials` + `private_key_jwt`, `crates/auth/src/outbound.rs`).
- **Asset base path.** `/ui/assets/*` is hardcoded in `layouts/base.html`; it
  would need resolving generically if the mount point ever moves off `/ui`.
- **Richer status read paths** — CodeSystem/ValueSet lookup alongside the
  existing resource counts.

---

## References

- htmx documentation — <https://htmx.org/docs/>
- *Hypermedia Systems* — <https://hypermedia.systems/>
- Locality of Behaviour — <https://htmx.org/essays/locality-of-behaviour/>
- HATEOAS — <https://htmx.org/essays/hateoas/>
- htmx patterns/examples — <https://htmx.org/examples/>
- Axum — <https://docs.rs/axum> · tower-http — <https://docs.rs/tower-http>
- Askama — <https://docs.rs/askama> · Maud — <https://maud.lambda.xyz/> ·
  Minijinja — <https://docs.rs/minijinja>
- rust-embed — <https://docs.rs/rust-embed> · axum-htmx — <https://docs.rs/axum-htmx>
- Fluent — <https://projectfluent.org/> · axe-core — <https://github.com/dequelabs/axe-core>
- OWASP XSS Prevention Cheat Sheet —
  <https://cheatsheetseries.owasp.org/cheatsheets/Cross_Site_Scripting_Prevention_Cheat_Sheet.html>
