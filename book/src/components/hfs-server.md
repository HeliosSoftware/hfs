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

History reads (instance, type and system `_history`) accept `_count` (capped at `HFS_MAX_PAGE_SIZE`), `_since` (an RFC 3339 instant with a time zone, such as `2024-01-01T00:00:00Z`; only versions created at or after it are returned) and `_format`, each at most once. FHIR's general parameters `_pretty`, `_summary` and `_elements` are accepted but not applied to history. The other history parameters FHIR defines, `_at`, `_list` and `_sort`, are not implemented. They are refused with `400 Bad Request` and an OperationOutcome with issue code `not-supported`, whatever their value and whatever `Prefer: handling` the request sends, because answering with the unfiltered, default-ordered history would look like a filtered or sorted answer. Use `_since` to narrow a history read by time. Any other parameter is refused with `400` (issue code `invalid`) under `Prefer: handling=strict` and ignored otherwise: unlike a searchset, a history Bundle has nowhere to report an ignored parameter. A `_count` that is not a non-negative integer, a `_since` that is not an instant, or a repeated `_count`, `_since` or `_format` is a `400` with issue code `invalid`. History Bundles carry a `self` link and, when there is another page, a `next` link that carries the effective `_count` (the default when the request sent none, capped at `HFS_MAX_PAGE_SIZE`) plus the request's `_since` and `_format`: follow `next` to read the following page. A `_count=0` page has no `next`. Its `_cursor` is opaque, so use the link as given; a `_cursor` that does not decode, or a repeated one, is a `400` with issue code `invalid`.

The same URLs in a batch or transaction `GET` entry are answered like the direct requests (#2380): a vread entry carries the requested version (or is a `404` / `410` entry), and a history entry carries the `history` Bundle, with the parameters, refusals and links above and the Bundle request's `Prefer: handling`; `_format` in an entry URL is ignored. In a transaction they run after the writes, as search entries do: a refused or malformed parameter fails the whole transaction before anything is written, while a missing or deleted version is that entry's own error.

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
