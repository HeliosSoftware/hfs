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
-- Read `Buffers: shared read=` as the primary metric, not Execution Time —
-- it is immune to cache state and to runner noise.

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

-- ─────────────────────────────────────────────────────────────────────────────
-- COMPOSITE. Baseline A is what ships today: median 13.2s, 975ms cold single
-- shot, 111,881 buffer READS. Only 222 of 656,737 Observations match, so the
-- outer side is cheap — all the cost is the subquery's random heap access.
-- ─────────────────────────────────────────────────────────────────────────────

\echo ''
\echo '######## A. SHIPPED: OR-prefilter + IN  (baseline: 975ms / 111881 reads) ########'
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

DROP INDEX IF EXISTS tmp_search_token_code_cover;
DROP INDEX IF EXISTS tmp_search_date_cover;
