# Sub-Header output — HTS UI design sync

Two `StrReplace`-ready edits for `edson/docs/hts-ui-design.md`. Both `old_string`
blocks are copied verbatim from the doc (line ranges called out per edit).
Line numbers of Rust source references were confirmed against `main` at the
time of drafting.

## Edit 1: Header status (D1)

- **Rationale**: Doc L8 still claims "research / design — not yet
  implemented", but Phases 1–3 landed on `feat/551-hts-ui` with 80/0 Rust and
  75/0/3 Playwright green (per the plan file's deliverable snapshot). Aligns
  the header with reality without touching the rest of the preamble.
- **File**: `edson/docs/hts-ui-design.md`
- **old_string** (verbatim, single line from L8, preserves the trailing
  two-space markdown line break):

```
**Status:** research / design — not yet implemented.  
```

- **new_string**:

```
**Status:** v1 shipped on `feat/551-hts-ui` (§7.1–§7.7 + §7.9; §7.8 Bootstrap ledger deferred to Phase 8+). Tests green: `cargo test -p helios-hts-ui` 80/0, `pnpm --filter helios-hts-ui-e2e test` 75/0/3 skipped.  
```

## Edit 2: New §7 preamble subsection (D4 D9 M9 M10 M11)

- **Rationale**: The current §7 header covers Fluent key convention, the
  three inherited guards, and mentions `HTS_UI_UPSTREAM_URL` once — but has
  no consolidated view of the `HTS_UI_*` / `HTS_*` env surface that governs
  boot. This subsection fills M9, corrects `HTS_UI_ENABLED` parsing (D9:
  clap bool, not `"1"`/`"0"`), fixes the port default (M10: `8090`), and
  documents `.no_proxy()` as production behavior on loopback (D4 / M11)
  rather than a test-only shim. Sits between the guards block and §7.1 so
  every per-page section inherits it. Numbering `### 7.0` matches the
  numeric style already used at §7.10 / §7.11.
- **File**: `edson/docs/hts-ui-design.md`
- **old_string** (verbatim, L725–730 — the two closing guard bullets; unique
  in the doc):

```
- **nojs** — every control is a real `<a>` or `<form>` first; htmx only
  augments. `hx-swap-oob`, `hx-trigger="every Ns"`, and click-to-load pagers
  degrade to full-page navigation with a `?next=` cursor.
- **Error** — every operation surfaces `partials/hts-outcome.html`
  (`OperationOutcome`) inline in the affected region; never a modal, never
  a full-page swap.
```

- **new_string** (old_string + new subsection appended; a single blank line
  separates the closing guard bullet from the new subsection, matching the
  blank line at L731 that precedes §7.1):

```
- **nojs** — every control is a real `<a>` or `<form>` first; htmx only
  augments. `hx-swap-oob`, `hx-trigger="every Ns"`, and click-to-load pagers
  degrade to full-page navigation with a `?next=` cursor.
- **Error** — every operation surfaces `partials/hts-outcome.html`
  (`OperationOutcome`) inline in the affected region; never a modal, never
  a full-page swap.

### 7.0 Boot & runtime env

The `hts` binary boots with the settings below; each is a clap arg wired to
an `HTS_*` environment variable in `crates/hts/src/config.rs`. The UI only
cares about the subset listed here.

- **`HTS_UI_ENABLED`** — clap bool: `"true"` or `"false"` only; `"1"` / `"0"`
  are **not** accepted. Default `false`; the UI router mounts at `/ui` only
  when set to `true` (`crates/hts/src/config.rs:123-124`).
- **`HTS_SERVER_PORT`** default `8090` (not `8080`) and **`HTS_SERVER_HOST`**
  default `127.0.0.1` — combined they form the bind address the mount site
  synthesizes as the upstream base when no override is set.
- **`HTS_UI_UPSTREAM_URL`** — remote HTS base URL override; see the guards
  above for degraded-state semantics. Unset means "loopback to this binary".
- **`HTS_MAX_EXPANSION_SIZE`** — default `3500`. `too-costly` ceiling on
  `$expand`, applied **only when the request omits `count`**. UI expand flows
  always send an explicit `count`, so a user-visible "too costly" demo path
  requires either lifting the ceiling or emitting a bare `$expand` from the
  workbench (M8, Sub-CS).

Two knobs live in code, not env — a rebuild is the only way to change them:

- **`HTS_UI_MAX_EXPANSION_SIZE_HINT`** = `100_000`
  (`crates/hts-ui/src/upstream.rs:1391`). Threshold above which the UI omits
  the `count` param on `$expand` (§7.4).
- **`HTS_UI_BATCH_FANOUT_CONCURRENCY`** = `8`
  (`crates/hts-ui/src/upstream.rs:2806`). Concurrent fan-out for batch
  `$validate-code`.

**Corporate proxy handling.** `UpstreamClient::new_with_timeouts` calls
`reqwest::ClientBuilder::no_proxy()` when the base URL is loopback
(`crates/hts-ui/src/upstream.rs:78-79`, gated by `is_loopback_base_url` at
L1257). This is production behavior, not a test-only shim: operators with
`HTTP_PROXY` / `HTTPS_PROXY` in the environment still get a working
in-process self-call against `127.0.0.1` / `localhost`.

| Env var | Default | Effect on UI |
|---|---|---|
| `HTS_UI_ENABLED` | `false` | Mount `/ui` when `true` |
| `HTS_SERVER_HOST` / `HTS_SERVER_PORT` | `127.0.0.1` / `8090` | Loopback base URL when no override |
| `HTS_UI_UPSTREAM_URL` | *(unset)* | Point UI at a remote HTS |
| `HTS_MAX_EXPANSION_SIZE` | `3500` | `too-costly` ceiling on bare `$expand` |
```

## Verification aids

- Edit 1 `old_string` first 40 chars: `**Status:** research / design — not yet ` (matches L8 verbatim, including trailing double-space).
- Edit 2 `old_string` first 40 chars: `- **nojs** — every control is a real \`<` (matches L725 verbatim; the bullet pair L725–730 is unique in the doc).

## Structural notes

- Chose `### 7.0` over `### 7 preamble: …` because the doc already uses numeric sub-sections up to §7.11 (`### 7.10 States matrix`, `### 7.11 Utility surfaces covered`), so `### 7.0` is consistent. `### 7 preamble: …` would be the only non-numeric H3 under §7.
- Did **not** restate `HTS_UI_UPSTREAM_URL`'s degraded-state semantics — the guards block above (L716–724) already owns that; the new subsection only cross-refs it.
- The new subsection is 40 lines of markdown counted line-for-line (heading + intro + 4 bullets + 2 constants bullets + proxy paragraph + 4-row table). At the ~40-line ceiling; if it needs to drop, the table is the safest cut (bullets already carry the same info).
- Left the em-dash character (`—`, U+2014) unchanged everywhere — the doc uses it throughout, and `StrReplace` matches on bytes so the em-dash must round-trip verbatim.
