# Historical corpus evidence for #1579

These complete prepared statements, bindings, and JSON EXPLAIN ANALYZE/BUFFERS
outputs were captured on 2026-09-30 during PR #1597 development. They back the
historical corpus tables in [the measurement document](../../../postgres-selective-search-1579.md).
The manifest records database settings and SHA-256 digests; the source fingerprint
identifies the then-uncommitted candidate. They are not measurements of the later
merge with `main`.

`before-timings.json` and `after-timings.json` contain the two isolated HTTP runs
per total mode. The statement files include the actual prepared page/count SQL,
bindings, and full plans for the first accurate-total request of each query.
Timeout errors are retained where the baseline could not finish. No database
connection credentials, auth headers, or resource bodies are included. Resource
IDs belong to the Synthea synthetic corpus.

Q2 is explicitly a **negative control** here: its original patient is absent and
both page and count return zero. This evidence cannot certify its required three
positive results. The resource/index counts in the manifest are PostgreSQL
estimates, not exact matches to the issue's original snapshot.
