# Draft issues surfaced while implementing #859 — filed 2026-09-02

Found while resolving `PUT/DELETE [type]?[criteria]` inside transactions (PR #919). Each finding
was validated at runtime before filing; the reproductions live in the issues.

| Finding | Status |
|---|---|
| D1. Transactional `DELETE [type]/[id]` never reaches composite secondaries | Filed as **#921**. Reproduced on a SQLite/SQLite composite in synchronous mode. Relates to discussion #223's E1 durable sync outbox (which would not close it by itself) and #28's CQRS projection framing. |
| D2. The spec fixture's `POST ValueSet/$lookup` entry is not a create | Already tracked by **#868**. |
| D3. MongoDB transaction tests skip silently on the standalone testcontainer; `test-hfs` skill lacks the backend commands | Added as a [comment on #390](https://github.com/HeliosSoftware/hfs/issues/390#issuecomment-5518697101), the umbrella issue for vacuously passing suites. |
| D4. `BundleError` index refers to the sorted processing order, not the client's Bundle index | Filed as **#922**. Reproduced on the batch_conformance harness. |
