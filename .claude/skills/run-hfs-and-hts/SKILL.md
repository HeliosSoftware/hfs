---
name: run-hfs-and-hts
description: Start the HFS FHIR server and HTS terminology server together for local development, wired via HFS_TERMINOLOGY_SERVER/FHIRPATH_TERMINOLOGY_SERVER so HFS's terminology-backed features (search :in modifier, FHIRPath memberOf()/subsumes()) work against a locally-seeded HTS. Composes run-hfs-server and run-hts-server.
---

# HFS + HTS

Use this when you need both servers running together — e.g. to exercise
HFS features that delegate to HTS, or to develop `crates/ui` terminology
pages that proxy through HFS. This skill only covers the combined startup;
see [run-hfs-server](../run-hfs-server/SKILL.md) and
[run-hts-server](../run-hts-server/SKILL.md) for each server's full
env-var reference.

## Running both

Start `hts` first, seeded from the bundled terminology data, then start
`hfs` pointed at it:

```bash
# Terminal 1 — HTS, seeded with the bundled terminology set, admin UI on
HTS_BOOTSTRAP_DIR=./crates/hts/terminology-data HTS_UI_ENABLED=true cargo run --bin hts
# binds 127.0.0.1:8090, admin UI at /ui/hts

# Terminal 2 — HFS, wired to use HTS for terminology operations
HFS_TERMINOLOGY_SERVER=http://127.0.0.1:8090 \
FHIRPATH_TERMINOLOGY_SERVER=http://127.0.0.1:8090 \
cargo run --bin hfs
# binds 127.0.0.1:8080, UI at /ui
```

```powershell
# Terminal 1 — HTS
$env:HTS_BOOTSTRAP_DIR = ".\crates\hts\terminology-data"
$env:HTS_UI_ENABLED    = "true"
cargo run --bin hts              # binds 127.0.0.1:8090, admin UI at /ui/hts

# Terminal 2 — HFS
$env:HFS_TERMINOLOGY_SERVER      = "http://127.0.0.1:8090"
$env:FHIRPATH_TERMINOLOGY_SERVER = "http://127.0.0.1:8090"
cargo run --bin hfs               # binds 127.0.0.1:8080, UI at /ui
```

Start HTS first — HFS only reads `HFS_TERMINOLOGY_SERVER` at request time,
not at boot, but there's nothing to look up until HTS has finished its
bootstrap import.

Note there are two distinct `/ui` surfaces once both are running: HFS's own
UI at `http://127.0.0.1:8080/ui`, and — only with `HTS_UI_ENABLED=true` —
HTS's administrative UI at `http://127.0.0.1:8090/ui/hts` (off by default;
see [run-hts-server](../run-hts-server/SKILL.md#admin-ui)).

## Sanity checks

```bash
curl http://localhost:8090/health
curl "http://localhost:8090/metadata?mode=terminology"   # TerminologyCapabilities once seeded
curl http://localhost:8080/health
curl -o /dev/null -w "%{http_code}\n" http://localhost:8090/ui/hts   # 200 only if HTS_UI_ENABLED=true
```

## What this enables

With both wired together:

- FHIR search `:in` modifier on HFS (ValueSet expansion filtering).
- FHIRPath `memberOf()` and `subsumes()` on HFS, delegated through
  `FHIRPATH_TERMINOLOGY_SERVER`.
- `crates/ui` terminology-proxy handlers (e.g. `/ui/editor/expand`), which
  read HTS through `WebState.terminology` rather than calling it directly
  from the browser.

Without `HFS_TERMINOLOGY_SERVER` set, these features degrade gracefully
(no terminology-backed filtering / proxy handlers return empty) rather
than erroring.
