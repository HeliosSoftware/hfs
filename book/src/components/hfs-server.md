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

History reads (instance, type and system `_history`) accept `_count` (capped at `HFS_MAX_PAGE_SIZE`), `_since` (an RFC 3339 instant with a time zone, such as `2024-01-01T00:00:00Z`; only versions created at or after it are returned) and `_format`. `_at` is refused with `400 Bad Request` and an OperationOutcome with issue code `not-supported`, whatever its value and whatever `Prefer: handling` the request sends. HFS cannot select the versions that were current at a point in time, and answering with the unfiltered history would look like a filtered answer. Use `_since` to narrow a history read by time. The other history parameters FHIR defines (`_list`, `_sort`) are not implemented and are currently ignored.

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
HFS_BASE_URL=https://fhir.example.com \
HFS_STORAGE_BACKEND=postgres \
HFS_DATABASE_URL="postgresql://user:pass@localhost/fhir" \
HFS_LOG_LEVEL=debug \
  ./hfs
```

`HFS_BASE_URL` is the public origin HFS writes into FHIR responses. Include a
reverse-proxy path prefix when one exists, for example
`https://fhir.example.com/fhir`. HFS validates and logs the value at startup.
It warns when a loopback base does not match the listener.
