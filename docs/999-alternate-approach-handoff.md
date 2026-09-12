# #999 — internal alternate approach: handoff

**Status: parked, deliberately unpushed.** Nothing about this branch has been posted to
GitHub. It exists so the work is not lost while [PR #1026][pr1026] — the external fix for
the same issue — is given a chance to land.

**Branch:** `fix/999-mongodb-aggregation-cursor-intersection`
**Base:** `fb56e7dea` (applies cleanly onto `a5322d9e7` as of 2026-09-12; `git merge-tree`
reports zero conflicts)

```
6348a30dc perf(persistence): hint idx_search_composite on bounded candidate verification
51dcc0f39 fix(persistence): bound mongodb matching_resource_ids instead of unbounded distinct (#999)
```

Delete this file before opening any PR from this branch — it is working notes, not project
documentation.

---

## 1. Why this exists, and the decision rule for picking it up

[#1026][pr1026] by `bomanaps` fixes #999 by a different route. It has **changes requested**
(see the review on that PR) on three blocking findings. This branch is an independent
implementation that measured better, built while the comparison was being made.

**Do not open a competing PR while #1026 is live.** Two implementations of the same function
is waste, and the author may well fix the findings themselves.

Pick this up if, and only if, one of these becomes true:

- #1026 is closed or abandoned.
- #1026 lands but leaves the structural problems below unfixed, in which case the useful
  contribution is the *measurements* plus the specific fixes, not necessarily this whole
  branch.
- The author asks for this approach — offering the measurements and the `idx_search_composite`
  hint finding so they keep authorship is a better outcome than shipping this instead.

## 2. What the branch does

Replaces `matching_resource_ids`' per-parameter unbounded `distinct` with a bounded flow:

- **Partition** the query into positive index predicates (ordinary params plus the
  compartment) and complements (`:missing`, `:not`).
- **Cost-class order** positives by inspecting the *built filter document* — class 2 for an
  unanchored `$regex` (the bare-reference branch), class 1 for `$or`/anchored regex, class 0
  otherwise. This is what keeps a bare-id `patient=` predicate off the driver slot without
  touching `build_reference_filter`.
- **Probe** each positive with `find(filter).projection(resource_id).limit(CHUNK+1)`. It
  decides **exactness only** — a complete id set iff the cursor exhausted inside the bound —
  never selectivity. This is the key difference from #1026, whose probe measures
  distinct-ids-per-row-window and therefore misranks multi-row parameters.
- **Verify** every remaining predicate against the candidate set with a `resource_id`-bounded
  `find`, so an unbounded `distinct` is not expressible at any call site.
- **Stream** only when every positive saturates: one unsorted cursor drained in chunks. No
  sort, no keyset, no `_id` reconstruction — termination is the cursor's own end-of-stream.
- **Single positive, no complements** skips probing entirely: the stream *is* the probe.
- **Cap** accumulation at 100,000 ids (`SearchError::TooManyResults` → 422) so the downstream
  `{id: {$in: …}}` splice in `build_resource_filter` cannot exceed its own 16 MB limit.
- Plus: `search()` computes `matching_resource_ids` once and shares it with the count, and all
  three total gates move to `query.wants_total()`.

## 3. The measurements — the most valuable thing here

Taken with `explain("executionStats")` against the real 11.2 M-resource corpus (827,985
Encounters, `date=ge2016` matching 551,015 distinct ids). **These are reusable regardless of
which implementation ships**, and they are not recorded anywhere else.

| query shape | index chosen | keys examined | docs | ms |
|---|---|---|---|---|
| #1026's keyset page: `sort({_id:1}).limit(512)` | `_id_` | **8,185,346** | 8,185,346 | **235,166** |
| this branch's driver stream: unsorted, projected, limit 2049 | `idx_search_date` | 2,049 | 2,049 | 212 |
| bounded verify, **unhinted** | `idx_search_string` | 1,655,936 | 1,655,936 | 51,471–80,011 |
| bounded verify, **hinted** `idx_search_composite` | `idx_search_composite` | 8,191 | 4,096 | 640–844 |
| `patient=<bare id>` | `idx_search_reference` | 827,986 | 41 | 123,293 |
| `patient=Patient/<id>` (qualified) | `idx_search_reference` | 41 | 41 | 12 |

Three conclusions worth carrying forward:

1. **`sort({_id: 1})` cannot be served.** `_id` is in no `search_index` index, so the planner
   falls to the global `_id_` index and applies the real predicate as a residual — walking
   past Observation rows too. That is #1026's blocking finding, and it is a property of the
   index set, not of any particular code.
2. **The hint is load-bearing, not an optimisation.** The planner does *not* choose
   `idx_search_composite` for the bounded verify; it picks a value index and treats
   `resource_id: {$in: …}` as a residual. ~200× difference in keys examined. Commit
   `6348a30dc` exists solely because the explain contradicted the design's assumption.
3. **The bare-id reference filter is a ~10,000× penalty** and `docsExamined: 41` against
   `keysExamined: 827,986` proves the regex is an *in-index* residual — sequential key reads,
   not random fetches. Tracked as **#1083**, independent of this work.

Also measured, for sizing any future test corpus: the 16 MB `distinct` cap sits at roughly
372,000 ids. `Observation value-quantity=gt150` is **423,382** — only 14 % headroom — so an
Observation subset cannot be sliced below ~88 % without the bug ceasing to reproduce.
`Observation code=8302-2` is 175,355, i.e. **under** the cap, so it is not the parameter that
raises 17217 in matrix row 4.5.

## 4. Verified state

- `cargo fmt --all -- --check` — clean
- `cargo clippy -p helios-persistence -p helios-rest --features mongodb --all-targets` with
  CI's allow-list — clean
- `cargo test -p helios-persistence --features mongodb --test mongodb_tests` — **112 passed,
  0 skipped** (Docker live)
- The pinned param-sort regression test at `mongodb_tests.rs:2639-2684` passes **unedited** —
  this was a design constraint, and is why the unfiltered `None` branch of `param_sorted_ids`
  is deliberately left alone.

Adversarial diff review scored it **8/10, recommend merge**, and rated it better than #1026
on six of seven findings and worse on none.

## 5. Known defects — fix these before it ever ships

1. **Doc comment contradicts the code.** `search_scan_chunk`'s comment (backend.rs, ~146-157)
   says a bounded verification's `$in` "never carries more than this many ids". False on the
   no-positives universe path, which passes up to `max_matched_ids` (100,000). Still safe
   (~4.8 MB), but a reader sizing the `$in` from `chunk` will be misled.
2. **Silent driver tie-break.** When two saturated positives both hit `TIER1_COUNT_LIMIT`
   (100,000) they tie and the driver falls back to (cost-class, query-position) order. Matrix
   row 4.5 is exactly that case, so the choice depends on URL parameter order. Same structural
   residual as #1026's strict-`<`, at a 200× higher threshold — but undocumented. Needs a
   comment and a deliberate tie-break rule.
3. **No multi-batch compartment test.** The compartment predicate is only exercised through
   bounded-verify at the default chunk; there is no small-chunk variant where it saturates and
   is streamed as a driver. This is the same path #1026 left untested — half-covered here.
4. **`drain_field` swallows errors.** `if let Ok(doc) = cursor.deserialize_current()` silently
   drops a row that fails to deserialize, producing a smaller result set rather than an error.
   Termination is unaffected (rows are counted, not ids). Propagate it.
5. **Dead zero-check.** In the ranking loop, `if n == 0 { return Ok(Some(HashSet::new())) }`
   is unreachable — every predicate there is `Probe::Saturated`, so `n >= chunk+1`. Harmless
   but misleading about where the empty-set short circuit lives (the probe loop).

## 6. Follow-ups this work identified, not fixed here

- **#1083** — anchor `build_reference_filter`'s bare-reference regex. Largest remaining
  constant on any Observation-rooted reference query. Independent of this branch.
- **#1084** — append `resource_id` to the value-index tails so probe/verify/driver scans
  become covered. Every plan above shows `docsExamined > 0` because of this. Needs an
  out-of-band index build; `ensure_search_indexes` runs inline at every boot.
- **`storage.rs:3452-3599`** — the `ifNoneExist` probe still uses
  `[$match, $group, $limit, $count]`, which bounds its *output*, not its input. This branch
  fixes the equivalent in `search_impl.rs`; the older copy was left alone per scope. Note that
  **#1026 gets this ordering right**, so whichever lands, `storage.rs` should be brought in
  line.
- **`resolve_revincludes`** had the same unbounded-`distinct` exposure — since fixed on
  `fix/1061-include-path` ([PR #1089](https://github.com/HeliosSoftware/hfs/pull/1089)).
- `_id`/`_lastUpdated` restrictions are not pushed into driver/cost-class selection, because
  `search_index` rows carry no `last_updated`. A broad positive narrowed only by `_id` still
  accumulates the broad param's full id set before the resource-level filter runs.

## 7. Landing notes

- **Rebase before anything.** Base is `fb56e7dea`; `main` has moved. No conflicts as of
  `a5322d9e7`, but re-check.
- **Expect a keep-both conflict with `fix/1062-comma-or`** ([PR #1088](https://github.com/HeliosSoftware/hfs/pull/1088))
  at the end-of-file test-module append: that branch adds `mod value_list_tests`, this one adds
  `mod cost_class_tests`, at the same spot. Both production hunks auto-merge.
- **Docker/testcontainer hygiene** — see **#1090**. Every suite leaks its container on a
  passing run; clean up with
  `docker rm -v $(docker ps -aq --filter ancestor=mongo:5.0.6)` after runs, or the disk fills.
- The 11.2 M corpus lives in the `hfs-mongo` container (`mongo:7.0`, replica set `rs0`). It is
  **not** a testcontainer and must never be caught by an `ancestor=mongo:5.0.6` filter. Warm it
  before measuring; a cold pass proves nothing.

[pr1026]: https://github.com/HeliosSoftware/hfs/pull/1026
