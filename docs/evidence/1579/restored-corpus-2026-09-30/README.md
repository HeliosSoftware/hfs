# Restored full-corpus verification of #1579

Source commit `803270e4d` merges `main` without changing the three selective
query shapes. `manifest.json` records runtime conditions, source hashes and
artifact digests. `verification.json` records the 208 passing debug tests;
the HTTP measurements use a separate release binary.

`primary-timings.json` has the twelve clean warm HTTP requests, two per query
and total mode. `first-warm-pass-timings.json` retains the earlier twelve
requests, including three cargo-only activity samples in its host monitor.
The primary pass has zero compiler/cargo samples during its HTTP window.
No competing searches ran on this database copy. Both passes captured
identical SQL and typed bindings, and each returned the same ordered IDs.

`warmup.json` preserves the much slower first observed accurate-total Q1
execution (37.402620 s) and Q3 execution (2.645491 s). Their cache state was
not controlled. These observations are outside a cold-cache guarantee and
require separate diagnosis; restarting HFS does not establish cold caches.

`plans/` contains actual prepared page/count statements, bindings and complete
EXPLAIN ANALYZE/BUFFERS JSON. The additional export GIN index from schema 45
is absent from every recorded search plan. `baseline-q2/` replays the genuine
pre-fence SQL captured at base `4427ae16c5dda7338e43032ac49d7c4e79b9875f`,
changing only its tenant and patient binds. Its first count had uncontrolled
cache state; a subsequent count plan and page are preserved separately, with
read blocks still present in their PostgreSQL plans.

The original Q2 patient and its indexed references are absent. The replacement
is an existing patient with 106 Observation candidates and exactly three 164.1 cm
height matches. Ground truth evaluates those resource bodies independently
of the composite index predicate. No clinical resources were added or changed.
Corpus counts match the issue after accounting for 1377 conformance rows.
The source database remained stopped and unchanged throughout.

[#1625](https://github.com/HeliosSoftware/hfs/issues/1625) tracks first-execution
latency separately from the isolated warm acceptance measurements.
