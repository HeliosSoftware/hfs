# SQL-on-FHIR `$viewdefinition-run`, `$viewdefinition-export`, and `$sqlquery-run` Spec Inconsistencies

Items where the [SQL-on-FHIR v2 `$viewdefinition-run`](https://build.fhir.org/ig/FHIR/sql-on-fhir-v2/OperationDefinition-ViewDefinitionRun.html), [`$viewdefinition-export`](https://build.fhir.org/ig/FHIR/sql-on-fhir-v2/OperationDefinition-ViewDefinitionExport.html), and [`$sqlquery-run`](https://build.fhir.org/ig/FHIR/sql-on-fhir-v2/OperationDefinition-SQLQueryRun.html) OperationDefinitions are internally inconsistent, ambiguous, or silent on behavior that implementations must nevertheless decide — including places where the sibling operations drift from each other. Each entry records the spec text, the conflict, our chosen behavior, and the rationale.

---

## A — `return Binary 1..1` vs. raw payload in examples

**Spec text (parameter table):**

> **return** — Binary, 1..1 — "Transformed data encoded in the requested output format."

**Spec examples** (Examples 1–4 on the OperationDefinition page) all return raw bytes with the appropriate `Content-Type`, never a FHIR `Binary` JSON envelope. From Example 1:

```http
HTTP/1.1 200 OK
Content-Type: text/csv
Transfer-Encoding: chunked

id,birthDate,family,given
pt-1,1990-01-15,Smith,John
```

**Inconsistency:** The parameter type `Binary` implies a FHIR resource wrapper (`{"resourceType":"Binary","contentType":"...","data":"<base64>"}`), but every worked example returns the unwrapped payload directly.

**Our behavior:** Raw bytes with the matching `Content-Type` (`text/csv`, `application/json`, `application/x-ndjson`, `application/octet-stream`).
- `crates/sof/src/handlers.rs:417` (sof-server)
- `crates/rest/src/handlers/sof/run.rs:752` (HFS REST)

**Rationale:** Matches the spec's own examples and the behavior of reference implementations (Pathling, sof-js). A wrapped `Binary` would require clients to base64-decode before parsing CSV/NDJSON, which no spec example demonstrates.

**Recommendation:** Treat as a spec documentation gap. No change required.

---

## D — `resource 0..* Resource` — Bundle unwrap unspecified

**Spec text (parameter table):**

> **resource** — Resource, 0..* — "FHIR resources to transform instead of using server data."

**Spec Example 3** demonstrates passing discrete resources as separate parameter entries:

```json
{
  "resourceType": "Parameters",
  "parameter": [
    { "name": "viewResource", "resource": { "resourceType": "ViewDefinition", "...": "..." } },
    { "name": "resource", "resource": { "resourceType": "Patient", "id": "pt-1", "...": "..." } },
    { "name": "resource", "resource": { "resourceType": "Patient", "id": "pt-2", "...": "..." } }
  ]
}
```

**Inconsistency:** `Bundle` is a `Resource`, so a Bundle technically satisfies `type=Resource`. The spec is silent on whether the server should:
1. Treat the Bundle as opaque (no entries iterated, ViewDefinition runs against the Bundle itself), or
2. Unwrap `Bundle.entry[*].resource` and apply the ViewDefinition to each entry.

**Our behavior (asymmetric across binaries):**
- **sof-server** unwraps `Bundle.entry[*].resource` as a convenience — `crates/sof/src/models.rs:332-353`.
- **HFS REST** does not unwrap; the Bundle flows through as a single resource — `crates/rest/src/handlers/sof/run.rs`.

**Rationale for the asymmetry:** sof-server is the stateless CLI/server path where users commonly pipe FHIR Bundles directly; unwrapping matches their expectation. HFS REST is integrated with persistent storage, where Bundle uploads have explicit batch/transaction semantics elsewhere.

**Open decision:** Should HFS REST adopt sof-server's unwrap behavior to remove the footgun? Pending direction. If yes, the change point is the `resource` parameter handler in `crates/rest/src/handlers/sof/run.rs`, mirroring `crates/sof/src/models.rs:332-353`.

**Recommendation:** File a clarification with the SOF working group; in the meantime, make HFS REST match sof-server (Bundle entries flattened).

---

## E — Return type shape diverges between `$viewdefinition-run` and `$sqlquery-run`

**Spec text:**

- `$viewdefinition-run` — `return Binary 1..1` for every supported `_format` (`csv`, `json`, `ndjson`, `parquet`).
- `$sqlquery-run` — `return Binary` for the four flat formats, **but** `return Parameters` (with a repeating `row` parameter, one per result row) when `_format=fhir`.

**Inconsistency:** `$sqlquery-run` introduces a sixth format (`fhir`) that flips the return type to `Parameters`. `$viewdefinition-run` has no equivalent polymorphism — it always returns `Binary` and does not offer a `fhir` format at all. Two sibling operations in the same IG, designed for the same downstream consumers, should not disagree on either the set of supported formats or the return-type contract.

**Our behavior:**

- `$viewdefinition-run`: returns raw bytes for `csv`/`json`/`ndjson`/`parquet` (see entry A) — `crates/rest/src/handlers/sof/run.rs`.
- `$sqlquery-run`: returns raw bytes for flat formats and a `Parameters` resource for `_format=fhir` — `crates/rest/src/handlers/sof/sqlquery.rs`.

**Recommendation:** File a clarification with the SOF working group asking for one of:

1. Add `_format=fhir` to `$viewdefinition-run` with the same `Parameters` + repeating-`row` return shape, or
2. Drop `_format=fhir` from `$sqlquery-run` and let clients run a follow-up transformation if they need FHIR-typed rows.

Either way, the supported `_format` set and the return type matrix should be identical across the two ops.

---

## F — Streaming guidance present for `$viewdefinition-run`, absent for `$sqlquery-run`

**Spec text:**

- `$viewdefinition-run` says streaming **MAY use chunked transfer encoding for large result sets**, and every worked example shows `Transfer-Encoding: chunked` in the response headers (see entry A).
- `$sqlquery-run` says **nothing** about streaming, chunking, async, or polling. No worked example shows `Transfer-Encoding` at all.

**Inconsistency:** `$sqlquery-run` is the op more likely to produce unbounded result sets — it executes arbitrary SQL, with no `_limit` or `_since` parameter to constrain output (`$viewdefinition-run` has both). The op that most needs streaming guidance has none; the op that has more natural row caps gets the streaming paragraph.

**Our behavior:** We stream both ops identically — chunked transfer encoding with raw bytes — because that is the only practical implementation for parquet/NDJSON output regardless of which op served it.

- `crates/rest/src/handlers/sof/run.rs`
- `crates/rest/src/handlers/sof/sqlquery.rs`

**Recommendation:** Add a streaming/chunked-transfer paragraph to `$sqlquery-run` that mirrors the one in `$viewdefinition-run`, and explicitly call out NDJSON as the streaming-friendly format on both. Better still, lift the streaming language into a section both OperationDefinitions reference, so the rule is stated once.

---

## G — `Accept: application/octet-stream` semantics undefined on both ops

**Spec text:** Both operations declare `return Binary` but neither specifies how a client signals "give me the raw payload" vs. "give me a FHIR `Binary` resource envelope with base64-encoded `data`". The OperationDefinition pages do not mention the `Accept` header at all; only the worked examples imply the answer (raw bytes — see entry A).

**Inconsistency:** The standard FHIR convention for reading a `Binary` resource is:

- `Accept: application/octet-stream` → server returns the raw payload with `Content-Type` set to the underlying media type (`text/csv`, `application/x-ndjson`, `application/vnd.apache.parquet`, …).
- `Accept: application/fhir+json` (or `+xml`) → server returns a `Binary` resource with `contentType` and base64-encoded `data`.

Neither SoF op cites this convention, and the per-format implications matter:

- **Parquet** is the worst case for the envelope form — base64 inflates the payload ~33% and forces clients to decode before they can mmap/scan the file. Anyone asking for parquet wants raw bytes.
- **NDJSON** only streams meaningfully as raw bytes; wrapping it in a base64 `Binary.data` defeats the format.
- **CSV / JSON** can go either way, but clients should be able to ask for raw with `Accept: application/octet-stream`.

**Our behavior:** Both ops always return raw bytes with the format's native `Content-Type`, regardless of `Accept`. We do not currently honor `Accept: application/fhir+json` by wrapping the payload in a `Binary` envelope.

- `crates/rest/src/handlers/sof/run.rs`
- `crates/rest/src/handlers/sof/sqlquery.rs`

**Recommendation:** The spec should state, once, in a shared section both ops reference, that:

1. When `Accept: application/octet-stream` is present (or absent — i.e. default), the response body is the raw output in the requested `_format` and `Content-Type` reflects the format's native media type. `Transfer-Encoding: chunked` is allowed for streamable formats.
2. When a FHIR media type is requested, the server MAY return a `Binary` resource envelope with the payload base64-encoded — and SHOULD document whether large/streaming formats are supported in this mode at all (parquet and NDJSON realistically are not).

---

## See also

Other spec ambiguities surfaced during audit but **not** classified as inconsistencies (they are deliberate deployment-policy deviations or out-of-scope conveniences). Tracked separately in the audit log:

- `_limit` upper bound on `$viewdefinition-run` (we cap at 10000; spec is unbounded). `$viewdefinition-export` rejects `_limit` outright — the spec's input parameter table for the export op does not list it, and the bulk-export contract is unbounded by design.
- `patient` query-string comma-splitting into multiple references (spec cardinality is 0..1 on `$viewdefinition-run`; 0..* on `$viewdefinition-export`).
- `source` URI scheme enumeration (`file://`, `http(s)://`, `s3://`, `gs://`, `azure://` — spec says only "URI or bucket name").
- R4-only build exposing type+instance routes (spec's R4 OperationDefinition restricts to system-level).
- GET requests carrying `viewResource`/`resource` body (spec text reserves these for POST).
- `$viewdefinition-export` adds `status: in-progress` to its polling Parameters body — the spec's status-polling parameter table lists only `exportId` and `estimatedTimeRemaining`. Additive, no behavioral impact for spec-compliant clients.
