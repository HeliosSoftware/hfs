# Run 8 — #1178 `s3` short verification pass, `main` `69ab711c6` (includes #1822)

Collected 2026-10-07 02:23Z → 05:20Z. Release build of `main` `69ab711c6` (`CARGO_BUILD_JOBS=1`, 2 h 47 m 12 s, peak RSS 16.3 GB; `outputs/build-tail.txt`), with HFS and MinIO stopped. MinIO runs on `data-1178c` (the full corpus); nothing was imported. Auth is on, `HFS_LOG_LEVEL=info`, realm lifespan 300 s, and the same guard as run 7 is active (`outputs/guard.log`; not triggered).

## Compartments and Search Parameters (#1821 / #1822) — ❌ → #1838
Session `demo`, first load after start (`outputs/pages-first.out`, `shots/`):

| page | first load | result |
|---|---|---|
| Search Parameters | 0.5 s | no notice, **0 parameters** (expected ~1,377 in R4) |
| Compartments | 0.5 s | notice *"Compartment definitions could not be loaded … the self-call to /CompartmentDefinition failed (… outbound service token is missing or invalid)"*; no compartments |

**API** (`outputs/api-listing.txt`): `GET /SearchParameter` and `GET /CompartmentDefinition` now answer **200** instead of 501, as #1822 intended. But the Bundle is **empty**, `total 0`.

**Server log:** 0 auth failures and 0 × 501 during the loads.

**Cause:** standalone S3 never seeds the conformance resources. `start_s3` says so (*"Standalone S3 seeds no conformance resources"*), and the bucket has no `SearchParameter` or `CompartmentDefinition` prefix, so #1822's scan has nothing to list → **#1838**.

**Pagination past 1,000 could not be checked:** there are no stored parameters, and seeding them would mean importing data, which this pass excludes.

## 7.e Cancel
**Pending:** #1825 (the fix for #1823) is still open. 7.e will be repeated once it is merged.
