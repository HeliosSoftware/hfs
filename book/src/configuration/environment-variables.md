# Environment Variables

All server behavior is controlled through environment variables. No configuration files are required.

## Server

| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_SERVER_PORT` | `8080` | Server port |
| `HFS_SERVER_HOST` | `127.0.0.1` | Host to bind |
| `HFS_LOG_LEVEL` | `info` | Log level: `error`, `warn`, `info`, `debug`, `trace` |
| `HFS_BASE_URL` | `http://localhost:8080` | Base URL for Location headers and Bundle links |
| `HFS_DATA_DIR` | `./data` | Path to FHIR data directory (search parameters) |

## Limits

| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_MAX_BODY_SIZE` | `10485760` | Max request body size (bytes) |
| `HFS_REQUEST_TIMEOUT` | `30` | Request timeout (seconds) |
| `HFS_DEFAULT_PAGE_SIZE` | `20` | Default search result page size |
| `HFS_MAX_PAGE_SIZE` | `1000` | Maximum search result page size |

## CORS

| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_ENABLE_CORS` | `true` | Enable CORS |
| `HFS_CORS_ORIGINS` | `*` | Allowed origins |
| `HFS_CORS_METHODS` | `GET,POST,PUT,PATCH,DELETE,OPTIONS` | Allowed methods |
| `HFS_CORS_HEADERS` | `Content-Type,Authorization,Accept,...` | Allowed headers |

## Storage

| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_STORAGE_BACKEND` | `sqlite` | Backend mode (see [Storage Backends](storage-backends.md)) |
| `HFS_DATABASE_URL` | `fhir.db` | Database URL (SQLite path or PostgreSQL connection string) |
| `HFS_ELASTICSEARCH_NODES` | `http://localhost:9200` | Comma-separated Elasticsearch node URLs |
| `HFS_ELASTICSEARCH_INDEX_PREFIX` | `hfs` | Elasticsearch index name prefix |
| `HFS_ELASTICSEARCH_USERNAME` | *(none)* | Elasticsearch basic auth username |
| `HFS_ELASTICSEARCH_PASSWORD` | *(none)* | Elasticsearch basic auth password |
| `HFS_S3_BUCKET` | `hfs` | S3 bucket name |
| `HFS_S3_REGION` | *(AWS chain)* | AWS region override |
| `HFS_S3_PREFIX` | *(none)* | Optional key prefix for all S3 object keys |
| `HFS_S3_VALIDATE_BUCKETS` | `true` | Validate bucket access on startup |

## Multi-Tenancy

| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_DEFAULT_TENANT` | `default` | Default tenant ID |
| `HFS_TENANT_ROUTING_MODE` | `header_only` | Routing mode: `header_only`, `url_path`, `both` |
| `HFS_TENANT_STRICT_VALIDATION` | `false` | Error if URL and header tenant disagree |
| `HFS_JWT_TENANT_CLAIM` | `tenant_id` | JWT claim name for tenant (future use) |

## Behavior

| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_DEFAULT_FHIR_VERSION` | `R4` | Default FHIR version: `R4`, `R4B`, `R5`, `R6` |
| `HFS_ENABLE_REQUEST_ID` | `true` | Enable request ID tracking |
| `HFS_RETURN_GONE` | `true` | Return 410 Gone for deleted resources (vs 404) |
| `HFS_ENABLE_VERSIONING` | `true` | Enable ETag versioning |
| `HFS_REQUIRE_IF_MATCH` | `false` | Require `If-Match` header for updates |
