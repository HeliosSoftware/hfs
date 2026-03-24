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

Standard AWS credential chain applies (`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, instance profiles, etc.). For S3-compatible endpoints (e.g., MinIO), configure `endpoint_url` and `force_path_style` directly.

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
