# Storage Backends

The Helios FHIR Server supports several storage backend configurations. Set `HFS_STORAGE_BACKEND` to choose one.

## Available Backends

| Mode | `HFS_STORAGE_BACKEND` value | Description |
|------|-----------------------------|-------------|
| SQLite (default) | `sqlite` | Zero-config, file or in-memory. Built-in FTS5 full-text search. |
| SQLite + Elasticsearch | `sqlite-elasticsearch` or `sqlite-es` | SQLite for CRUD, Elasticsearch for search |
| PostgreSQL | `postgres` or `pg` or `postgresql` | PostgreSQL-native full-text search (tsvector/tsquery) |
| PostgreSQL + Elasticsearch | `postgres-elasticsearch` or `pg-es` | PostgreSQL for CRUD, Elasticsearch for search |
| S3 | `s3` | Object storage for CRUD, versioning, history, and bulk ops. No search. |
| S3 + Elasticsearch | `s3-elasticsearch` or `s3-es` | S3 for CRUD, Elasticsearch for search |

## Running with Each Backend

### SQLite (default)

```bash
./hfs
# or with explicit file path
HFS_DATABASE_URL=./my-fhir.db ./hfs
```

### SQLite + Elasticsearch

```bash
HFS_STORAGE_BACKEND=sqlite-elasticsearch \
HFS_ELASTICSEARCH_NODES=http://localhost:9200 \
  ./hfs
```

### PostgreSQL

```bash
HFS_STORAGE_BACKEND=postgres \
HFS_DATABASE_URL="postgresql://user:pass@localhost:5432/fhir" \
  ./hfs
```

### PostgreSQL + Elasticsearch

```bash
HFS_STORAGE_BACKEND=postgres-elasticsearch \
HFS_DATABASE_URL="postgresql://user:pass@localhost:5432/fhir" \
HFS_ELASTICSEARCH_NODES=http://localhost:9200 \
  ./hfs
```

### S3

The S3 backend requires building with the `s3` feature flag:

```bash
cargo build -p helios-hfs --features s3
```

```bash
HFS_STORAGE_BACKEND=s3 \
HFS_S3_BUCKET=my-fhir-bucket \
AWS_PROFILE=your-aws-profile \
AWS_REGION=us-east-1 \
  ./hfs
```

Standard AWS credential chain applies (`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, instance profiles, etc.). For S3-compatible endpoints (e.g., MinIO), set `HFS_S3_ENDPOINT` and `HFS_S3_FORCE_PATH_STYLE=true`.

#### Tenancy on S3

By default every tenant lives in the one `HFS_S3_BUCKET` under its own key
prefix (`prefix-per-tenant`), the same shared-storage model as the other
backends. S3 can instead give each tenant a dedicated bucket, with its own
bucket policy, lifecycle rules, encryption key or region:

```bash
HFS_STORAGE_BACKEND=s3 \
HFS_S3_TENANCY=bucket-per-tenant \
HFS_S3_TENANT_BUCKETS="default=hfs-default,acme=hfs-acme,globex=hfs-globex" \
HFS_S3_SYSTEM_BUCKET=hfs-system \
  ./hfs
```

The buckets must already exist (startup checks each one with `HeadBucket`),
every tenant needs its own, and `HFS_DEFAULT_TENANT` must be mapped. The
mapped tenants are the provisioned tenants: they are registered and seeded at
startup, and `POST /admin/tenants` refuses an id that is not in the map (add
it and restart). `HFS_S3_SYSTEM_BUCKET` holds what belongs to no tenant: the
tenant registry, `/_user/settings`, web login sessions, the `$bulk-submit`
worker index and the `__system__` tenant's AuditEvents. It is optional, but
without it those endpoints answer `501`, login sessions stay in process, and a
database audit sink needs its own `HFS_AUDIT_S3_BUCKET`.

#### Keyspace layout on S3
**S3 keyspace layout (`HFS_S3_KEYSPACE_LAYOUT`).** Each tenant has one layout, `v1` or `v2`, recorded in a marker object `history/_keyspace.json` (`{"layout":2}`). The variable only decides the layout of a tenant that has no marker and no data yet; a tenant that already holds objects without a marker is always `v1`, and a stamped marker always wins, so changing the variable never moves existing data. `v2` stores `resources/{type}/{h2}/{id}/…` and `history/type/{type}/{h2}/…` with `h2 = hex(sha256(id))[0..2]` (256 sub-prefixes) and drops the tenant-wide `history/system/` index (system history is served from `history/type/`), so each write does 3 PUTs instead of 4. The layout of a tenant is immutable: there is no migration, and `purge_tenant_data` keeps the marker. To change it, stop every instance and delete the tenant's prefix by hand. `v2` requires every instance that writes the bucket to run a build that reads the marker and to use the same setting; an older binary ignores the marker and would write `v1` keys into a `v2` tenant, leaving them unreachable. Opt-in: the default is `v1`.

#### Read concurrency on S3
**S3 read concurrency (`HFS_S3_SCAN_CONCURRENCY`).** The `GetObject` requests behind SQL-on-FHIR scans and by-id reads (`$sql-run`, live previews, SQL Query, `$sql-export`), type counts, conditional-create and SearchParameter scans, and `_history` draw on one budget per S3 store, 32 by default. Concurrent requests share it instead of each fanning out on its own, so four previews at once no longer mean 4 x 32 GETs; one request alone still uses up to the whole budget, so a single scan behaves as it always has. Lower it on a high-latency link or one that resets connections under load (`failed to collect object body` or `DispatchFailure` errors during several parallel previews), and raise it for a bucket close to the server. It applies to both `s3` and `s3-elasticsearch`. Writes (startup seeding, bulk writes) and FHIR bulk `$export` keep their own fan-out and are not counted against it. The budget is shared with the reads some writes make (the SearchParameter overlay reload after a SearchParameter write, and a conditional create's match scan), so with many concurrent SQL-on-FHIR scans those writes can wait for permits. The value is also each scan's fan-out width, so a large value raises how many object bodies one scan can hold in memory at once. A value that is not a whole number of at least 1 fails startup.

### S3 + Elasticsearch

```bash
HFS_STORAGE_BACKEND=s3-elasticsearch \
HFS_S3_BUCKET=my-fhir-bucket \
HFS_ELASTICSEARCH_NODES=http://localhost:9200 \
AWS_PROFILE=your-aws-profile \
AWS_REGION=us-east-1 \
  ./hfs
```

## Choosing a Backend

| Use case | Recommended backend |
|----------|---------------------|
| Development / local testing | SQLite |
| Small production deployment | SQLite or PostgreSQL |
| Production with robust search | SQLite/PostgreSQL + Elasticsearch |
| Archival / bulk analytics | S3 |
| Large-scale with full FHIR search | S3 + Elasticsearch |
