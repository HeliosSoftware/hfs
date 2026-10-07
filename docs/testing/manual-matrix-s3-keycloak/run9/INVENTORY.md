# Run 9 — #1178 `s3` short pass for #1839 (#1838) and #1825 (#1823), `main` `a5e74a360`

Collected 2026-10-07 20:57Z → 23:00Z on the full corpus in `data-1178c` (nothing imported). Auth is on (Keycloak :8180), `HFS_LOG_LEVEL=info`, and the run-8 guard is active (`outputs/guard.log`: not triggered; MemAvailable ≥ 21,343 MB, swap ≤ 1,515 MB).

**Build:** `main` `a5e74a360`, which includes `6bcef13ee` (#1839) and `d78d3a733` (#1825). `--all-features` release, `CARGO_BUILD_JOBS=1`, HFS and MinIO stopped: **1 h 51 m 16 s, peak RSS 16.26 GB** (`outputs/build-tail.txt`).

## #1838 — conformance seed on standalone S3 ✅
Provisioned tenants: `default` only. The bucket has no other tenant prefix, and the registry `tenants/default.json` was written by the first start.

| | first start (seeds) | second start | run 8 (reference) |
|---|---|---|---|
| launch → `/metadata` 200 | **2.27 s** | **0.42 s** | < 1 s (marker 05:11:22Z, listening 05:11:22.718Z) |
| seed log | `Seeded spec SearchParameters … created=1372 existing=0 failed=0 tenant=default`; `Seeded spec CompartmentDefinitions … created=5 existing=0 failed=0` | no seed line (one `count` per type, no writes) | — |
| `default/SearchParameter` objects | 0 → **1,372** | 1,372 (newest object still from the first start, 22:49:10Z) | 0 |
| `default/CompartmentDefinition` objects | 0 → **5** | 5 | 0 |

**API** (`outputs/api-counts.txt`, tenant `default`):
- `SearchParameter?_summary=count` → **1,372**.
- `CompartmentDefinition?_summary=count` → **5** (RelatedPerson, Practitioner, Patient, Encounter, Device).
- `SearchParameter?_count=1000` → 1,000 entries plus a `next` link, so the scan listing pages past 1,000.

**Why 1,372 and not ~1,377:** the R4 spec bundle has 1,375 SearchParameters. The loader drops the ones HFS does not implement (`is_unimplemented_spec_param`) and any that fail `parse_resource` (`crates/fhir/src/search/loader.rs:242`), and duplicate-id fallbacks are dropped by the seeder. That is by design.

**UI** (`outputs/pages-first.out`, `shots/`):

| page | first load | result |
|---|---|---|
| Search Parameters | 1.0 s | no notice; **1,372** parameters; 152 resource types in the rail |
| Compartments | 0.5 s | no notice (the token wording is gone); Device 32/145, Encounter 25/145, Patient 66/145, Practitioner 59/145, RelatedPerson 32/145 |

0 auth failures and 0 × 501 during the loads.

## #1823 — 7.e Cancel on S3 ✅ (`outputs/t7e.out`, `outputs/t7e-trace-summary.txt`, `outputs/mc-trace-around-cancel.log`)
**Setup:** MinIO idle (3.6 % CPU); hfs baseline 87 MB, swap 0. `observation_flat` NDJSON was kicked off from the SQL Export page at 22:51:13Z (UI job `b08759c9`, API job `fbca7bfb`). During the first minute hfs ran at 62–87 % CPU, ~2,700 S3 calls/s.

**Cancel** pressed at **22:52:16.938Z** (303). The card reached **Cancelled** within 1 s.
- **S3 traffic** (`mc admin trace --call s3`, MinIO's own timestamps): the last Observation `GetObject`/`ListObjectsV2` was at **22:52:18.187Z, 1.25 s after the click**. The 2,029 calls after the click were in-flight requests; none came later. Over the whole job there were 71,821 Observation calls (145 LIST, 71,676 GET), against 4,620 per 10 s before the cancel.
- **hfs CPU:** 0 % from +40 s onwards; ~1 S3 call/s afterwards, background reads only.
- **Bytes received** by hfs flattened 2 s after the click (+10.7 MB). Its MinIO connections were then closed.
- **Log:** no job line after the cancel. The last scan line is at 22:51:43Z.
- **Memory:** hfs RSS stayed at **411–415 MB** after the cancel (87 MB before the job). The scan stopped, but the allocator keeps the pages. That is not part of this run's criterion and is recorded as an observation.

**Click → end of traffic: 1.25 s** (run 7 on `04bb721d5`: still scanning 1 h 31 m after the cancel).

**Negatives** (`outputs/t7e-negatives.out`):
- **Cancel on a completed job** (`done-then-cancel`): the page leaves the card **Complete**. API `DELETE` → 202 *"cancellation accepted"*, but the job stays completed and its manifest and output stay downloadable. That is documented as intentional in `in_memory.rs::cancel`: a finished job is a no-op, and output is reclaimed by the reaper.
- **Cancel on an already-cancelled job** (the 7.e job): the card stays **Cancelled**. API `DELETE` → 202 again; its status is 404.

## New finding → #1858
During the scan the in-process runner pre-resolved references for a view that never calls `resolve()`: **34,500 Encounter + 34,500 Patient `GetObject`s next to 71,676 Observation** (`outputs/t7e-fanout.txt`). That is about half the S3 reads. It also logged `reference fan-out exceeded cap; extra references left unresolved requested≈1,850 cap=1000` for every batch (`outputs/t7e-log-window.txt`).

## Issues
#1858 (new). Verified: #1838 (via #1839) and #1823 (via #1825).
