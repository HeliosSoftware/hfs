# HFS Server

The `hfs` binary is the main Helios FHIR Server. It exposes a FHIR-compliant REST API backed by a configurable [storage backend](../configuration/storage-backends.md).

## Running

```bash
# Default: R4, SQLite (file fhir.db), port 8080
cargo run --bin hfs

# Or from a release binary
./hfs
```

## API Endpoints

| Interaction | Method | URL |
|------------|--------|-----|
| Capabilities | GET | `/metadata` |
| Read | GET | `/[type]/[id]` |
| Version read | GET | `/[type]/[id]/_history/[vid]` |
| Update | PUT | `/[type]/[id]` |
| Patch | PATCH | `/[type]/[id]` |
| Delete | DELETE | `/[type]/[id]` |
| Create | POST | `/[type]` |
| Search | GET / POST | `/[type]?params` or `/[type]/_search` |
| Instance history | GET | `/[type]/[id]/_history` |
| Type history | GET | `/[type]/_history` |
| System history | GET | `/_history` |
| Batch / transaction | POST | `/` |
| Health | GET | `/health` |

## Key Features

- Full CRUD operations
- Search with chained parameters
- Resource history and ETag versioning
- Batch and transaction bundle support
- Multi-tenancy via header or URL path (see [Multi-Tenancy](../configuration/multi-tenancy.md))
- CORS support
- Request ID tracking

## Configuration

The server is configured entirely through [environment variables](../configuration/environment-variables.md). No config files required.

```bash
# Explicit example
HFS_SERVER_PORT=3000 \
HFS_STORAGE_BACKEND=postgres \
HFS_DATABASE_URL="postgresql://user:pass@localhost/fhir" \
HFS_LOG_LEVEL=debug \
  ./hfs
```
