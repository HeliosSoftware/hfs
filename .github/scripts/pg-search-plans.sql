-- Real-data plan capture for the search shapes still slow after the first #224 fix.
--
-- Run against the fully-imported benchmark DB, after VACUUM (ANALYZE).
--
-- WHY THIS RUNS IN CI RATHER THAN LOCALLY: the dominant cost here is *random heap
-- I/O* on a ~10M-row search_index (the first run showed the composite subquery
-- doing 111,881 buffer reads — nearly one random page per row). A local replica
-- small enough to fit in cache cannot reproduce that, and A/B-ing there produces
-- confidently wrong answers: it ranked the covering-index variant LAST when in CI
-- it is the only one that removes the I/O. Measure where the I/O is real.
--
-- HOW TO READ THIS FILE — the metric is TOTAL BUFFERS TOUCHED (hit + read),
-- not `read=` alone and not Execution Time.
--
-- `read=` is NOT immune to cache state, and treating it as such is how run
-- 32740257894's capture was misread: the token lever appeared to be a 5x win
-- (375ms -> 76ms, read=13099 -> read=97) when the two plans were byte-identical,
-- same index, same node counts. All that changed was that the second run found
-- the pages already in shared buffers. `hit + read` barely moved, and `hit+read`
-- is what actually tracks the work done.
--
-- Because every variant below runs sequentially against the same database and
-- warms the cache for the next one, a lever can only be believed when at least
-- one of these is true:
--   1. the PLAN CHANGED — a different index or node type appears; or
--   2. total buffers touched (hit + read) dropped materially; or
--   3. the paired RE-BASELINE control below (same lever dropped, re-measured
--      warm) is still slower than the lever.
-- A time drop with an unchanged plan and unchanged hit+read is cache warming.
-- Report it as such.

\pset pager off
\timing on

\echo '################ CARDINALITY ################'
SELECT param_name, count(*) AS rows, count(DISTINCT resource_id) AS resources
FROM search_index
WHERE tenant_id = 'default' AND resource_type = 'Observation'
  AND param_name IN ('code-value-quantity','combo-code-value-quantity','code')
GROUP BY param_name ORDER BY rows DESC;

SELECT pg_size_pretty(pg_total_relation_size('search_index')) AS search_index_total,
       pg_size_pretty(pg_relation_size('search_index'))       AS heap_only;

-- Which of the ~19 search_index indexes the suites actually used, and what each
-- costs. `idx_scan = 0` after a full import+search run means nothing read it,
-- while every write still maintained it and it still occupied cache. The heap is
-- ~8 GB against ~23 GB of indexes on an 11 GB host, so a dead index is not free:
-- it evicts pages the live ones need.
--
-- Read this BEFORE dropping anything: several indexes look redundant on paper
-- (idx_search_token vs the code-first idx_search_token_code, idx_search_reference
-- vs the text_pattern_ops idx_search_reference_pattern) but only a run that
-- exercised every shape can say so.
\echo ''
\echo '################ INDEX USAGE AND SIZE ################'
SELECT s.indexrelname                                  AS index_name,
       s.idx_scan                                      AS scans,
       s.idx_tup_read                                  AS tuples_read,
       pg_size_pretty(pg_relation_size(s.indexrelid))  AS size
FROM pg_stat_user_indexes s
WHERE s.relname = 'search_index'
ORDER BY s.idx_scan ASC, pg_relation_size(s.indexrelid) DESC;

SELECT pg_size_pretty(sum(pg_relation_size(s.indexrelid))) AS all_indexes
FROM pg_stat_user_indexes s WHERE s.relname = 'search_index';

-- ─────────────────────────────────────────────────────────────────────────────
-- COMPOSITE. Baseline A is what ships today: median 13.2s, 975ms cold single
-- shot, 111,881 buffer READS. Only 222 of 656,737 Observations match, so the
-- outer side is cheap — all the cost is the subquery's random heap access.
-- ─────────────────────────────────────────────────────────────────────────────

\echo ''
\echo '######## A0. SHIPPED SINCE #279: denormalized flat conjunction ########'
-- This is what the query builder emits today. One row per composite instance
-- carries every component's value, so the match is a plain conjunction that
-- idx_search_composite_token_quantity can answer without the grouped
-- aggregate's scattered heap reads. Compare its `hit + read` against section A
-- below, which is the form this replaced.
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT id, version_id, data, last_updated, fhir_version FROM resources
WHERE tenant_id = 'default' AND resource_type = 'Observation' AND is_deleted = FALSE
  AND (id IN (SELECT resource_id FROM search_index
              WHERE tenant_id = 'default' AND resource_type = 'Observation'
                AND param_name = 'code-value-quantity'
                AND composite_group IS NOT NULL
                AND (value_token_code = '8867-4') AND (value_quantity_value > 100)))
ORDER BY last_updated DESC, id ASC LIMIT 21;

\echo ''
\echo '######## A. LEGACY (pre-#279 grouped form) — kept as the comparison baseline ########'
-- NO LONGER SHIPS. Retained so each run measures the old and new forms against
-- the same data on the same host; do not read this as current behaviour.
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT id, version_id, data, last_updated, fhir_version FROM resources
WHERE tenant_id = 'default' AND resource_type = 'Observation' AND is_deleted = FALSE
  AND (id IN (SELECT resource_id FROM search_index
              WHERE tenant_id = 'default' AND resource_type = 'Observation'
                AND param_name = 'code-value-quantity'
                AND ((value_token_code = '8867-4') OR (value_quantity_value > 100))
              GROUP BY resource_id, composite_group
              HAVING MAX(CASE WHEN value_token_code = '8867-4' THEN 1 ELSE 0 END) = 1
                 AND MAX(CASE WHEN value_quantity_value > 100 THEN 1 ELSE 0 END) = 1))
ORDER BY last_updated DESC, id ASC LIMIT 21;

-- The covering index: leading columns are the (tenant, type, param) equality plus
-- the GROUP BY key in order, so the aggregate can stream; INCLUDE carries every
-- value column the HAVING touches, so the scan never visits the heap.
CREATE INDEX IF NOT EXISTS tmp_composite_cover ON search_index
  (tenant_id, resource_type, param_name, resource_id, composite_group)
  INCLUDE (value_token_system, value_token_code, value_quantity_value,
           value_quantity_unit, value_date, value_number)
  WHERE composite_group IS NOT NULL;
VACUUM (ANALYZE) search_index;

\echo ''
\echo '######## B. NO prefilter + covering index (index-only?) ########'
-- The OR-prefilter is what forces the BitmapOr -> Bitmap Heap Scan, i.e. it is
-- what CAUSES the heap I/O. Without it, the whole (tenant,type,param) slice can be
-- read index-only. More rows scanned, but sequentially and with zero heap fetches.
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT id, version_id, data, last_updated, fhir_version FROM resources
WHERE tenant_id = 'default' AND resource_type = 'Observation' AND is_deleted = FALSE
  AND (id IN (SELECT resource_id FROM search_index
              WHERE tenant_id = 'default' AND resource_type = 'Observation'
                AND param_name = 'code-value-quantity'
              GROUP BY resource_id, composite_group
              HAVING MAX(CASE WHEN value_token_code = '8867-4' THEN 1 ELSE 0 END) = 1
                 AND MAX(CASE WHEN value_quantity_value > 100 THEN 1 ELSE 0 END) = 1))
ORDER BY last_updated DESC, id ASC LIMIT 21;

\echo ''
\echo '######## C. as B, forced streaming GroupAggregate (no hash spill) ########'
SET enable_hashagg = off;
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT id, version_id, data, last_updated, fhir_version FROM resources
WHERE tenant_id = 'default' AND resource_type = 'Observation' AND is_deleted = FALSE
  AND (id IN (SELECT resource_id FROM search_index
              WHERE tenant_id = 'default' AND resource_type = 'Observation'
                AND param_name = 'code-value-quantity'
              GROUP BY resource_id, composite_group
              HAVING MAX(CASE WHEN value_token_code = '8867-4' THEN 1 ELSE 0 END) = 1
                 AND MAX(CASE WHEN value_quantity_value > 100 THEN 1 ELSE 0 END) = 1))
ORDER BY last_updated DESC, id ASC LIMIT 21;
RESET enable_hashagg;

\echo ''
\echo '######## D. prefilter + covering index (does the prefilter still win?) ########'
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT id, version_id, data, last_updated, fhir_version FROM resources
WHERE tenant_id = 'default' AND resource_type = 'Observation' AND is_deleted = FALSE
  AND (id IN (SELECT resource_id FROM search_index
              WHERE tenant_id = 'default' AND resource_type = 'Observation'
                AND param_name = 'code-value-quantity'
                AND ((value_token_code = '8867-4') OR (value_quantity_value > 100))
              GROUP BY resource_id, composite_group
              HAVING MAX(CASE WHEN value_token_code = '8867-4' THEN 1 ELSE 0 END) = 1
                 AND MAX(CASE WHEN value_quantity_value > 100 THEN 1 ELSE 0 END) = 1))
ORDER BY last_updated DESC, id ASC LIMIT 21;

\echo ''
\echo '######## E. combo-code-value-quantity (2.46M rows) with covering index ########'
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT id, version_id, data, last_updated, fhir_version FROM resources
WHERE tenant_id = 'default' AND resource_type = 'Observation' AND is_deleted = FALSE
  AND (id IN (SELECT resource_id FROM search_index
              WHERE tenant_id = 'default' AND resource_type = 'Observation'
                AND param_name = 'combo-code-value-quantity'
              GROUP BY resource_id, composite_group
              HAVING MAX(CASE WHEN value_token_system = 'http://loinc.org'
                              AND value_token_code = '8480-6' THEN 1 ELSE 0 END) = 1
                 AND MAX(CASE WHEN value_quantity_value > 140 THEN 1 ELSE 0 END) = 1))
ORDER BY last_updated DESC, id ASC LIMIT 21;

DROP INDEX IF EXISTS tmp_composite_cover;

-- ─────────────────────────────────────────────────────────────────────────────
-- TOKEN. Encounter?status=finished matches ALL 65,659 Encounters but Postgres
-- estimates the token scan at rows=1832 — a 36x UNDER-estimate — so it
-- materialises 65k ids, does 65k pkey heap fetches, sorts, and returns 21
-- (2,746ms). With a correct estimate it should instead walk idx_resources_search
-- in last_updated order and stop after ~21 rows.
--
-- The v14 statistics are on (param_name, value_token_code), but the query also
-- binds resource_type, and the three are near-perfectly correlated — so Postgres
-- still multiplies independent marginals. Test a 3-column MCV.
-- ─────────────────────────────────────────────────────────────────────────────

\echo ''
\echo '######## F. TOKEN status=finished — BEFORE 3-col stats (baseline 2746ms) ########'
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT id, version_id, data, last_updated, fhir_version FROM resources
WHERE tenant_id = 'default' AND resource_type = 'Encounter' AND is_deleted = FALSE
  AND (id IN (SELECT resource_id FROM search_index
              WHERE tenant_id = 'default' AND resource_type = 'Encounter'
                AND param_name = 'status' AND (value_token_code = 'finished')))
ORDER BY last_updated DESC, id ASC LIMIT 21;

CREATE STATISTICS IF NOT EXISTS tmp_stx_type_param_code (mcv, dependencies)
  ON resource_type, param_name, value_token_code FROM search_index;
ALTER TABLE search_index ALTER COLUMN value_token_code SET STATISTICS 2000;
ANALYZE search_index;

\echo ''
\echo '######## G. TOKEN status=finished — AFTER 3-col MCV (estimate fixed?) ########'
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT id, version_id, data, last_updated, fhir_version FROM resources
WHERE tenant_id = 'default' AND resource_type = 'Encounter' AND is_deleted = FALSE
  AND (id IN (SELECT resource_id FROM search_index
              WHERE tenant_id = 'default' AND resource_type = 'Encounter'
                AND param_name = 'status' AND (value_token_code = 'finished')))
ORDER BY last_updated DESC, id ASC LIMIT 21;

\echo ''
\echo '######## H. TOKEN status=missing-status — AFTER (must stay fast: was 0.8ms) ########'
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT id, version_id, data, last_updated, fhir_version FROM resources
WHERE tenant_id = 'default' AND resource_type = 'Encounter' AND is_deleted = FALSE
  AND (id IN (SELECT resource_id FROM search_index
              WHERE tenant_id = 'default' AND resource_type = 'Encounter'
                AND param_name = 'status' AND (value_token_code = 'missing-status')))
ORDER BY last_updated DESC, id ASC LIMIT 21;

\echo ''
\echo '######## I. Observation?category=laboratory — AFTER (high-match control) ########'
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT id, version_id, data, last_updated, fhir_version FROM resources
WHERE tenant_id = 'default' AND resource_type = 'Observation' AND is_deleted = FALSE
  AND (id IN (SELECT resource_id FROM search_index
              WHERE tenant_id = 'default' AND resource_type = 'Observation'
                AND param_name = 'category' AND (value_token_code = 'laboratory')))
ORDER BY last_updated DESC, id ASC LIMIT 21;

DROP STATISTICS IF EXISTS tmp_stx_type_param_code;

\echo ''
\echo '######## J. TOKEN Observation?code=NOT-A-LOINC — BASELINE (sparse) ########'
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT id, version_id, data, last_updated, fhir_version FROM resources
WHERE tenant_id = 'default' AND resource_type = 'Observation' AND is_deleted = FALSE
  AND (id IN (SELECT resource_id FROM search_index
              WHERE tenant_id = 'default' AND resource_type = 'Observation'
                AND param_name = 'code' AND (value_token_code = 'NOT-A-LOINC')))
ORDER BY last_updated DESC, id ASC LIMIT 21;

\echo ''
\echo '######## K. TOKEN Observation?code=8302-2 — BASELINE (common control) ########'
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT id, version_id, data, last_updated, fhir_version FROM resources
WHERE tenant_id = 'default' AND resource_type = 'Observation' AND is_deleted = FALSE
  AND (id IN (SELECT resource_id FROM search_index
              WHERE tenant_id = 'default' AND resource_type = 'Observation'
                AND param_name = 'code' AND (value_token_code = '8302-2')))
ORDER BY last_updated DESC, id ASC LIMIT 21;

\echo ''
\echo '######## L. DATE Encounter?date=gt2200-01-01 — BASELINE (sparse) ########'
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT id, version_id, data, last_updated, fhir_version FROM resources
WHERE tenant_id = 'default' AND resource_type = 'Encounter' AND is_deleted = FALSE
  AND (id IN (SELECT resource_id FROM search_index
              WHERE tenant_id = 'default' AND resource_type = 'Encounter'
                AND param_name = 'date' AND value_date >= '2200-01-01'))
ORDER BY last_updated DESC, id ASC LIMIT 21;

\echo ''
\echo '######## M. DATE Encounter?date=gt2010-01-01 — BASELINE (common control) ########'
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT id, version_id, data, last_updated, fhir_version FROM resources
WHERE tenant_id = 'default' AND resource_type = 'Encounter' AND is_deleted = FALSE
  AND (id IN (SELECT resource_id FROM search_index
              WHERE tenant_id = 'default' AND resource_type = 'Encounter'
                AND param_name = 'date' AND value_date >= '2010-01-01'))
ORDER BY last_updated DESC, id ASC LIMIT 21;

CREATE INDEX IF NOT EXISTS tmp_search_token_code_cover ON search_index
  (tenant_id, resource_type, param_name, value_token_code, value_token_system)
  INCLUDE (resource_id)
  WHERE value_token_code IS NOT NULL;
CREATE INDEX IF NOT EXISTS tmp_search_date_cover ON search_index
  (tenant_id, resource_type, param_name, value_date)
  INCLUDE (resource_id)
  WHERE value_date IS NOT NULL;
VACUUM (ANALYZE) search_index;

\echo ''
\echo '######## N. TOKEN Observation?code=NOT-A-LOINC — AFTER lever 1 ########'
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT id, version_id, data, last_updated, fhir_version FROM resources
WHERE tenant_id = 'default' AND resource_type = 'Observation' AND is_deleted = FALSE
  AND (id IN (SELECT resource_id FROM search_index
              WHERE tenant_id = 'default' AND resource_type = 'Observation'
                AND param_name = 'code' AND (value_token_code = 'NOT-A-LOINC')))
ORDER BY last_updated DESC, id ASC LIMIT 21;

\echo ''
\echo '######## O. TOKEN Observation?code=8302-2 — AFTER lever 1 (must stay fast) ########'
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT id, version_id, data, last_updated, fhir_version FROM resources
WHERE tenant_id = 'default' AND resource_type = 'Observation' AND is_deleted = FALSE
  AND (id IN (SELECT resource_id FROM search_index
              WHERE tenant_id = 'default' AND resource_type = 'Observation'
                AND param_name = 'code' AND (value_token_code = '8302-2')))
ORDER BY last_updated DESC, id ASC LIMIT 21;

\echo ''
\echo '######## P. DATE Encounter?date=gt2200-01-01 — AFTER lever 1 ########'
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT id, version_id, data, last_updated, fhir_version FROM resources
WHERE tenant_id = 'default' AND resource_type = 'Encounter' AND is_deleted = FALSE
  AND (id IN (SELECT resource_id FROM search_index
              WHERE tenant_id = 'default' AND resource_type = 'Encounter'
                AND param_name = 'date' AND value_date >= '2200-01-01'))
ORDER BY last_updated DESC, id ASC LIMIT 21;

\echo ''
\echo '######## Q. DATE Encounter?date=gt2010-01-01 — AFTER lever 1 (must stay fast) ########'
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT id, version_id, data, last_updated, fhir_version FROM resources
WHERE tenant_id = 'default' AND resource_type = 'Encounter' AND is_deleted = FALSE
  AND (id IN (SELECT resource_id FROM search_index
              WHERE tenant_id = 'default' AND resource_type = 'Encounter'
                AND param_name = 'date' AND value_date >= '2010-01-01'))
ORDER BY last_updated DESC, id ASC LIMIT 21;

-- ─────────────────────────────────────────────────────────────────────────────
-- RE-BASELINE CONTROLS. K/M measured the baseline COLD; O/Q measured the lever
-- with those same pages already in shared buffers, so the pair cannot separate
-- "the index helped" from "the cache was warm". Dropping the lever and
-- re-measuring the identical baseline query — now warm — gives the honest
-- comparison: R vs O and S vs Q. If R ≈ O (or S ≈ Q) the lever did nothing and
-- the apparent win was cache warming.
-- ─────────────────────────────────────────────────────────────────────────────
DROP INDEX IF EXISTS tmp_search_token_code_cover;
DROP INDEX IF EXISTS tmp_search_date_cover;

\echo ''
\echo '######## R. TOKEN Observation?code=8302-2 — BASELINE RE-RUN, WARM (control for O) ########'
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT id, version_id, data, last_updated, fhir_version FROM resources
WHERE tenant_id = 'default' AND resource_type = 'Observation' AND is_deleted = FALSE
  AND (id IN (SELECT resource_id FROM search_index
              WHERE tenant_id = 'default' AND resource_type = 'Observation'
                AND param_name = 'code' AND (value_token_code = '8302-2')))
ORDER BY last_updated DESC, id ASC LIMIT 21;

\echo ''
\echo '######## S. DATE Encounter?date=gt2010-01-01 — BASELINE RE-RUN, WARM (control for Q) ########'
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT id, version_id, data, last_updated, fhir_version FROM resources
WHERE tenant_id = 'default' AND resource_type = 'Encounter' AND is_deleted = FALSE
  AND (id IN (SELECT resource_id FROM search_index
              WHERE tenant_id = 'default' AND resource_type = 'Encounter'
                AND param_name = 'date' AND value_date >= '2010-01-01'))
ORDER BY last_updated DESC, id ASC LIMIT 21;

-- The composite covering index (section A-E) gets the same treatment: E measured
-- it warm against A's cold baseline. T re-measures the shipped query with the
-- covering index gone.
DROP INDEX IF EXISTS tmp_composite_cover;

\echo ''
\echo '######## T. COMPOSITE code-value-quantity — LEGACY GROUPED RE-RUN, WARM (control for D) ########'
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT id, version_id, data, last_updated, fhir_version FROM resources
WHERE tenant_id = 'default' AND resource_type = 'Observation' AND is_deleted = FALSE
  AND (id IN (SELECT resource_id FROM search_index
              WHERE tenant_id = 'default' AND resource_type = 'Observation'
                AND param_name = 'code-value-quantity'
                AND ((value_token_code = '8867-4') OR (value_quantity_value > 100))
              GROUP BY resource_id, composite_group
              HAVING MAX(CASE WHEN value_token_code = '8867-4' THEN 1 ELSE 0 END) = 1
                 AND MAX(CASE WHEN value_quantity_value > 100 THEN 1 ELSE 0 END) = 1))
ORDER BY last_updated DESC, id ASC LIMIT 21;

-- ────────────────────────────────────────────────────────────────────────────
-- U-X. THE SQL THE SERVER ACTUALLY EMITS.
--
-- Sections A-T measure hand-written levers. None of them is the fast-path query
-- the #279/v17 path builds, so when v17's targeted shapes stayed slow and its
-- untargeted ones regressed 12x, this capture could not say why — the plan for
-- the emitted SQL had never been recorded. These four sections close that gap:
-- the exact shape from `search_impl.rs`, over both selectivity regimes.
--
-- What to look for: `idx_search_*_recent` (v19) driving an Index Only Scan with
-- rows≈22 and a streaming `Unique`, meaning the LIMIT stopped the scan. A Sort
-- node, or `rows` in the tens of thousands, means early termination did NOT
-- happen and the recent-first index was not chosen.
-- ────────────────────────────────────────────────────────────────────────────

\echo ''
\echo '######## U. FAST PATH date Observation?date=gt2010 — non-selective (early termination expected) ########'
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT r.id, r.version_id, r.data, r.last_updated, r.fhir_version,
       r.last_updated AS sort_key
FROM ( SELECT DISTINCT resource_id, last_updated FROM search_index
       WHERE tenant_id = 'default' AND resource_type = 'Observation'
         AND param_name = 'date' AND value_date >= '2010-01-01'
       ORDER BY last_updated DESC, resource_id ASC LIMIT 22 ) c
JOIN resources r ON r.tenant_id = 'default' AND r.resource_type = 'Observation'
                AND r.id = c.resource_id
WHERE r.is_deleted = FALSE
ORDER BY c.last_updated DESC, c.resource_id ASC;

\echo ''
\echo '######## V. FAST PATH date Observation?date=gt2200 — sparse (value-first index must still win) ########'
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT r.id, r.version_id, r.data, r.last_updated, r.fhir_version,
       r.last_updated AS sort_key
FROM ( SELECT DISTINCT resource_id, last_updated FROM search_index
       WHERE tenant_id = 'default' AND resource_type = 'Observation'
         AND param_name = 'date' AND value_date >= '2200-01-01'
       ORDER BY last_updated DESC, resource_id ASC LIMIT 22 ) c
JOIN resources r ON r.tenant_id = 'default' AND r.resource_type = 'Observation'
                AND r.id = c.resource_id
WHERE r.is_deleted = FALSE
ORDER BY c.last_updated DESC, c.resource_id ASC;

\echo ''
\echo '######## W. FAST PATH token Observation?category=laboratory — the 4162ms shape ########'
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT r.id, r.version_id, r.data, r.last_updated, r.fhir_version,
       r.last_updated AS sort_key
FROM ( SELECT DISTINCT resource_id, last_updated FROM search_index
       WHERE tenant_id = 'default' AND resource_type = 'Observation'
         AND param_name = 'category' AND value_token_code = 'laboratory'
       ORDER BY last_updated DESC, resource_id ASC LIMIT 22 ) c
JOIN resources r ON r.tenant_id = 'default' AND r.resource_type = 'Observation'
                AND r.id = c.resource_id
WHERE r.is_deleted = FALSE
ORDER BY c.last_updated DESC, c.resource_id ASC;

\echo ''
\echo '######## X. FAST PATH token Encounter?class=AMB — the shape v17 regressed 12x ########'
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT r.id, r.version_id, r.data, r.last_updated, r.fhir_version,
       r.last_updated AS sort_key
FROM ( SELECT DISTINCT resource_id, last_updated FROM search_index
       WHERE tenant_id = 'default' AND resource_type = 'Encounter'
         AND param_name = 'class' AND value_token_code = 'AMB'
       ORDER BY last_updated DESC, resource_id ASC LIMIT 22 ) c
JOIN resources r ON r.tenant_id = 'default' AND r.resource_type = 'Encounter'
                AND r.id = c.resource_id
WHERE r.is_deleted = FALSE
ORDER BY c.last_updated DESC, c.resource_id ASC;
