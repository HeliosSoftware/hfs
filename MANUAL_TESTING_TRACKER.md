# HFS Manual Testing Tracker

One GitHub issue per storage configuration per authentication mode. Each issue runs
the T0 through T9 procedure in [`MANUAL_TESTING_MATRIX.md`](MANUAL_TESTING_MATRIX.md)
and records its results in the issue. This file only tracks which issue covers which
cell; update the status column in the issue, not here.

Anonymous means `HFS_AUTH_ENABLED` unset. The auth columns run the same procedure
with `HFS_AUTH_ENABLED=true` against that identity provider, plus the A1 through A5
token checks described in the Keycloak issues. Provider-level verification (claims
inventory, scope-claim shape, all four IdPs) is [#724](https://github.com/HeliosSoftware/hfs/issues/724).

| Storage configuration (`HFS_STORAGE_BACKEND`) | Anonymous | Keycloak | Okta | Auth0 | Entra ID |
|---|---|---|---|---|---|
| `sqlite` | [#936](https://github.com/HeliosSoftware/hfs/issues/936) | [#1170](https://github.com/HeliosSoftware/hfs/issues/1170) | [#1180](https://github.com/HeliosSoftware/hfs/issues/1180) | [#1181](https://github.com/HeliosSoftware/hfs/issues/1181) | [#1182](https://github.com/HeliosSoftware/hfs/issues/1182) |
| `sqlite-es` (SQLite + Elasticsearch) | [#937](https://github.com/HeliosSoftware/hfs/issues/937) | [#1171](https://github.com/HeliosSoftware/hfs/issues/1171) | [#1180](https://github.com/HeliosSoftware/hfs/issues/1180) | [#1181](https://github.com/HeliosSoftware/hfs/issues/1181) | [#1182](https://github.com/HeliosSoftware/hfs/issues/1182) |
| `postgres` | [#938](https://github.com/HeliosSoftware/hfs/issues/938) | [#1172](https://github.com/HeliosSoftware/hfs/issues/1172) | [#1180](https://github.com/HeliosSoftware/hfs/issues/1180) | [#1181](https://github.com/HeliosSoftware/hfs/issues/1181) | [#1182](https://github.com/HeliosSoftware/hfs/issues/1182) |
| `pg-es` (PostgreSQL + Elasticsearch) | [#939](https://github.com/HeliosSoftware/hfs/issues/939) | [#1173](https://github.com/HeliosSoftware/hfs/issues/1173) | [#1180](https://github.com/HeliosSoftware/hfs/issues/1180) | [#1181](https://github.com/HeliosSoftware/hfs/issues/1181) | [#1182](https://github.com/HeliosSoftware/hfs/issues/1182) |
| `mongodb` | [#940](https://github.com/HeliosSoftware/hfs/issues/940) | [#1174](https://github.com/HeliosSoftware/hfs/issues/1174) | [#1180](https://github.com/HeliosSoftware/hfs/issues/1180) | [#1181](https://github.com/HeliosSoftware/hfs/issues/1181) | [#1182](https://github.com/HeliosSoftware/hfs/issues/1182) |
| `mongo-es` (MongoDB + Elasticsearch) | [#941](https://github.com/HeliosSoftware/hfs/issues/941) | [#1175](https://github.com/HeliosSoftware/hfs/issues/1175) | [#1180](https://github.com/HeliosSoftware/hfs/issues/1180) | [#1181](https://github.com/HeliosSoftware/hfs/issues/1181) | [#1182](https://github.com/HeliosSoftware/hfs/issues/1182) |
| `s3` (AWS S3) | [#1168](https://github.com/HeliosSoftware/hfs/issues/1168) | [#1176](https://github.com/HeliosSoftware/hfs/issues/1176) | [#1180](https://github.com/HeliosSoftware/hfs/issues/1180) | [#1181](https://github.com/HeliosSoftware/hfs/issues/1181) | [#1182](https://github.com/HeliosSoftware/hfs/issues/1182) |
| `s3-es` (AWS S3 + Elasticsearch) | [#1169](https://github.com/HeliosSoftware/hfs/issues/1169) | [#1177](https://github.com/HeliosSoftware/hfs/issues/1177) | [#1180](https://github.com/HeliosSoftware/hfs/issues/1180) | [#1181](https://github.com/HeliosSoftware/hfs/issues/1181) | [#1182](https://github.com/HeliosSoftware/hfs/issues/1182) |
| `s3` (MinIO) | [#1166](https://github.com/HeliosSoftware/hfs/issues/1166) | [#1178](https://github.com/HeliosSoftware/hfs/issues/1178) | [#1180](https://github.com/HeliosSoftware/hfs/issues/1180) | [#1181](https://github.com/HeliosSoftware/hfs/issues/1181) | [#1182](https://github.com/HeliosSoftware/hfs/issues/1182) |
| `s3-es` (MinIO + Elasticsearch) | [#1167](https://github.com/HeliosSoftware/hfs/issues/1167) | [#1179](https://github.com/HeliosSoftware/hfs/issues/1179) | [#1180](https://github.com/HeliosSoftware/hfs/issues/1180) | [#1181](https://github.com/HeliosSoftware/hfs/issues/1181) | [#1182](https://github.com/HeliosSoftware/hfs/issues/1182) |

The Okta, Auth0, and Entra ID columns currently point at one umbrella issue per
provider. Once the Keycloak rows are complete, split each umbrella into one issue
per storage configuration and replace the links in that column.
