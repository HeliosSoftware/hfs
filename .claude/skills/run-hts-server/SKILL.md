---
name: run-hts-server
description: Run or configure the HTS terminology server binary for local development. Use for hts binary startup, HTS_* runtime env vars, and bootstrapping it with the bundled terminology-data seed set. For import CLI, full API, and operator config, see work-with-hts.
---

# HTS Server

Use this when you need a running `hts` process — the run/config surface
only. For the full `HTS_*` environment reference, API endpoints, bulk
import CLI, and bootstrap-sync internals, see
[work-with-hts](../work-with-hts/SKILL.md).

## Running

```bash
# Default: SQLite, port 8090, no seed data
cargo run --bin hts

# Custom database path and port
HTS_DATABASE_URL=./my-terminology.db HTS_SERVER_PORT=9090 cargo run --bin hts
```

```powershell
# Windows equivalent
$env:HTS_DATABASE_URL = ".\my-terminology.db"
$env:HTS_SERVER_PORT  = "9090"
cargo run --bin hts
```

## Seeding terminology data

An empty HTS has no CodeSystems or ValueSets to look up. The repo ships a
bundled, public-domain/permissively-licensed terminology set at
[crates/hts/terminology-data/](../../../crates/hts/terminology-data) — the
same files baked into every HTS release archive and Docker image. Point
`HTS_BOOTSTRAP_DIR` at it to auto-import them on startup:

```bash
HTS_BOOTSTRAP_DIR=./crates/hts/terminology-data cargo run --bin hts
```

```powershell
$env:HTS_BOOTSTRAP_DIR = ".\crates\hts\terminology-data"
cargo run --bin hts
```

## Admin UI

The HTS administrative UI (`crates/hts-ui`) is mounted on `hts` itself at
`/ui/hts` — it is **on by default**, matching HFS's always-on UI. Operators
who deploy behind an API gateway and don't want an HTML surface listening
at all can opt out with `HTS_UI_ENABLED=false`:

```bash
HTS_BOOTSTRAP_DIR=./crates/hts/terminology-data cargo run --bin hts
```

```powershell
$env:HTS_BOOTSTRAP_DIR = ".\crates\hts\terminology-data"
cargo run --bin hts
```

`http://localhost:8090/ui/hts` serves the dashboard; the bare root `/`
redirects there too.

This gives you ICD-10-CM, ICD-9-CM, NCI Thesaurus, MeSH (via NCI Thesaurus
FLAT), NDC, HL7 THO, HL7 v2 tables, UCUM, and NUCC out of the box. Each
file is hashed (SHA-256) into a `bootstrap_imports` ledger, so it's safe to
leave `HTS_BOOTSTRAP_DIR` set permanently in a dev `.env` — unchanged files
are skipped on every restart, only new or changed files re-import.

**Not included**: SNOMED CT, LOINC, RxNorm, CPT, and MedDRA require a
separate license/registration and must be imported manually via
`hts import <path> --format ...` (see
[work-with-hts](../work-with-hts/SKILL.md#bulk-import-cli)).

The bundled files are machine-managed — don't hand-edit them. Refresh the
pinned versions with
[crates/hts/scripts/download-bundled-terminologies.ps1](../../../crates/hts/scripts/download-bundled-terminologies.ps1)
(or the `.sh` equivalent); see
[crates/hts/terminology-data/README.md](../../../crates/hts/terminology-data/README.md)
and `RELEASING.md` for the full refresh workflow.

## Environment (running)

| Variable | Default | Description |
|---|---|---|
| `HTS_SERVER_PORT` | `8090` | Server port |
| `HTS_SERVER_HOST` | `127.0.0.1` | Host to bind |
| `HTS_DATABASE_URL` | `./data/hts.db` | SQLite file path or PostgreSQL connection URL |
| `HTS_BOOTSTRAP_DIR` | none | Directory of terminology files imported on startup — set to `./crates/hts/terminology-data` for the bundled seed set |
| `HTS_LOG_LEVEL` | `info` | Log level: error, warn, info, debug, trace |

See [work-with-hts](../work-with-hts/SKILL.md) for storage backend
selection, CORS/body-size limits, the full API surface, and the bulk
import CLI.

## Sanity check

```bash
curl http://localhost:8090/health
curl "http://localhost:8090/metadata?mode=terminology"
```

The second call should return a `TerminologyCapabilities` resource once
seeded.
