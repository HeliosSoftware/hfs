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

-- Full row census. The three-parameter query above was chosen to investigate the
-- #279 composite work and says nothing about where the table's rows actually
-- come from — which is what the import path pays for, one index insert per row
-- per applicable index. Import is the worst remaining gap, so this reports the
-- whole distribution: rows per (resource_type, param_name), rows per resource,
-- and the total. Anything with a high rows/resource ratio is a candidate for
-- writing fewer rows rather than for indexing them faster.
\echo ''
\echo '######## ROW CENSUS — top 30 (resource_type, param_name) by rows ########'
SELECT resource_type, param_name, count(*) AS rows,
       count(DISTINCT resource_id) AS resources,
       round(count(*)::numeric / NULLIF(count(DISTINCT resource_id), 0), 2) AS rows_per_resource
FROM search_index WHERE tenant_id = 'default'
GROUP BY resource_type, param_name
ORDER BY rows DESC LIMIT 30;

\echo ''
\echo '######## ROW CENSUS — totals ########'
SELECT count(*) AS index_rows,
       count(DISTINCT (resource_type, resource_id)) AS resources,
       round(count(*)::numeric / NULLIF(count(DISTINCT (resource_type, resource_id)), 0), 1)
         AS rows_per_resource
FROM search_index WHERE tenant_id = 'default';

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

\echo ''
\echo '######## Y. FAST PATH token Observation?category=laboratory,vital-signs — MULTI-VALUE ########'
-- The shape v20 regressed 50x and no earlier section modelled: a comma list is
-- an OR over equality tests. Look for idx_search_token_code_recent driving ONE
-- ordered Index Only Scan with rows=22 and a streaming Unique. A BitmapOr, or a
-- Sort over tens of thousands of rows, means the merge could not stay ordered.
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT r.id, r.version_id, r.data, r.last_updated, r.fhir_version,
       r.last_updated AS sort_key
FROM ( SELECT DISTINCT resource_id, last_updated FROM search_index
       WHERE tenant_id = 'default' AND resource_type = 'Observation'
         AND param_name = 'category'
         AND value_token_code IN ('laboratory', 'vital-signs')
       ORDER BY last_updated DESC, resource_id ASC LIMIT 22 ) c
JOIN resources r ON r.tenant_id = 'default' AND r.resource_type = 'Observation'
                AND r.id = c.resource_id
WHERE r.is_deleted = FALSE
ORDER BY c.last_updated DESC, c.resource_id ASC;

\echo ''
\echo '######## Z. FAST PATH token Observation?code=8302-2,29463-7 — MULTI-VALUE, SELECTIVE ########'
-- The other regime: a comma list whose members are each rare. Value-first may
-- legitimately win here even though it must sort, because the match set is tiny.
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT r.id, r.version_id, r.data, r.last_updated, r.fhir_version,
       r.last_updated AS sort_key
FROM ( SELECT DISTINCT resource_id, last_updated FROM search_index
       WHERE tenant_id = 'default' AND resource_type = 'Observation'
         AND param_name = 'code'
         AND value_token_code IN ('8302-2', '29463-7')
       ORDER BY last_updated DESC, resource_id ASC LIMIT 22 ) c
JOIN resources r ON r.tenant_id = 'default' AND r.resource_type = 'Observation'
                AND r.id = c.resource_id
WHERE r.is_deleted = FALSE
ORDER BY c.last_updated DESC, c.resource_id ASC;

-- ─────────────────────────────────────────────────────────────────────────────
-- SCHEMA v24 — the `system|code` token form and the folded-string prefix.
--
-- Sections W-Z above model only BARE token codes. `k6/searchConfig.js` also
-- sends system-qualified values, and only those build
-- `value_token_system = $n AND value_token_code = $m` — the one predicate shape
-- in the benchmark that is strict in `value_token_system` and can therefore
-- reach the partial `idx_search_token`. Run 33029355759 measured that form as
-- the single most expensive statement of the whole search suite (314.5 s over
-- 6254 calls, 50.286 ms mean) and no section here modelled it. That omission is
-- the same class of mistake as v20's: a capture that models only one value form
-- ranks an index that the other form is what actually hurts on.
-- ─────────────────────────────────────────────────────────────────────────────

\echo ''
\echo '######## AA. FAST PATH token Encounter?class=v3-ActCode|AMB — the 462ms p99 shape ########'
-- WHAT TO LOOK FOR, AS OF v30: `Index Only Scan using idx_search_token_code`
-- with `Filter: (value_token_system = ...)`, `Heap Fetches: 0`, rows=22 out of
-- the Limit, and NO Sort node.
--
-- v24 built `idx_search_token` for this shape and got it index-only and ordered.
-- v30 dropped that index (2,283 MB) because v20's code-first
-- `idx_search_token_code` reaches the identical plan: it seeks the code, its
-- remaining key columns ARE `last_updated DESC, resource_id ASC` so the LIMIT
-- still stops at 22, and `value_token_system` is in its INCLUDE payload so the
-- system is filtered without a heap fetch. On a local replica the two plans read
-- the same 6 buffers for the same 22 rows. AB below is the paired counterfactual
-- that measures the claim here, on real data.
--
-- A `Sort`, a `Bitmap Heap Scan`, or a non-zero `Heap Fetches` means the v30
-- reasoning is wrong and this shape is materialising the whole match set again.
-- `Rows Removed by Filter` far above `rows` means the requested (system, code)
-- pair barely co-occurs and the scan is walking the code slice — the bounded
-- worst case v30's docstring states.
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT r.id, r.version_id, r.data, r.last_updated, r.fhir_version,
       r.last_updated AS sort_key
FROM ( SELECT DISTINCT resource_id, last_updated FROM search_index
       WHERE tenant_id = 'default' AND resource_type = 'Encounter'
         AND param_name = 'class'
         AND (value_token_system = 'http://terminology.hl7.org/CodeSystem/v3-ActCode'
              AND value_token_code = 'AMB')
       ORDER BY last_updated DESC, resource_id ASC LIMIT 22 ) c
JOIN resources r ON r.tenant_id = 'default' AND r.resource_type = 'Encounter'
                AND r.id = c.resource_id
WHERE r.is_deleted = FALSE
ORDER BY c.last_updated DESC, c.resource_id ASC;

\echo ''
\echo '######## AB. AA COUNTERFACTUAL — the same query with v24 idx_search_token back ########'
-- The paired counterfactual for AA, in the same pass and against the same cache,
-- so the comparison is not a cross-run one (see the header: only a changed plan
-- or a changed hit+read counts).
--
-- v30 dropped `idx_search_token`. This rebuilds exactly the v24 definition
-- inside a transaction that ROLLBACKs, so the run measures what those 2,283 MB
-- bought for the `system|code` shape rather than arguing about it. DDL is
-- transactional in Postgres, so the database is left exactly as it was; the
-- build is partial on `value_token_system IS NOT NULL` and runs before the k6
-- load starts. If any statement in this block fails, psql's session ends the
-- transaction and nothing is left behind.
--
-- EXPECTED, if v30's reasoning is right: the SAME node type as AA (Index Only
-- Scan), rows=22, Heap Fetches 0, and hit+read within noise of AA — with the
-- only difference being that the system moves from `Filter` into the
-- `Index Cond`. If instead AB is materially cheaper than AA, the drop was wrong
-- and the index has to come back; the per-shape number to weigh it against is
-- the ~2,170 MB of cache AA's index set no longer occupies.
BEGIN;
-- Bound the blast radius: if anything else holds a lock on search_index we abort
-- rather than hang the whole capture, and the ROLLBACK below still restores the
-- v30 index set.
SET LOCAL lock_timeout = '30s';
SET LOCAL statement_timeout = '300s';
CREATE INDEX idx_search_token_v24
  ON search_index (tenant_id, resource_type, param_name, value_token_system,
                   value_token_code, last_updated DESC, resource_id ASC)
  WHERE value_token_system IS NOT NULL;
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT r.id, r.version_id, r.data, r.last_updated, r.fhir_version,
       r.last_updated AS sort_key
FROM ( SELECT DISTINCT resource_id, last_updated FROM search_index
       WHERE tenant_id = 'default' AND resource_type = 'Encounter'
         AND param_name = 'class'
         AND (value_token_system = 'http://terminology.hl7.org/CodeSystem/v3-ActCode'
              AND value_token_code = 'AMB')
       ORDER BY last_updated DESC, resource_id ASC LIMIT 22 ) c
JOIN resources r ON r.tenant_id = 'default' AND r.resource_type = 'Encounter'
                AND r.id = c.resource_id
WHERE r.is_deleted = FALSE
ORDER BY c.last_updated DESC, c.resource_id ASC;
ROLLBACK;

\echo ''
\echo '######## AC. v30 token index set (must print idx_search_token_code, _code_recent, _system) ########'
SELECT indexname, indexdef FROM pg_indexes
WHERE tablename = 'search_index' AND indexname LIKE 'idx_search_token%'
ORDER BY indexname;

\echo ''
\echo '######## AD. FAST PATH token Encounter?class=missing|class-code — zero-match system|code ########'
-- The sibling value: same predicate shape, no matching row. Under the v23 index
-- it still had to walk the parameter slice; under v24 the equality seek finds
-- nothing and returns immediately. Expect rows=0 and single-digit buffers.
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT r.id, r.version_id, r.data, r.last_updated, r.fhir_version,
       r.last_updated AS sort_key
FROM ( SELECT DISTINCT resource_id, last_updated FROM search_index
       WHERE tenant_id = 'default' AND resource_type = 'Encounter'
         AND param_name = 'class'
         AND (value_token_system = 'missing' AND value_token_code = 'class-code')
       ORDER BY last_updated DESC, resource_id ASC LIMIT 22 ) c
JOIN resources r ON r.tenant_id = 'default' AND r.resource_type = 'Encounter'
                AND r.id = c.resource_id
WHERE r.is_deleted = FALSE
ORDER BY c.last_updated DESC, c.resource_id ASC;

\echo ''
\echo '######## AE. FAST PATH token Observation?category=<system>|laboratory — MUST STAY FAST ########'
-- The control. This one was already fast (the `category` slice is 689,080 rows,
-- 11x the `class` slice, which pushed the planner off idx_search_token and onto
-- idx_search_token_code_recent). v24 makes idx_search_token cheap enough that
-- the planner may now prefer it here too — which is fine only if the plan stays
-- index-only with rows=22. A regression here would cancel the AA win.
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT r.id, r.version_id, r.data, r.last_updated, r.fhir_version,
       r.last_updated AS sort_key
FROM ( SELECT DISTINCT resource_id, last_updated FROM search_index
       WHERE tenant_id = 'default' AND resource_type = 'Observation'
         AND param_name = 'category'
         AND (value_token_system = 'http://terminology.hl7.org/CodeSystem/observation-category'
              AND value_token_code = 'laboratory')
       ORDER BY last_updated DESC, resource_id ASC LIMIT 22 ) c
JOIN resources r ON r.tenant_id = 'default' AND r.resource_type = 'Observation'
                AND r.id = c.resource_id
WHERE r.is_deleted = FALSE
ORDER BY c.last_updated DESC, c.resource_id ASC;

\echo ''
\echo '######## AF. FAST PATH token Observation?code=http://loinc.org|8302-2 — selective system|code ########'
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT r.id, r.version_id, r.data, r.last_updated, r.fhir_version,
       r.last_updated AS sort_key
FROM ( SELECT DISTINCT resource_id, last_updated FROM search_index
       WHERE tenant_id = 'default' AND resource_type = 'Observation'
         AND param_name = 'code'
         AND (value_token_system = 'http://loinc.org' AND value_token_code = '8302-2')
       ORDER BY last_updated DESC, resource_id ASC LIMIT 22 ) c
JOIN resources r ON r.tenant_id = 'default' AND r.resource_type = 'Observation'
                AND r.id = c.resource_id
WHERE r.is_deleted = FALSE
ORDER BY c.last_updated DESC, c.resource_id ASC;

\echo ''
-- ─────────────────────────────────────────────────────────────────────────────
-- SCHEMA v30 — the `system|` form, which nothing above ever modelled.
--
-- `build_token_condition` emits four predicate shapes and only ONE is strict in
-- `value_token_system` alone: the `system|` spelling, `value_token_system = $n`.
-- Every section above sends `system|code`, which is strict in both columns. So
-- the whole capture measured the shape `idx_search_token` was *chosen* for and
-- never the shape it was the only candidate for — the same omission the v24
-- header calls out about v20, one level down.
--
-- v30 replaced `idx_search_token` (2,283 MB, 62% of the token family together
-- with its two siblings) with a seek-only, deduplicating
-- `idx_search_token_system (tenant_id, resource_type, param_name,
-- value_token_system)` and made `build_token_condition` emit
-- `value_token_code IS NOT NULL` alongside the system equality, which brings the
-- recent-first `idx_search_token_code_recent` in as a second candidate.
--
-- The two regimes to look for, and which index should win each:
--   broad system  (matches much of the parameter slice) -> idx_search_token_code_recent,
--                  streaming `last_updated DESC`, LIMIT stops it, ~22 rows out
--   narrow system (matches little)                      -> idx_search_token_system,
--                  exact seek, heap fetch per match, small Sort
-- Either is acceptable. What is NOT is a Sort over tens of thousands of rows,
-- which is what `idx_search_token` itself did here: its key is
-- (…, value_token_system, value_token_code, last_updated, resource_id), and a
-- `system|` predicate does not bind `value_token_code`, so the sort key was
-- never reachable for this shape. On a local replica that cost 1,074 buffers and
-- a 66,667-row top-N sort to return 22.
-- ─────────────────────────────────────────────────────────────────────────────

\echo ''
\echo '######## AF1. FAST PATH token Encounter?class=<v3-ActCode>| — BROAD system| ########'
-- The system that most Encounter.class rows carry. Expect
-- idx_search_token_code_recent, no Sort, rows=22, double-digit buffers.
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT r.id, r.version_id, r.data, r.last_updated, r.fhir_version,
       r.last_updated AS sort_key
FROM ( SELECT DISTINCT resource_id, last_updated FROM search_index
       WHERE tenant_id = 'default' AND resource_type = 'Encounter'
         AND param_name = 'class'
         AND (value_token_code IS NOT NULL
              AND value_token_system = 'http://terminology.hl7.org/CodeSystem/v3-ActCode')
       ORDER BY last_updated DESC, resource_id ASC LIMIT 22 ) c
JOIN resources r ON r.tenant_id = 'default' AND r.resource_type = 'Encounter'
                AND r.id = c.resource_id
WHERE r.is_deleted = FALSE
ORDER BY c.last_updated DESC, c.resource_id ASC;

\echo ''
\echo '######## AF2. AF1 COUNTERFACTUAL — pre-v30 form, no IS NOT NULL conjunct ########'
-- The identical query without the conjunct v30 added, so the only candidate is
-- the seek-only index (`idx_search_token_code_recent` is partial on
-- `value_token_code IS NOT NULL`, which this form cannot prove). Expect a seek
-- followed by a heap fetch per matching row and a Sort — i.e. materially MORE
-- hit+read than AF1. If AF2 is not worse, the conjunct is buying nothing and can
-- be reverted.
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT r.id, r.version_id, r.data, r.last_updated, r.fhir_version,
       r.last_updated AS sort_key
FROM ( SELECT DISTINCT resource_id, last_updated FROM search_index
       WHERE tenant_id = 'default' AND resource_type = 'Encounter'
         AND param_name = 'class'
         AND (value_token_system = 'http://terminology.hl7.org/CodeSystem/v3-ActCode')
       ORDER BY last_updated DESC, resource_id ASC LIMIT 22 ) c
JOIN resources r ON r.tenant_id = 'default' AND r.resource_type = 'Encounter'
                AND r.id = c.resource_id
WHERE r.is_deleted = FALSE
ORDER BY c.last_updated DESC, c.resource_id ASC;

\echo ''
\echo '######## AF3. FAST PATH token Encounter?class=missing| — NARROW/zero-match system| ########'
-- The other regime. Expect idx_search_token_system, an exact seek, rows=0 and
-- single-digit buffers. This is the shape that justifies keeping a system-keyed
-- index at all instead of dropping one outright.
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT r.id, r.version_id, r.data, r.last_updated, r.fhir_version,
       r.last_updated AS sort_key
FROM ( SELECT DISTINCT resource_id, last_updated FROM search_index
       WHERE tenant_id = 'default' AND resource_type = 'Encounter'
         AND param_name = 'class'
         AND (value_token_code IS NOT NULL AND value_token_system = 'missing')
       ORDER BY last_updated DESC, resource_id ASC LIMIT 22 ) c
JOIN resources r ON r.tenant_id = 'default' AND r.resource_type = 'Encounter'
                AND r.id = c.resource_id
WHERE r.is_deleted = FALSE
ORDER BY c.last_updated DESC, c.resource_id ASC;

\echo ''
\echo '######## AF4. v30 TOKEN FAMILY FOOTPRINT — the number the whole change is about ########'
-- The token family was 7,474 MB of an 11.9 GB index set on an 11 GB host, and
-- `idx_search_token` was 2,283 MB of it. The replacement carries the same rows
-- with no payload and no per-row key column, so btree deduplication collapses
-- each (tenant, type, param, system) group to one key plus a posting list. A
-- local replica put the ratio at 4.9%; this is the real number.
SELECT s.indexrelname,
       pg_size_pretty(pg_relation_size(s.indexrelid)) AS size,
       s.idx_scan, s.idx_tup_read
FROM pg_stat_user_indexes s
WHERE s.relname = 'search_index'
  AND s.indexrelname LIKE 'idx_search_token%'
ORDER BY pg_relation_size(s.indexrelid) DESC;

SELECT count(*) FILTER (WHERE value_token_code IS NOT NULL)   AS rows_with_a_code,
       count(*) FILTER (WHERE value_token_system IS NOT NULL) AS rows_with_a_system,
       count(DISTINCT (resource_type, param_name, value_token_system))
         FILTER (WHERE value_token_system IS NOT NULL)        AS distinct_system_groups
FROM search_index;

\echo ''
\echo '######## AG. STRING Patient?name=Emilia — v24 form (implies the partial index predicate) ########'
-- `idx_search_string_folded_pattern` is partial on `value_string IS NOT NULL`
-- and had **0 scans** in run 33029355759, because `COALESCE(a, b) LIKE …` does
-- not imply that predicate (COALESCE is not strict). The whole load fell to
-- `idx_search_string_folded`, which is keyed on the bare column and so can only
-- supply the (tenant, type, param) prefix before filtering: 38,694 scans,
-- 142,058,567 tuples, 3,671 tuples read per query.
--
-- WHAT TO LOOK FOR: `Index Only Scan using idx_search_string_folded_pattern`
-- with an Index Cond containing `~>=~`/`~<~` (the text_pattern_ops range LIKE
-- rewrites into) rather than a Filter, and `Heap Fetches: 0` from the v24
-- INCLUDE (resource_id, last_updated).
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT r.id, r.version_id, r.data, r.last_updated, r.fhir_version,
       r.last_updated AS sort_key
FROM ( SELECT DISTINCT resource_id, last_updated FROM search_index
       WHERE tenant_id = 'default' AND resource_type = 'Patient'
         AND param_name = 'name' AND value_string IS NOT NULL
         AND COALESCE(value_string_folded, lower(value_string)) LIKE 'emilia%' ESCAPE '\'
       ORDER BY last_updated DESC, resource_id ASC LIMIT 22 ) c
JOIN resources r ON r.tenant_id = 'default' AND r.resource_type = 'Patient'
                AND r.id = c.resource_id
WHERE r.is_deleted = FALSE
ORDER BY c.last_updated DESC, c.resource_id ASC;

\echo ''
\echo '######## AH. AG COUNTERFACTUAL — pre-v24 form, no IS NOT NULL conjunct ########'
-- Identical query minus the one conjunct. Same pass, same cache, so this is a
-- true paired control: if AG and AH produce the same plan then the conjunct did
-- not unlock the pattern index and the string half of v24 is a no-op (it is
-- still not a regression — idx_search_string_folded is deliberately kept).
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT r.id, r.version_id, r.data, r.last_updated, r.fhir_version,
       r.last_updated AS sort_key
FROM ( SELECT DISTINCT resource_id, last_updated FROM search_index
       WHERE tenant_id = 'default' AND resource_type = 'Patient'
         AND param_name = 'name'
         AND COALESCE(value_string_folded, lower(value_string)) LIKE 'emilia%' ESCAPE '\'
       ORDER BY last_updated DESC, resource_id ASC LIMIT 22 ) c
JOIN resources r ON r.tenant_id = 'default' AND r.resource_type = 'Patient'
                AND r.id = c.resource_id
WHERE r.is_deleted = FALSE
ORDER BY c.last_updated DESC, c.resource_id ASC;

\echo ''
\echo '######## AI. STRING Patient?address=Springfield — v24 form, larger slice ########'
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT r.id, r.version_id, r.data, r.last_updated, r.fhir_version,
       r.last_updated AS sort_key
FROM ( SELECT DISTINCT resource_id, last_updated FROM search_index
       WHERE tenant_id = 'default' AND resource_type = 'Patient'
         AND param_name = 'address' AND value_string IS NOT NULL
         AND COALESCE(value_string_folded, lower(value_string)) LIKE 'springfield%' ESCAPE '\'
       ORDER BY last_updated DESC, resource_id ASC LIMIT 22 ) c
JOIN resources r ON r.tenant_id = 'default' AND r.resource_type = 'Patient'
                AND r.id = c.resource_id
WHERE r.is_deleted = FALSE
ORDER BY c.last_updated DESC, c.resource_id ASC;

\echo ''
\echo '######## AJ. STRING Patient?address:contains=Springfield — leading % is not sargable ########'
-- Half of the benchmark's string requests carry `:contains`, whose leading `%`
-- no btree can seek. The conjunct still helps: the pattern index is partial and
-- covering, so the unavoidable scan runs over ~250k entries index-only instead
-- of over the parameter slice of a 372 MB non-partial index with a heap fetch
-- per candidate. Expect a scan, not a seek — but check which index and whether
-- Heap Fetches is 0.
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT r.id, r.version_id, r.data, r.last_updated, r.fhir_version,
       r.last_updated AS sort_key
FROM ( SELECT DISTINCT resource_id, last_updated FROM search_index
       WHERE tenant_id = 'default' AND resource_type = 'Patient'
         AND param_name = 'address' AND value_string IS NOT NULL
         AND COALESCE(value_string_folded, lower(value_string)) LIKE '%springfield%' ESCAPE '\'
       ORDER BY last_updated DESC, resource_id ASC LIMIT 22 ) c
JOIN resources r ON r.tenant_id = 'default' AND r.resource_type = 'Patient'
                AND r.id = c.resource_id
WHERE r.is_deleted = FALSE
ORDER BY c.last_updated DESC, c.resource_id ASC;

\echo ''
\echo '######## AK. COMPOSITE code-value-date — the family with NO covering index ########'
-- v26 narrowed `idx_search_composite_token_quantity` to
-- `value_quantity_value IS NOT NULL` and `idx_search_composite_token_token` to
-- its slot-2 token, so a composite row now inserts into ONE family index
-- instead of both. 19 of the 46 R4 composites are token+quantity and 20 are
-- token+token; this shape is in NEITHER group, and it is the one the narrowing
-- could plausibly have stranded.
--
-- It should not be stranded, and this section is how you check. Nothing was
-- taken away from `idx_search_token_code` — it stays partial on
-- `value_token_code IS NOT NULL` only, so it still holds composite rows and is
-- the catch-all for every composite family without one of its own. EXPECT an
-- index scan on `idx_search_token_code`, seeking (tenant_id, resource_type,
-- param_name, value_token_code) and filtering `value_date` from the payload or
-- the heap. A Seq Scan or a scan of `idx_search_resource` here means the
-- narrowing DID strand this family and section 2 of migrate_v25_to_v26 needs a
-- token+date covering index of its own.
--
-- (This may legitimately return no rows: `fold_composites` drops incomplete
-- groups, so a `code-value-date` row exists only where the Observation actually
-- carries a dateTime value. An empty result still shows the plan, which is what
-- is being checked.)
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT id, version_id, data, last_updated, fhir_version FROM resources
WHERE tenant_id = 'default' AND resource_type = 'Observation' AND is_deleted = FALSE
  AND (id IN (SELECT resource_id FROM search_index
              WHERE tenant_id = 'default' AND resource_type = 'Observation'
                AND param_name = 'code-value-date'
                AND composite_group IS NOT NULL
                AND (value_token_code = '8302-2') AND (value_date >= '2010-01-01')))
ORDER BY last_updated DESC, id ASC LIMIT 21;

\echo ''
\echo '######## AL. v26 PREDICATE REACH — rows an index holds vs rows it could hold ########'
-- Each v26 predicate is only worth what it excludes, and it is only SAFE if the
-- query builder's SQL implies it (predtest.c proves `x IS NOT NULL` from any
-- strict operator over x, and an OR predicate from either arm). The plans above
-- are the safety check; this is the size check. `kept` is what the narrowed
-- index now indexes, `dropped` is the insert every row of that kind no longer
-- pays, once per row per import.
SELECT 'idx_search_string_folded' AS index_name,
       count(*) FILTER (WHERE value_string IS NOT NULL) AS kept,
       count(*) FILTER (WHERE value_string IS NULL)     AS dropped
FROM search_index WHERE tenant_id = 'default'
UNION ALL
SELECT 'idx_search_composite_token_quantity',
       count(*) FILTER (WHERE value_quantity_value IS NOT NULL),
       count(*) FILTER (WHERE value_quantity_value IS NULL)
FROM search_index WHERE tenant_id = 'default' AND composite_group IS NOT NULL
UNION ALL
SELECT 'idx_search_composite_token_token',
       count(*) FILTER (WHERE value_token_code_2 IS NOT NULL OR value_token_system_2 IS NOT NULL),
       count(*) FILTER (WHERE value_token_code_2 IS NULL AND value_token_system_2 IS NULL)
FROM search_index WHERE tenant_id = 'default' AND composite_group IS NOT NULL
UNION ALL
SELECT 'idx_search_composite (dropped entirely)',
       0, count(*)
FROM search_index WHERE tenant_id = 'default' AND composite_group IS NOT NULL;

\echo ''
\echo '######## AM. COMPOSITE FAST PATH — the SQL that actually ships (broad value) ########'
-- Sections A0-E and T all measure `SELECT … FROM resources WHERE id IN (…)`.
-- That is NOT what a composite search runs. A lone composite parameter is a
-- single membership test, so `single_index_predicate` extracts it and
-- `search()` takes the v17 fast path — the form below. Every earlier composite
-- section measured a plan the server does not execute, which is why the
-- 88 -> 114 ms regression was invisible to this capture.
--
-- v27 keys `idx_search_composite_token_quantity` on
-- (tenant_id, resource_type, param_name, value_token_code,
--  last_updated DESC, resource_id ASC) INCLUDE (value_quantity_value,
--  value_token_system), so EXPECT: `Index Only Scan using
-- idx_search_composite_token_quantity`, **no Sort node**, `Filter:
-- (value_quantity_value > 100)` on the scan itself, and `rows` on the Unique
-- capped near 21 with far fewer index tuples read than the code slice holds.
--
-- A `Sort` node here, or `Heap Fetches` in the thousands, means the index is
-- not being used as intended and section AQ will say which index was.
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT r.id, r.version_id, r.data, r.last_updated, r.fhir_version,
       r.last_updated AS sort_key
FROM ( SELECT DISTINCT resource_id, last_updated FROM search_index
       WHERE tenant_id = 'default' AND resource_type = 'Observation'
         AND param_name = 'combo-code-value-quantity'
         AND composite_group IS NOT NULL
         AND (value_token_code = '8867-4') AND (value_quantity_value > 100)
       ORDER BY last_updated DESC, resource_id ASC LIMIT 21 ) c
JOIN resources r ON r.tenant_id = 'default' AND r.resource_type = 'Observation'
                AND r.id = c.resource_id
WHERE r.is_deleted = FALSE
ORDER BY c.last_updated DESC, c.resource_id ASC;

\echo ''
\echo '######## AN. COMPOSITE FAST PATH — system|code form (the 7,767-call statement) ########'
-- Same index, one extra qual. `value_token_system` is INCLUDE payload, so this
-- must stay an Index Only Scan with the system test in `Filter`. If it shows
-- an Index Scan with heap fetches, the payload is not being used and the
-- second composite statement is paying a random page per candidate row.
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT r.id, r.version_id, r.data, r.last_updated, r.fhir_version,
       r.last_updated AS sort_key
FROM ( SELECT DISTINCT resource_id, last_updated FROM search_index
       WHERE tenant_id = 'default' AND resource_type = 'Observation'
         AND param_name = 'combo-code-value-quantity'
         AND composite_group IS NOT NULL
         AND (value_token_system = 'http://loinc.org' AND value_token_code = '8480-6')
         AND (value_quantity_value > 140)
       ORDER BY last_updated DESC, resource_id ASC LIMIT 21 ) c
JOIN resources r ON r.tenant_id = 'default' AND r.resource_type = 'Observation'
                AND r.id = c.resource_id
WHERE r.is_deleted = FALSE
ORDER BY c.last_updated DESC, c.resource_id ASC;

\echo ''
\echo '######## AO. COMPOSITE FAST PATH — sparse regime, the trade v27 accepts ########'
-- `code-value-quantity=29463-7$lt5` is body weight under 5 kg: a code slice of
-- tens of thousands of rows of which almost none pass the quantity filter. The
-- v26 index could seek straight to an empty range; v27 walks the code slice in
-- `last_updated` order instead. THIS IS THE COST OF THE CHANGE and this section
-- is how much it is. It should be single-digit milliseconds and index-only —
-- read `Index Only Scan … rows removed by filter` for the walk length.
--
-- If this comes back at tens of milliseconds while AM is fast, the right answer
-- is to re-add a value-first `(…, value_token_code, value_quantity_value)
-- INCLUDE (resource_id, last_updated)` index ALONGSIDE this one and let the
-- planner choose (v19's two-shapes doctrine) — at the price of one more btree
-- insert per composite token+quantity row.
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT DISTINCT resource_id, last_updated FROM search_index
WHERE tenant_id = 'default' AND resource_type = 'Observation'
  AND param_name = 'code-value-quantity'
  AND composite_group IS NOT NULL
  AND (value_token_code = '29463-7') AND (value_quantity_value < 5)
ORDER BY last_updated DESC, resource_id ASC LIMIT 21;

\echo ''
\echo '######## AP. COMPOSITE FAST PATH — zero-match code (must stay instant) ########'
-- `non-existent-code$gt0`. The token equality finds nothing, so neither index
-- shape can walk anything. A control: if this is slow the leading key columns
-- are not being seeked at all.
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT DISTINCT resource_id, last_updated FROM search_index
WHERE tenant_id = 'default' AND resource_type = 'Observation'
  AND param_name = 'code-value-quantity'
  AND composite_group IS NOT NULL
  AND (value_token_code = 'non-existent-code') AND (value_quantity_value > 0)
ORDER BY last_updated DESC, resource_id ASC LIMIT 21;

\echo ''
\echo '######## AQ. WHICH INDEX SERVED THE COMPOSITE SECTIONS ########'
-- Scans and tuples read for every index that can hold a composite row. The
-- number to watch is tuples-per-scan on `idx_search_composite_token_quantity`:
-- v27 exists to make it ~21 for a broad value instead of the whole match set.
-- `idx_search_token_code` also covers composite rows (it is partial on
-- `value_token_code IS NOT NULL` only) but has no quantity column, so if IT is
-- taking the composite scans the composite index is being rejected and the
-- quantity filter is costing a heap fetch per row.
SELECT s.indexrelname,
       s.idx_scan,
       s.idx_tup_read,
       CASE WHEN s.idx_scan > 0 THEN s.idx_tup_read / s.idx_scan END AS tuples_per_scan,
       pg_size_pretty(pg_relation_size(s.indexrelid)) AS size
FROM pg_stat_user_indexes s
WHERE s.relname = 'search_index'
  AND s.indexrelname IN ('idx_search_composite_token_quantity',
                         'idx_search_composite_token_token',
                         'idx_search_token_code',
                         'idx_search_quantity_recent',
                         'idx_search_resource')
ORDER BY s.idx_scan DESC;

\echo ''
\echo '######## AR. v27 INDEX DEFINITIONS — the sort key must be KEY, not INCLUDE ########'
-- Trap 6, made checkable. `indexdef` must show `last_updated DESC, resource_id`
-- BEFORE the `INCLUDE (` on both composite indexes. v26 regressed by putting
-- `last_updated` nowhere at all; v17 regressed by putting it in `INCLUDE`.
SELECT indexname, indexdef FROM pg_indexes
WHERE tablename = 'search_index'
  AND indexname LIKE 'idx_search_composite%'
ORDER BY indexname;

-- ── Seat G: index-removal verification (relabelled AS.. to avoid collision) ──
\echo '################ v27 — THE FOUR DROPPED INDEXES ################'
-- v27 drops `idx_search_reference`, `idx_search_reference_display`,
-- `idx_search_token_display` and `idx_search_string_folded` on the argument
-- that NO predicate the query builder emits can seek any of them, and that the
-- (tenant_id, resource_type, param_name) slice each would still have been
-- scanned for is served by a surviving index over a superset of its rows.
--
-- Sections AM–AR are that argument's falsification test. Read them together:
-- a Seq Scan on `search_index`, or an index scan with no `Index Cond` on the
-- value column where one is claimed, means the argument is wrong and the index
-- has to come back.

\echo ''
\echo '######## AS. LIVE INDEX SET on search_index (must show none of the four) ########'
-- The record of what actually exists after the migration, so a later run can be
-- read without guessing which schema version produced it.
SELECT indexname, pg_size_pretty(pg_relation_size(indexname::regclass)) AS size, indexdef
FROM pg_indexes WHERE tablename = 'search_index' ORDER BY indexname;

-- A real stored reference for the shapes below. COALESCE keeps this to exactly
-- one row even on an empty table, so `:'ref'` is always set.
SELECT COALESCE((SELECT value_reference FROM search_index
                 WHERE tenant_id = 'default' AND resource_type = 'Observation'
                   AND param_name = 'subject' AND value_reference IS NOT NULL
                 LIMIT 1), 'Patient/no-such-id') AS ref \gset

\echo ''
\echo '######## AT. REFERENCE Observation?subject=<ref> — does text_pattern_ops serve `=`? ########'
-- THE claim behind dropping `idx_search_reference`. It and
-- `idx_search_reference_pattern` have identical key columns and an identical
-- partial predicate; the only difference is the operator class. The drop is
-- correct only because `text_pattern_ops` carries `=` at
-- BTEqualStrategyNumber — text equality is `texteq`, byte equality under any
-- deterministic collation, so both families share it.
--
-- EXPECT: `Index Scan using idx_search_reference_pattern` (or a Bitmap Index
-- Scan on it) with `Index Cond: ((value_reference = '…') OR (value_reference ~>=~ …))`,
-- and single- to low-double-digit buffers.
--
-- FALSIFIED IF: the value predicate appears under `Filter:` instead of
-- `Index Cond:`, or the plan reads the whole `subject` slice. That would mean
-- `=` is not reachable through this index and `idx_search_reference` must be
-- restored.
         AND param_name = 'subject'
         AND (value_reference = :'ref'
              OR value_reference LIKE :'ref' || '/\_history/%' ESCAPE '\')
       ORDER BY last_updated DESC, resource_id ASC LIMIT 22 ) c
\echo '######## AU. AN COUNTERFACTUAL — the same shape with idx_search_reference back ########'
-- The paired "before", in the same pass and against the same cache. DDL is
-- transactional, so the ROLLBACK leaves the database exactly as it was.
--
-- This one BUILDS an index over every reference row (the largest single row
-- class left in the table), so it is the most expensive block in this file.
-- The timeouts bound it: if it cannot finish, the transaction aborts and the
-- schema is untouched — that is a missing measurement, not a broken database.
--
-- EXPECT: the same plan and the same hit+read as AN, with the index name
-- swapped. That is what "subsumed" means here. A materially cheaper plan would
-- mean the drop cost something real on the read side and should be reverted.
BEGIN;
SET LOCAL lock_timeout = '30s';
SET LOCAL statement_timeout = '600s';
CREATE INDEX idx_search_reference_v26
  ON search_index (tenant_id, resource_type, param_name, value_reference)
  WHERE value_reference IS NOT NULL;
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT r.id, r.version_id, r.data, r.last_updated, r.fhir_version,
       r.last_updated AS sort_key
FROM ( SELECT DISTINCT resource_id, last_updated
       FROM search_index
       WHERE tenant_id = 'default' AND resource_type = 'Observation'
         AND param_name = 'subject'
         AND (value_reference = :'ref'
              OR value_reference LIKE :'ref' || '/\_history/%' ESCAPE '\')
       ORDER BY last_updated DESC, resource_id ASC LIMIT 22 ) c
JOIN resources r ON r.tenant_id = 'default' AND r.resource_type = 'Observation'
                AND r.id = c.resource_id
WHERE r.is_deleted = FALSE
ORDER BY c.last_updated DESC, c.resource_id ASC;
ROLLBACK;

\echo ''
\echo '######## AV. TOKEN Observation?code:text=blood — the token_display shape ########'
-- `value_token_display ILIKE $n`. `idx_search_token_display` was a btree in the
-- DEFAULT operator class, and `match_pattern_prefix` derives bounds for a
-- case-insensitive pattern only into a `text_pattern_ops` family, and only when
-- the fixed prefix carries no letter — so this predicate could never seek that
-- index. It could only scan the index's (tenant_id, resource_type, param_name)
-- slice with the ILIKE as a filter, and `idx_search_token_code` covers that
-- slice over a superset of the rows (`IndexValue::Token` always sets
-- `value_token_code`; `from_composite` never sets a display).
--
-- EXPECT: an index scan on one of the surviving (tenant, type, param) indexes
-- with `Filter: (value_token_display ~~* '%blood%')`, and rows removed by
-- filter. FALSIFIED IF: `Seq Scan on search_index`.
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT r.id, r.version_id, r.data, r.last_updated, r.fhir_version,
       r.last_updated AS sort_key
FROM ( SELECT DISTINCT resource_id, last_updated FROM search_index
       WHERE tenant_id = 'default' AND resource_type = 'Observation'
         AND param_name = 'code'
         AND value_token_display ILIKE '%blood%'
       ORDER BY last_updated DESC, resource_id ASC LIMIT 22 ) c
\echo '######## AW. REFERENCE Observation?subject:code-text=a — the reference_display shape ########'
-- `value_reference_display ILIKE $n || '%'`. Same argument as AP, and stronger:
-- the pattern here is an `OpExpr` (`$n || '%'`), not a `Const`, so no fixed
-- prefix can be read off it at plan time even in principle.
--
-- EXPECT: an index scan on a surviving (tenant, type, param) index with
-- `Filter: (value_reference_display ~~* …)`. FALSIFIED IF: Seq Scan.
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT r.id, r.version_id, r.data, r.last_updated, r.fhir_version,
       r.last_updated AS sort_key
FROM ( SELECT DISTINCT resource_id, last_updated FROM search_index
       WHERE tenant_id = 'default' AND resource_type = 'Observation'
         AND param_name = 'subject'
         AND value_reference_display ILIKE 'a' || '%'
       ORDER BY last_updated DESC, resource_id ASC LIMIT 22 ) c
JOIN resources r ON r.tenant_id = 'default' AND r.resource_type = 'Observation'
                AND r.id = c.resource_id
WHERE r.is_deleted = FALSE
ORDER BY c.last_updated DESC, c.resource_id ASC;

\echo ''
\echo '######## AX. v27 INDEX ENTRIES REMOVED — rows x indexes each row stopped entering ########'
-- The write-side size of the change, counted the way the import path pays it:
-- one btree insert per row per applicable index. `rows` is how many rows of
-- that kind the table holds; `entries_saved` is how many index insertions per
-- import v27 removes for them.
--
-- `idx_search_string` (`:exact`) and `idx_search_string_folded_pattern` (the
-- COALESCE the default string search seeks) are deliberately NOT in this list:
-- they survive, and a string row still enters both.
SELECT 'reference rows (idx_search_reference)' AS class,
       count(*) FILTER (WHERE value_reference IS NOT NULL) AS rows,
       count(*) FILTER (WHERE value_reference IS NOT NULL) AS entries_saved
FROM search_index WHERE tenant_id = 'default'
UNION ALL
SELECT 'reference rows with a display (idx_search_reference_display)',
       count(*) FILTER (WHERE value_reference_display IS NOT NULL),
       count(*) FILTER (WHERE value_reference_display IS NOT NULL)
FROM search_index WHERE tenant_id = 'default'
UNION ALL
SELECT 'token rows with a display (idx_search_token_display)',
       count(*) FILTER (WHERE value_token_display IS NOT NULL),
       count(*) FILTER (WHERE value_token_display IS NOT NULL)
FROM search_index WHERE tenant_id = 'default'
UNION ALL
SELECT 'rows in idx_search_string_folded (predicate: value_string IS NOT NULL)',
       count(*) FILTER (WHERE value_string IS NOT NULL),
       count(*) FILTER (WHERE value_string IS NOT NULL)
FROM search_index WHERE tenant_id = 'default';

-- ── Seat F: FTS/GIN verification (relabelled BE.. to avoid collision) ──
\echo '################ FULL-TEXT WRITE PATH (resource_fts) ################'
-- Nothing above this line looks at `resource_fts`, and it needs to: on run
-- 33086933938 `INSERT INTO resource_fts` was 4,225.7 s of the crud suite's
-- 17,190 s of Postgres execution time — 385,650 calls at 10.96 ms to insert ONE
-- row. A GIN index costs roughly what its ENTRY TREE costs: a lexeme that has
-- been seen before only appends to a posting list, while a lexeme that has not
-- is a fresh key inserted at a random position in the tree, with the page split,
-- the WAL and the cache miss that implies. `collect_strings` therefore stops
-- feeding `_content` the ids and literal references that are, by construction,
-- never seen twice (schema v27). These queries are how you check that it worked
-- and that it is still working.

\echo ''
\echo '######## BE. resource_fts PHYSICAL SIZE ########'
-- EXPECT `idx_fts_content` to be a small multiple of the heap, not an order of
-- magnitude above it. Measured locally over 177,603 Synthea resources it fell
-- from 88 MB to 30 MB when the ids and references came out.
SELECT 'heap' AS part, pg_size_pretty(pg_relation_size('resource_fts')) AS size,
       (SELECT count(*) FROM resource_fts) AS rows
UNION ALL
SELECT c.relname, pg_size_pretty(pg_relation_size(c.oid)), NULL
FROM pg_class c JOIN pg_index i ON i.indexrelid = c.oid
WHERE i.indrelid = 'resource_fts'::regclass
ORDER BY 1;

\echo ''
\echo '######## BF. GIN ENTRY-TREE KEYS PER ROW — the number that sets the write cost ########'
-- `ts_stat` over a bounded sample: `distinct_keys` is how many entry-tree keys
-- those rows demand and `entries` how many posting-list slots. The ratio is the
-- whole story. Measured over 45,000 crud-shaped resources the vectors held
-- 224,438 distinct lexemes before the v27 writer and 333 after, at an almost
-- unchanged 3.7M vs 3.3M entries: 99.85% of the entry tree was ids and links,
-- and none of the actual words moved. A `distinct_keys` that grows roughly in
-- step with `sampled_rows` means one-off lexemes are being indexed again and
-- the filter has regressed.
SELECT (SELECT count(*) FROM (SELECT 1 FROM resource_fts LIMIT 20000) s) AS sampled_rows,
       count(*) AS distinct_keys, sum(nentry) AS entries
FROM ts_stat('SELECT content_tsvector FROM resource_fts LIMIT 20000');

\echo ''
\echo '######## BG. UUID RESIDUE — where the entry tree still gets one-off keys ########'
-- Not expected to be zero, and deliberately so. Only `id`, `reference`,
-- `fullUrl` and `versionId` are filtered; an `Identifier.value` is kept, because
-- that is where an MRN, an NPI or an accession number lives and searching
-- `_content` for one is a real query — Synthea happens to make its MRNs UUIDs.
-- Over the 177,603-resource Synthea corpus that filter takes the entry tree from
-- 638,685 distinct keys to 174,891, and what is left is overwhelmingly those
-- identifiers. The crud suite, which is the only workload that actually writes
-- `resource_fts` (the bundle path does not), reuses nine fixed seed resources,
-- so its identifiers repeat and its share falls 224,438 -> 333.
--
-- What this query is FOR: a sharp rise in `rows_with_a_uuid_lexeme` per sampled
-- row, on a database imported by v27 code, means ids or references are being
-- indexed again. Rows written before v27 also show up here; a pre-v27 vector is
-- a superset of what the current writer produces, so it is stale rather than
-- wrong, and `$reindex` rebuilds it.
SELECT count(*) AS rows_with_a_uuid_lexeme
FROM (SELECT content_tsvector FROM resource_fts LIMIT 20000) s
WHERE EXISTS (
  SELECT 1 FROM unnest(tsvector_to_array(s.content_tsvector)) AS lex
  WHERE lex ~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
);

\echo ''
\echo '######## BH. v27 SHAPE — the upsert needs its key, the writer needs no FK ########'
-- `idx_fts_lookup` must be UNIQUE: it is the conflict target of
-- `FTS_UPSERT_SQL`, which is what lets an update replace the row in place
-- instead of DELETE-then-INSERT (227.3 s of the crud suite). `fk_fts_resource`
-- must be absent: it charged a `SELECT 1 … FOR KEY SHARE` on `resources` per
-- full-text write, for a guarantee `purge`/`purge_all`/`purge_tenant_data`
-- already give explicitly (same trade as `migrate_v22_to_v23`).
SELECT c.relname AS index_name, i.indisunique AS is_unique
FROM pg_class c JOIN pg_index i ON i.indexrelid = c.oid
WHERE i.indrelid = 'resource_fts'::regclass AND c.relname = 'idx_fts_lookup';
SELECT count(*) AS fts_foreign_keys
FROM pg_constraint WHERE conrelid = 'resource_fts'::regclass AND contype = 'f';

\echo ''
\echo '######## BI. _content STILL READS THROUGH THE GIN INDEX ########'
-- The benchmark never issues `_text` or `_content`, so this is the only place
-- the read path is looked at. EXPECT a Bitmap Index Scan on `idx_fts_content`.
-- A Seq Scan on `resource_fts` means the narrowing broke the read side, which
-- would be a correctness regression, not a performance one.
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT id FROM resources
WHERE tenant_id = 'default' AND resource_type = 'Patient' AND is_deleted = FALSE
  AND id IN (SELECT resource_id FROM resource_fts
             WHERE tenant_id = 'default' AND resource_type = 'Patient'
               AND content_tsvector @@ plainto_tsquery('english', 'Springfield'))
LIMIT 21;

-- ── base-quantity capture (relabelled BM.. to avoid collision) ──
\echo '######## BM. BASE quantity Observation?value-quantity — the shape nothing modelled ########'
-- Added 2026-08-27 after run 33077075313. Normalising that run's per-shape p99
-- against the median shape ratio — the estimator for the neighbour-load factor
-- that multiplied every statement, since a tight IQR means one common cause —
-- left most shapes at 0.93-1.04x (unchanged, as expected) but put
-- `quantity Observation value-quantity` at 0.20x: five times worse than the
-- environment explains.
--
-- Nothing in that commit touched base quantity rows. The census confirms it:
-- `value-quantity` held 512,311 rows over 512,311 resources before and after.
-- What DID change is the table around it — the commit removed 8.55M rows
-- (-21.6%), so `value-quantity` is now a materially larger fraction of
-- `search_index`, and per-`param_name` selectivity is what the planner uses to
-- choose between the value-first and recent-first indexes. A statistics-driven
-- plan flip on an untouched shape is the hypothesis; this section is what can
-- confirm or kill it.
--
-- Three values spanning the selectivity range the benchmark actually sends
-- (k6/searchConfig.js: value-quantity uses gt/lt over 0-200 plus a no-match
-- sentinel), because a plan that is right for one end can be wrong for the
-- other — the exact way the v20 regression hid from a single-value capture.
\echo '-- AM1: broad (most Observations match) — early termination expected'
EXPLAIN (ANALYZE, BUFFERS)
SELECT r.id, r.version_id, r.data, r.last_updated, r.fhir_version, r.last_updated AS sort_key
FROM ( SELECT DISTINCT resource_id, last_updated FROM search_index
       WHERE tenant_id = 'default' AND resource_type = 'Observation'
         AND param_name = 'value-quantity' AND value_quantity_value > 0
       ORDER BY last_updated DESC, resource_id ASC LIMIT 22 ) s
JOIN resources r ON r.tenant_id = 'default' AND r.resource_type = 'Observation'
                AND r.id = s.resource_id AND NOT r.is_deleted;

\echo '-- AM2: selective (few match) — value-first index must still win'
EXPLAIN (ANALYZE, BUFFERS)
SELECT r.id, r.version_id, r.data, r.last_updated, r.fhir_version, r.last_updated AS sort_key
FROM ( SELECT DISTINCT resource_id, last_updated FROM search_index
       WHERE tenant_id = 'default' AND resource_type = 'Observation'
         AND param_name = 'value-quantity' AND value_quantity_value > 900000
       ORDER BY last_updated DESC, resource_id ASC LIMIT 22 ) s
JOIN resources r ON r.tenant_id = 'default' AND r.resource_type = 'Observation'
                AND r.id = s.resource_id AND NOT r.is_deleted;

\echo '-- AM3: what the planner believes about this parameter'
SELECT n_distinct, most_common_freqs[1:3] AS top_freqs
FROM pg_stats WHERE tablename = 'search_index' AND attname = 'param_name';
SELECT count(*) AS value_quantity_rows,
       (SELECT count(*) FROM search_index WHERE tenant_id = 'default') AS table_rows
FROM search_index
WHERE tenant_id = 'default' AND resource_type = 'Observation' AND param_name = 'value-quantity';
