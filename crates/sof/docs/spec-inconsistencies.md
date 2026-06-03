# SQL-on-FHIR Operation Spec Inconsistencies

Items where the SQL-on-FHIR v2 operation definitions are internally inconsistent, ambiguous, or silent on behavior that implementations must nevertheless decide — including places where the sibling operations drift from each other. The family now comprises **four** sibling operations in the same IG:

- [`$viewdefinition-run`](https://build.fhir.org/ig/HL7/sql-on-fhir/OperationDefinition-ViewDefinitionRun.html) — synchronous, returns `Binary`.
- [`$viewdefinition-export`](https://build.fhir.org/ig/HL7/sql-on-fhir/OperationDefinition-ViewDefinitionExport.html) — asynchronous bulk export.
- [`$sqlquery-run`](https://build.fhir.org/ig/HL7/sql-on-fhir/OperationDefinition-SQLQueryRun.html) — synchronous, returns `Binary` or `Parameters`.
- [`$sqlquery-export`](https://build.fhir.org/ig/HL7/sql-on-fhir/OperationDefinition-SQLQueryExport.html) — asynchronous bulk export.

Each entry records the relevant spec text, the conflict, and a recommended spec fix. Entries are written to be filed as standalone issues.

---

## A — `return Binary 1..1` vs. raw payload in examples

**Spec text** — `$viewdefinition-run` return parameter:

> **return** — Binary, 1..1 — "Transformed data encoded in the requested output format."

`$sqlquery-run` similarly declares `return` as `Binary` for the flat formats. Yet the worked examples return raw bytes with the appropriate `Content-Type`, never a FHIR `Binary` JSON envelope. From the `$viewdefinition-run` instance example:

```http
HTTP/1.1 200 OK
Content-Type: text/csv
Transfer-Encoding: chunked

id,birthDate,family,given
pt-1,1990-01-15,Smith,John
```

**Inconsistency:** The parameter type `Binary` implies a FHIR resource wrapper (`{"resourceType":"Binary","contentType":"...","data":"<base64>"}`), but every worked example returns the unwrapped payload directly. A client coding to the declared type would base64-decode a Binary envelope; a client coding to the examples would read raw CSV/NDJSON. The two cannot both be right.

**Recommendation:** Resolve the type-vs-example contradiction explicitly, once, in text both run operations reference. Either:

1. Keep the `Binary` type but add normative narrative stating that — as with a FHIR `Binary` read under `Accept: application/octet-stream` — the response body is the raw payload in the format's native media type, *not* a serialized `Binary` resource; the `Binary` type denotes the binary stream, not a JSON envelope. Or
2. Change the declared return so it no longer implies a JSON resource envelope (e.g. describe the output as a binary stream/attachment), and mark the worked examples normative.

Either way, state when (if ever) a serialized `Binary` resource envelope is returned, so the type and the examples stop disagreeing.

---

## B — `resource 0..* Resource`: Bundle unwrap unspecified

**Spec text** — `$viewdefinition-run` input parameter:

> **resource** — Resource, 0..* — "FHIR resources to transform instead of using server data."

The worked example passes discrete resources as repeated parameter entries:

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

**Inconsistency:** `Bundle` *is* a `Resource`, so a `Bundle` satisfies `type=Resource`. The spec does not say whether the server should:

1. Treat the `Bundle` opaquely — run the ViewDefinition against the `Bundle` resource itself, or
2. Unwrap `Bundle.entry[*].resource` and run the ViewDefinition against each entry.

Because `resource` is already repeatable for passing discrete resources, both readings are plausible and a client cannot predict which it will get. Passing a `Bundle` of Patients is a natural thing to do, and the two interpretations produce completely different result sets.

**Recommendation:** The spec should state one behavior for a `Bundle` supplied as a `resource` value. The least-surprising rule — given that `resource` already accepts repeated discrete resources — is to unwrap `Bundle.entry[*].resource` and run the ViewDefinition against each entry. Add a worked example with a `Bundle` input to make the chosen semantics unambiguous.

---

## C — Supported `_format` set and return type diverge across the four sibling operations

**Spec text:**

| Operation | Delivery | Supported `_format` | Return shape |
|-----------|----------|---------------------|--------------|
| `$viewdefinition-run` | synchronous | `csv`, `json`, `ndjson`, `parquet` | `Binary 1..1` |
| `$sqlquery-run` | synchronous | `csv`, `json`, `ndjson`, `parquet`, **`fhir`** | `Binary` (flat) / **`Parameters`** (`_format=fhir`) |
| `$viewdefinition-export` | asynchronous | `csv`, `json`, `ndjson`, `parquet` | async manifest → files |
| `$sqlquery-export` | asynchronous | `csv`, `json`, `ndjson`, `parquet` | async manifest → files |

For `$sqlquery-run`, `_format=fhir` returns a `Parameters` resource with a repeating `row` parameter (one per result row, each row's columns as `part`s, SQL `NULL` represented by omitting the part).

**Inconsistency:** Across four sibling operations in one IG, designed for the same downstream consumers, the `fhir` format and the `Parameters` return type are a **singleton**: only `$sqlquery-run` offers them. `$viewdefinition-run` has no `fhir` format and always returns `Binary`; both export operations omit `fhir` and deliver files. Most pointedly, `$sqlquery-export` — the asynchronous counterpart of `$sqlquery-run` — drops `fhir`, so a client that adopts FHIR-typed rows synchronously has no async path to the same shape, and no equivalent in any of the other three operations.

**Recommendation:** Make the supported-format enumeration and the return-type matrix consistent across all four operations, defined once in a shared section the OperationDefinitions reference rather than per-operation. Choose one of:

1. Promote `fhir` to a first-class format defined in the shared section and supported by all four operations — including deciding its asynchronous semantics for the two export operations (e.g. an NDJSON-of-rows or `Parameters`-per-file manifest). Or
2. Remove `fhir` from `$sqlquery-run` so all four operations share the flat-format set and a uniform `Binary`/file return contract, and let clients run a follow-up transformation if they need FHIR-typed rows.

The supported `_format` set and the return type SHALL be identical across operations that share a delivery model, and explicitly reconciled across the sync/async pair for each query type.

---

## D — Streaming/chunked-encoding guidance present on one operation, absent on the other three; and it conflates two concepts

**Spec text:**

- `$viewdefinition-run`: "MAY use chunked transfer encoding for large result sets," and the worked example shows `Transfer-Encoding: chunked` on a `text/csv` response.
- `$sqlquery-run`: says **nothing** about streaming, chunking, or framing.
- `$viewdefinition-export` and `$sqlquery-export`: use the asynchronous bulk model; neither says anything about the framing of the file downloads.

**Inconsistency:** Two problems — one of wording, one of coverage.

1. **The text conflates two independent concepts.** `Transfer-Encoding: chunked` is an HTTP/1.1 message-framing mechanism (RFC 9112 §7.1), independent of `Content-Type`: *any* payload — CSV, JSON, NDJSON, parquet, `application/octet-stream` — can be sent chunked. The choice between `Content-Length` and chunked framing depends solely on whether the server knows the body size before emitting the first byte, never on the `_format`. A separate, genuinely format-sensitive question is **incremental result production** — whether the server can emit output before the full result set is materialized (NDJSON and CSV are trivially row-incremental; a JSON array needs bracket/comma bookkeeping; parquet must finalize its footer last but can still flush row groups progressively). The current text reads as if chunked transfer were a property of large or "streamable" formats; it is not. The IG's own CSV example proves the point — it shows `Transfer-Encoding: chunked` on `text/csv`, contradicting any reading that ties chunking to NDJSON.

2. **The guidance is attached to only one of four sibling operations.** `$sqlquery-run` executes arbitrary SQL — joins and aggregations that can produce large or hard-to-size result sets — yet receives no streaming guidance, while the simpler `$viewdefinition-run` does. The two export operations exist precisely for large extracts but never relate their file-download framing to the run operations' streaming language.

**Recommendation:** In a shared section all four operations reference, the spec should:

1. State that `Transfer-Encoding: chunked` MAY be used for the response of **any** `_format` — it is a transport-framing choice, not a format property — and remove any wording that implies it is reserved for "streamable" formats or singles out NDJSON.
2. Separate, explicitly, the two concepts the current text conflates: chunked transfer encoding (HTTP transport framing) vs. incremental result production (a server/engine capability that varies by format).
3. Give `$sqlquery-run` the same streaming language as `$viewdefinition-run`.
4. Note that the two export operations' file downloads MAY likewise be chunked, again format-agnostic, while the operation response itself follows the asynchronous model.

---

## E — `Accept` header semantics: raw payload vs. `Binary` envelope undefined on both run operations

**Spec text:** Both `$viewdefinition-run` and `$sqlquery-run` declare a `Binary` return. `$viewdefinition-run` now addresses `Accept` only for *format negotiation*:

> "Servers MAY honour the HTTP `Accept` header to negotiate an alternative format when `_format` is not supplied. When `_format` is supplied, its value SHALL take precedence over `Accept`."

Neither operation says how a client signals "give me the raw payload" vs. "give me a FHIR `Binary` resource envelope with base64-encoded `data`."

**Inconsistency:** The base FHIR convention for reading a `Binary` resource is defined in the [Binary resource page, "Serving Binary Resources using the RESTful API"](https://www.hl7.org/fhir/binary.html#rest):

> "When a read request is made with a FHIR type in the Accept header (e.g. `application/fhir+xml` or `application/fhir+json`) the Binary resource is returned in the requested FHIR format. When the read request has some other type in the `Accept` header, then the content should be returned with the content type stated in the resource in the `Content-Type` header … the intent is that unless specifically requested, the FHIR XML/JSON representation is not returned."

In other words:

- `Accept: application/octet-stream` (or the native media type) → raw payload, `Content-Type` set to the underlying media type (`text/csv`, `application/x-ndjson`, the parquet media type, …).
- `Accept: application/fhir+json` (or `+xml`) → a `Binary` resource with `contentType` and base64-encoded `data`.

The SoF operations cite neither, and the `Accept` text that does exist governs a *different* axis (which `_format`), not the raw-vs-envelope question. The per-format stakes are real:

- **Parquet** is the worst case for the envelope form — base64 inflates the payload ~33% and forces clients to decode before they can mmap/scan the file.
- **NDJSON** only streams meaningfully as raw bytes; a base64 `Binary.data` defeats the format.
- **CSV / JSON** can go either way, but a client should be able to ask for raw explicitly.

**Recommendation:** In the shared section both run operations reference, state:

1. With `Accept: application/octet-stream` (or no `Accept`, i.e. default), the response body is the raw output in the requested `_format`, with `Content-Type` set to the format's native media type; chunked framing is allowed (see entry D).
2. With a FHIR media type (`application/fhir+json`/`+xml`), whether the server returns a `Binary` resource envelope with base64-encoded `data` — and explicitly whether the envelope form is supported for large/streaming formats (parquet, NDJSON) at all.
3. That this raw-vs-envelope axis is distinct from the existing `Accept`-vs-`_format` precedence rule, so the two are not conflated.

---

## F — Export operations specify `303 See Other` on completion but cite a pattern that uses `200 OK`

**Spec text** — both `$viewdefinition-export` and `$sqlquery-export`:

> "This operation follows the FHIR Asynchronous Interaction Request Pattern."

Their status-code flow, however, is:

- Kick-off → `202 Accepted` with a `Content-Location` polling URL.
- Polling while processing → `202 Accepted`.
- **Completion → `303 See Other` redirect**; "Client retrieves results from the redirect location," then downloads files from `output.location`.

The cited [FHIR Asynchronous Bulk Data Request Pattern](https://www.hl7.org/fhir/async-bulk.html) specifies the opposite for completion:

> "HTTP status of `200 OK`" with "A body containing a JSON object providing metadata, and links to the generated Bulk Data files."

That pattern uses `200 OK` with the manifest **in the body** of the status-poll response, and never uses `303 See Other`.

**Inconsistency:** Both export operations claim conformance to the FHIR async pattern but redefine its completion response — from `200 OK` + inline manifest to a `303 See Other` redirect to a separate result resource. This is substantive: it changes the status code clients branch on, and it moves the manifest (the `exportId`/`status`/`output` Parameters) from the poll-response body to behind a redirect. Clients written to the standard pattern — including existing FHIR Bulk Data clients, which the pattern targets — will not follow the redirect or will mis-handle the `303`. (The two SoF export operations are at least consistent with *each other* on `303`; both diverge from the pattern they cite.)

**Recommendation:** Make the export operations and the cited pattern agree. Either:

1. Align with the FHIR Asynchronous Interaction Request Pattern: on completion, return `200 OK` with the manifest (`exportId`/`status`/`output` Parameters) directly in the status-poll response body, and remove the `303` redirect. Or
2. If a redirect to a distinct result resource is intentional, stop citing the FHIR async pattern as conformant-as-is and instead define the `303`-based flow explicitly as a documented deviation, spelling out how it interoperates with existing async/bulk clients.

Apply the same resolution to both export operations. Relatedly, the "request headers sent during status polling apply only to the status response, not the final operation result" note only makes sense under a redirect model — it should be reconciled with whichever completion model is chosen.

---

## Minor ambiguities and silent points

Lower-severity gaps surfaced during the same review. Each is a place the spec is silent or under-specified rather than self-contradictory; worth a sentence of clarifying text.

- **`_limit` bounds and export applicability.** `$viewdefinition-run` and `$sqlquery-run` accept `_limit` with no defined upper bound or documented server-cap behavior; neither export operation lists `_limit` at all, so there is no documented way to bound an asynchronous extract. The spec should state server-cap semantics for the run operations and whether the omission of `_limit` from the export operations is intentional.
- **`patient` cardinality split.** `patient` is `0..1` on `$viewdefinition-run` but `0..*` on the export operations and `$sqlquery-export`. The spec does not say whether a single comma-delimited `patient` query parameter may carry multiple references on the `0..1` operations. Clarify.
- **`source` scheme enumeration.** `source` is a free `string` ("URI or bucket name") with no enumerated scheme set (`file://`, `http(s)://`, `s3://`, `gs://`, `azure://`, …). The spec should enumerate accepted schemes or reference a registry, so implementations agree on what a portable `source` value looks like.
- **R4 scope vs. type/instance routes.** Where R4 OperationDefinitions restrict an operation to system level, the spec should state the expected behavior of type- and instance-level routes under R4 vs. R5+.
- **GET requests carrying a body.** Worked examples reserve `viewResource`/`resource`/`queryResource` bodies for POST. The behavior of a GET request that nonetheless carries such a body is unspecified.
- **Bare resource body shortcut.** Examples only show the `Parameters` envelope. The spec should state whether a bare `ViewDefinition` (for the run/export view operations) or bare `Library` (for the SQL operations) body is an acceptable shorthand for a `Parameters` body containing a single `viewResource`/`queryResource` entry, since it is an unambiguous and common ergonomic convenience.
- **Status enumeration (now clarified).** The 2.1.0-pre export operations enumerate `status` values (`accepted`, `in-progress`, `completed`, `cancelled`, `failed`) and include `status 1..1` in the polling Parameters — resolving an earlier gap where `status`/`in-progress` were absent from the status-polling parameter table. Noted here as resolved.
