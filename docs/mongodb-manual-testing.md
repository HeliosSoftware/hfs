# MongoDB Backend Manual Testing

This guide provides a practical checklist to manually validate the new MongoDB backend in HFS, including standalone `mongodb` mode and optional `mongodb-elasticsearch` mode.

## Prerequisites

- Docker running locally
- Rust toolchain installed (`cargo`)
- `curl`
- Optional but helpful: `jq`

## 1) Build HFS for MongoDB

From repo root:

```bash
cargo build -p helios-hfs --features mongodb
```

If you also want to test `mongodb-elasticsearch` mode:

```bash
cargo build -p helios-hfs --features mongodb,elasticsearch
```

## 2) Start MongoDB (standalone)

```bash
docker rm -f hfs-mongo-manual >/dev/null 2>&1 || true
docker run -d --name hfs-mongo-manual -p 27017:27017 mongo:8.0
```

## 3) Start HFS in `mongodb` mode

Use a separate terminal and keep the process running.

```bash
export HFS_STORAGE_BACKEND=mongodb
export HFS_DATABASE_URL="mongodb://localhost:27017"
export HFS_MONGODB_DATABASE="helios_manual"
export HFS_SERVER_HOST="127.0.0.1"
export HFS_SERVER_PORT="8080"

BIN="./target/debug/hfs"
[ -f "./target/debug/hfs.exe" ] && BIN="./target/debug/hfs.exe"

"$BIN"
```

## 4) Health and metadata smoke checks

In another terminal:

```bash
export BASE_URL="http://127.0.0.1:8080"
export TENANT="default"

curl -s "$BASE_URL/health"
curl -s "$BASE_URL/metadata"
```

Expected: health status is OK and CapabilityStatement is returned from `/metadata`.

## 5) CRUD + version/history checks

### Create

```bash
cat > patient-v1.json <<'JSON'
{
  "resourceType": "Patient",
  "id": "mongo-manual-1",
  "identifier": [
    {
      "system": "http://example.org/mrn",
      "value": "MONGO-001"
    }
  ],
  "name": [
    {
      "family": "Manual",
      "given": ["Mongo"]
    }
  ],
  "active": true
}
JSON

curl -i -X PUT "$BASE_URL/Patient/mongo-manual-1" \
  -H "Content-Type: application/fhir+json" \
  -H "X-Tenant-ID: $TENANT" \
  --data-binary @patient-v1.json
```

### Read

```bash
curl -s "$BASE_URL/Patient/mongo-manual-1" -H "X-Tenant-ID: $TENANT"
```

### Update (new version)

```bash
cat > patient-v2.json <<'JSON'
{
  "resourceType": "Patient",
  "id": "mongo-manual-1",
  "identifier": [
    {
      "system": "http://example.org/mrn",
      "value": "MONGO-001"
    }
  ],
  "name": [
    {
      "family": "Manual",
      "given": ["Mongo", "Updated"]
    }
  ],
  "active": false
}
JSON

curl -i -X PUT "$BASE_URL/Patient/mongo-manual-1" \
  -H "Content-Type: application/fhir+json" \
  -H "X-Tenant-ID: $TENANT" \
  --data-binary @patient-v2.json
```

### History + vread

```bash
curl -s "$BASE_URL/Patient/mongo-manual-1/_history" -H "X-Tenant-ID: $TENANT"
curl -s "$BASE_URL/Patient/mongo-manual-1/_history/1" -H "X-Tenant-ID: $TENANT"
```

Expected: history bundle contains multiple versions and vread for version `1` returns the initial version.

### Delete

```bash
curl -i -X DELETE "$BASE_URL/Patient/mongo-manual-1" -H "X-Tenant-ID: $TENANT"
curl -i "$BASE_URL/Patient/mongo-manual-1" -H "X-Tenant-ID: $TENANT"
```

Expected: delete succeeds; read after delete returns `410 Gone` or `404 Not Found` depending server settings.

## 6) Search, sort, and pagination checks

Create two patients for search:

```bash
cat > patient-a.json <<'JSON'
{
  "resourceType": "Patient",
  "id": "mongo-search-a",
  "identifier": [{ "system": "http://example.org/mrn", "value": "MONGO-010" }],
  "name": [{ "family": "Search", "given": ["Alpha"] }]
}
JSON

cat > patient-b.json <<'JSON'
{
  "resourceType": "Patient",
  "id": "mongo-search-b",
  "identifier": [{ "system": "http://example.org/mrn", "value": "MONGO-011" }],
  "name": [{ "family": "Search", "given": ["Beta"] }]
}
JSON

curl -s -X PUT "$BASE_URL/Patient/mongo-search-a" -H "Content-Type: application/fhir+json" -H "X-Tenant-ID: $TENANT" --data-binary @patient-a.json
curl -s -X PUT "$BASE_URL/Patient/mongo-search-b" -H "Content-Type: application/fhir+json" -H "X-Tenant-ID: $TENANT" --data-binary @patient-b.json

curl -s "$BASE_URL/Patient?name=Search&_count=1&_sort=-_lastUpdated" -H "X-Tenant-ID: $TENANT"
```

Expected: search returns matching entries and supports `_count` + `_sort` behavior.

## 7) Conditional operation checks

### Conditional create (`If-None-Exist`)

```bash
cat > patient-cond-create.json <<'JSON'
{
  "resourceType": "Patient",
  "identifier": [{ "system": "http://example.org/mrn", "value": "MONGO-020" }],
  "name": [{ "family": "Conditional", "given": ["Create"] }]
}
JSON

curl -i -X POST "$BASE_URL/Patient" \
  -H "Content-Type: application/fhir+json" \
  -H "If-None-Exist: identifier=http://example.org/mrn|MONGO-020" \
  -H "X-Tenant-ID: $TENANT" \
  --data-binary @patient-cond-create.json

curl -i -X POST "$BASE_URL/Patient" \
  -H "Content-Type: application/fhir+json" \
  -H "If-None-Exist: identifier=http://example.org/mrn|MONGO-020" \
  -H "X-Tenant-ID: $TENANT" \
  --data-binary @patient-cond-create.json

curl -s "$BASE_URL/Patient?identifier=http://example.org/mrn|MONGO-020" -H "X-Tenant-ID: $TENANT"
```

Expected: second create does not produce a duplicate match.

### Conditional update

```bash
cat > patient-cond-update.json <<'JSON'
{
  "resourceType": "Patient",
  "identifier": [{ "system": "http://example.org/mrn", "value": "MONGO-021" }],
  "name": [{ "family": "Conditional", "given": ["Update"] }]
}
JSON

curl -i -X PUT "$BASE_URL/Patient?identifier=http://example.org/mrn|MONGO-021" \
  -H "Content-Type: application/fhir+json" \
  -H "X-Tenant-ID: $TENANT" \
  --data-binary @patient-cond-update.json

curl -s "$BASE_URL/Patient?identifier=http://example.org/mrn|MONGO-021" -H "X-Tenant-ID: $TENANT"
```

Expected: one matching resource is created/updated according to conditional update semantics.

### Conditional delete

```bash
curl -i -X DELETE "$BASE_URL/Patient?identifier=http://example.org/mrn|MONGO-021" -H "X-Tenant-ID: $TENANT"
curl -s "$BASE_URL/Patient?identifier=http://example.org/mrn|MONGO-021" -H "X-Tenant-ID: $TENANT"
```

Expected: conditional delete succeeds and the search no longer returns an active match.

## 8) Optional: transaction-bundle check on replica set

Use this only if you want to manually validate transaction behavior on a transaction-capable Mongo topology.

### Start MongoDB replica set

```bash
docker rm -f hfs-mongo-rs >/dev/null 2>&1 || true
docker run -d --name hfs-mongo-rs -p 27017:27017 mongo:8.0 --replSet rs0 --bind_ip_all

docker exec hfs-mongo-rs mongosh --quiet --eval 'try { rs.status().ok } catch (e) { rs.initiate({_id:"rs0",members:[{_id:0,host:"localhost:27017"}]}) }'
```

Restart HFS with:

```bash
export HFS_STORAGE_BACKEND=mongodb
export HFS_DATABASE_URL="mongodb://localhost:27017/?replicaSet=rs0&directConnection=true"
export HFS_MONGODB_DATABASE="helios_manual_rs"
```

### Submit a transaction bundle with `urn:uuid` reference

```bash
cat > txn-bundle.json <<'JSON'
{
  "resourceType": "Bundle",
  "type": "transaction",
  "entry": [
    {
      "fullUrl": "urn:uuid:pat-1",
      "resource": {
        "resourceType": "Patient",
        "identifier": [{ "system": "http://example.org/mrn", "value": "MONGO-TXN-001" }],
        "name": [{ "family": "Txn", "given": ["Patient"] }]
      },
      "request": { "method": "POST", "url": "Patient" }
    },
    {
      "resource": {
        "resourceType": "Observation",
        "status": "final",
        "code": { "text": "manual txn observation" },
        "subject": { "reference": "urn:uuid:pat-1" }
      },
      "request": { "method": "POST", "url": "Observation" }
    }
  ]
}
JSON

curl -i -X POST "$BASE_URL" \
  -H "Content-Type: application/fhir+json" \
  -H "X-Tenant-ID: $TENANT" \
  --data-binary @txn-bundle.json
```

Expected: transaction response bundle succeeds for both entries.

## 9) Optional: `mongodb-elasticsearch` mode check

### Start Elasticsearch

```bash
docker rm -f hfs-es-manual >/dev/null 2>&1 || true
docker run -d --name hfs-es-manual -p 9200:9200 \
  -e "discovery.type=single-node" \
  -e "xpack.security.enabled=false" \
  elasticsearch:8.15.0
```

### Start HFS in composite mode

```bash
export HFS_STORAGE_BACKEND=mongodb-elasticsearch
export HFS_DATABASE_URL="mongodb://localhost:27017"
export HFS_MONGODB_DATABASE="helios_manual_composite"
export HFS_ELASTICSEARCH_NODES="http://localhost:9200"

BIN="./target/debug/hfs"
[ -f "./target/debug/hfs.exe" ] && BIN="./target/debug/hfs.exe"
"$BIN"
```

Create/search a patient as above and verify search responses are returned in composite mode.

## 10) Cleanup

```bash
docker rm -f hfs-mongo-manual hfs-mongo-rs hfs-es-manual >/dev/null 2>&1 || true
rm -f patient-v1.json patient-v2.json patient-a.json patient-b.json \
  patient-cond-create.json patient-cond-update.json txn-bundle.json
```
