# Multi-Tenancy

The Helios FHIR Server has first-class multi-tenancy support. All storage operations are scoped to a tenant, ensuring complete data isolation at the query level.

## Tenant Routing

Configure how the server identifies the active tenant with `HFS_TENANT_ROUTING_MODE`:

| Mode | Description |
|------|-------------|
| `header_only` (default) | Tenant ID comes from the `X-Tenant-ID` request header |
| `url_path` | Tenant ID is the first path segment: `/{tenant-id}/Patient` |
| `both` | Accept either header or URL path |

## Examples

### Via Header (default)

```bash
curl -H "X-Tenant-ID: clinic-a" http://localhost:8080/Patient
curl -H "X-Tenant-ID: clinic-b" http://localhost:8080/Patient
```

### Via URL Path

```bash
# Requires HFS_TENANT_ROUTING_MODE=url_path or both
curl http://localhost:8080/clinic-a/Patient
curl http://localhost:8080/clinic-b/Patient
```

## Configuration Reference

| Variable | Default | Description |
|----------|---------|-------------|
| `HFS_DEFAULT_TENANT` | `default` | Tenant ID used when none is specified |
| `HFS_TENANT_ROUTING_MODE` | `header_only` | How tenant is resolved from the request |
| `HFS_TENANT_STRICT_VALIDATION` | `false` | Return an error if URL and header tenant disagree |
| `HFS_JWT_TENANT_CLAIM` | `tenant_id` | JWT claim name for tenant (future use) |

## Implementation Notes

All persistence operations accept a `TenantContext` as their first argument. Storage backends enforce tenant boundaries at the query level — there is no application-level filtering after the fact. Data isolation is guaranteed even when backends share the same underlying database or index.
