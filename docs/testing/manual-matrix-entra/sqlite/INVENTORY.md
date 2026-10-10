# #1182 manual pass — Microsoft Entra ID / `sqlite`, full corpus

Evidence for the row **"Entra ID / sqlite — corpus completo"** on #1182.

- **Build:** `main` at `3b54315b0` (includes #1872), `cargo build --release --all-features`; binary reports `hfs 0.2.4 (git 3b54315b0)`.
- **Server:** `127.0.0.1:18090`, own SQLite database, started from the repository root so `./data` loads, `HFS_LOG_LEVEL=info`, subscriptions on.
- **Auth:** Entra ID v2 (JWKS + issuer + audience), interactive browser login on, `HFS_UI_LOGIN_SCOPES` without `api://`. Service token: client credentials with `scope=api://<client-id>/.default`, `roles: ["system/*.cruds"]`, 65 min, lifetime not lengthened.
- **Corpus:** Synthea, 18,955,865 resources (`manifest.json`).
- **Dates:** 2026-10-08 → 2026-10-10.

## Masking

Every value below is replaced in this directory. The scrubber checked each file against the real values, not only against patterns.

| Masked as | What |
|---|---|
| `<tenant>`, `<client-id>`, `<id>` | Entra tenant id, the HFS and read-only client ids (including the 8-character `aud` prefixes printed by `claims.py`), other directory ids |
| `<entra_*>`, `<hfs_client_secret>`, `<ro_client_secret>` | client secrets (none were ever written to a file here) |
| `<jwt>`, `Bearer <masked>` | any access, id or refresh token; also the subscription fixture's static `Authorization` header value |
| `<session-id>`, `hfs_session=<session-id>` | web login session ids (cookie) |
| `<subject>` | the signed-in account's Entra subject (`sub`) |
| `<email>`, `<quick-tunnel>.trycloudflare.com` | account e-mail, the two (closed) SSH quick-tunnel host names |

- **Not included:** `private/` (session id, Playwright storage state, the A4 expiring token), the env and credential files, the binary, the HFS database, the full HFS log (`log-excerpts/hfs-key-lines.log` holds the relevant lines, scrubbed), the quick-tunnel logs, and the large `$sql-run` / export outputs (sizes and counts are in `run/t6.out`, `run/t7.out`).
- **Screenshots** were checked one by one. They show no URL bar, token, cookie, id or account name (the avatar shows only the initial). The `*-redirect.png` shots are the Microsoft sign-in page from T8 attempt 1 (see Deviations).

## Results

| Step | Result | Evidence |
|---|---|---|
| T0 build | ✅ 2 h 59 min 16 s, peak 16.56 GB | — |
| T1 start | ✅ `Authentication ENABLED` (with audience), `Interactive browser login ENABLED`, `Subscriptions engine ENABLED`; seed 1,372 SearchParameter + 5 CompartmentDefinition; `/metadata` 200 after 2.6 s | `run/t1-smoke.out`, `log-excerpts/` |
| A1 no token | ✅ 401 *Missing Authorization header*; open endpoints 200; `/ui` → 303 `/ui/login` → 303 Entra authorize | `run/a1-a5.out` |
| A2 service token | ✅ GET 200, POST 201 (probe Patient `EntraA2`) | 〃 |
| A3 read-only client | ✅ GET 200, POST 403 *insufficient scope for create on Patient* | 〃 |
| A4 bad tokens | ✅ tampered signature 401 *Invalid signature*; really expired token (exp 23:03:14Z) 401 *Token expired*; Microsoft Graph token (iss `sts.windows.net`, v1.0) 401 *Invalid signature* | 〃, `run/a4-expiring-token-note.txt` |
| A5 discovery | ✅ Entra endpoints advertised (`end_session_endpoint` not advertised — logout only) | `run/a1-a5.out` |
| Login | ✅ 04:03:24Z, session roles `system/bulk-submit`, `user/*.cruds`; re-login 21:18:04Z (same roles) | `run/session-roles.txt` |
| Service-token expiry | ✅ minted 01:01:26Z, expired 02:01:26Z; self-call pages (Search Parameters 1,372, Compartments, SQL pages) kept working under the session at 04:10Z | `run/selfcall-after-expiry.out`, `shots/selfcall-*.png` |
| T2 batch/transaction | ✅ 6.1 refusal text, 6.2 9 + 8 created, 6.3 662 created, 6.4 both refusals; 680 resources | `run/t2.out`, `shots/t2-t2-*.png` |
| T3 import | ✅ two submissions (see T3 below), 0 errors; every type indexed | `run/t3-*`, `run/t3b-*`, `run/reindex-*`, `run/index-coverage.txt` |
| T4 search | ❌ **#1930** on 4.11 only; the other 84 + 19 queries are 200 with the expected totals | `run/t4.out`, `run/t4-411.txt` |
| 7.5 counts | ✅ 18,957,925 stored = 18,955,865 + 1,372 SP + 5 CD + 680 (T2) + 3 (T4 fixtures); no corpus type short | `run/t75-counts.txt` |
| T5 bulk export | ✅ 5.1–5.13 including negatives, failure path (5.13 404, retry, delete) and cancel (5.5) | `run/t5.out` |
| T6 View Definitions | ✅ save, preview, lint, completion, duplicate/delete, 422 negative; `$sql-run` ×4 formats 11,706 rows; 401 without token | `run/t6.out` |
| T7 SQL export | ✅ / ⚠️ #1473 — 11 exports complete with the expected rows; the 4 that run SQL Query `tall_female_patients` answer 422 in seconds because `observation_flat` exceeds the 1,000,000-row source cap (`HFS_SOF_SQLQUERY_MAX_SOURCE_ROWS_PER_VD`); negatives correct; 7.e Cancelled | `run/t7.out`, `run/t7-negatives-visible.txt` |
| T8 subscriptions | ✅ handshake, 3 notifications, Condition 0, `$status`/`$events` 200, `$status` 401 without token, receiver down → delivered on return | `run/t8-t9.out`, `run/webhook.log` |
| T9 dashboard | ✅ counts and first-try rate, retry exhaustion → Failing / `error`, sorts, reactivation, restart (rehydrated, session survives, 24 h window resets by design #586), engine disabled notice | `run/t8-t9.out`, `run/t9-sort.txt` |

The T4 search was not an Entra-specific failure. Nothing in this pass failed only because of Entra auth.

## T3 timings

| Phase | Time |
|---|---|
| Submission 1 (`8a46125e…`, full manifest) | 04:07:14Z → aborted by the swap guard 05:19:34Z (**1 h 12 min 20 s**); 9,362,827 written, 0 errors; 16 files complete, Observation 648,600 / 7,699,881 |
| Submission 2 (`synthea-entra-sqlite-rest`, `d0b3779c…`, `run/manifest-rest.json`: Observation from line 648,601 + the 7 unstarted files, 9,593,038) | 13:11:52Z → last write 14:08:15Z (**56 min 23 s**), Completed 14:08:55Z, 8 outputs, 0 errors |
| Automatic reindex after submission 2 | 14:08:30Z → 18:31:26Z (**~4 h 23 min**), 10,241,966 resources, 188,025,313 entries, 0 errors |
| `$reindex` of the 16 submission-1 types (one `POST /{type}/$reindex` each, service token) | sum of completed jobs **10,228 s (2 h 50 min)**; wall clock 18:32:18Z → 22:14:45Z (3 h 42 min) including an ExplanationOfBenefit job cancelled by the swap guard and an HFS restart |

## Deviations

All of them, with timestamps, are in `run/deviations.txt`.

1. **Login:** Angela signed in through an SSH-only Cloudflare quick tunnel, with her own account, twice. The tunnel was closed right after each `web login completed`.
2. **UI steps:** replayed from the shell with her `hfs_session` cookie and `Sec-Fetch-Site: same-origin`. Playwright with the same cookie was used for the Batch page, and the status/card fragments were polled like the browser.
3. **Swap guard during T3:** the guard aborted submission 1. With Angela's approval, the sustained-growth rule was removed (idle-page swap-out, not memory pressure). The absolute swap cap (baseline 1,889 MB + 3 GiB), MemAvailable < 2 GiB and the disk floor of 135 GiB were kept, and the rest of the corpus was submitted from a byte-exact Observation suffix.
4. **`$reindex` of submission-1 types:** run with the service token (`system/reindex` is an operation scope that the user scopes do not cover).
5. **Session lost:** at 14:38:24Z the refresh was refused with AADSTS50076 (the tenant requires MFA again after ~10.5 h). This is not an HFS defect. Angela signed in again at 21:18:04Z, after an HFS restart; the session row had been removed, so the old cookie got 303 → `/ui/login`.
6. **Swap guard during the reindex:** the guard tripped again at 20:54:26Z (5,051 MB). `reindex-guard.sh` cancelled the running job (data kept). On Angela's instruction (option 1), HFS was restarted at 21:02:43Z (swap 4,840 → 1,763 MB) and the reindex resumed with the same guard.
7. **T8 attempt 1 (00:30Z) discarded:** the Playwright storage state kept the old session's cookie expiry, so the Batch page went to the Entra sign-in page. A debug call then ran one real Batch upload (+3 Encounters). T8/T9 were rerun from the start.
8. **Port 9999:** a stale listener left from an earlier pass was stopped when the T8 receiver started.

## Layout

| Path | What |
|---|---|
| `run/` | outputs of every step, guard log, reindex logs, counts, deviations |
| `scripts/` | the scripts that produced them (credentials are read at run time from a file outside the repo) |
| `fixtures/` | resources and Libraries used by T4–T9 |
| `shots/` | screenshots |
| `log-excerpts/hfs-key-lines.log` | startup, login, session, reindex, export-failure and subscription lines from the HFS log |
