-- Capture real query plans for the search shapes that are still slow (#224).
--
-- Runs against the fully-imported benchmark database, after VACUUM (ANALYZE), so
-- the plans and row counts are the ones the search suite actually gets. The SQL
-- below is exactly what PostgresQueryBuilder emits — keep it in sync if the
-- builder changes.
--
-- Why this exists: the composite shapes measured ~70ms against a synthetic
-- 1.45M-row replica but run at 12-13s here, so the replica's data distribution is
-- not representative and local A/B testing cannot be trusted for these shapes.
-- Stop guessing; read the plan.

\pset pager off
\timing on

\echo '################ CARDINALITY ################'
-- How many index rows per parameter, and how many composite groups per resource.
-- The replica assumed ~2 rows per composite group; if real Synthea carries many
-- more, that alone explains the divergence.
SELECT param_name, count(*) AS rows,
       count(DISTINCT resource_id) AS resources,
       round(count(*)::numeric / NULLIF(count(DISTINCT resource_id), 0), 1) AS rows_per_resource,
       count(DISTINCT composite_group) AS distinct_groups
FROM search_index
WHERE tenant_id = 'default' AND resource_type = 'Observation'
  AND param_name IN ('code-value-quantity','combo-code-value-quantity',
                     'component-code-value-quantity','code','value-quantity',
                     'combo-value-quantity','category')
GROUP BY param_name ORDER BY rows DESC;

SELECT resource_type, count(*) AS resources FROM resources
WHERE tenant_id = 'default' GROUP BY 1 ORDER BY 2 DESC LIMIT 8;

\echo ''
\echo '################ A. COMPOSITE — SHIPPED (prefilter + IN)  [median 13.2s] ################'
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
\echo '################ B. COMPOSITE — CANDIDATE (correlated EXISTS) ################'
-- Rejected earlier on replica numbers (1ms on a match, 440ms on a zero-match, and
-- it lost under pgbench). Against a 13s baseline that trade looks very different.
-- Needs idx_search_composite_param to probe; created below if absent.
CREATE INDEX IF NOT EXISTS tmp_idx_composite_param
  ON search_index (tenant_id, resource_type, param_name, resource_id, composite_group);
ANALYZE search_index;

EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT id, version_id, data, last_updated, fhir_version FROM resources r
WHERE tenant_id = 'default' AND resource_type = 'Observation' AND is_deleted = FALSE
  AND EXISTS (SELECT 1 FROM search_index s
              WHERE s.tenant_id = 'default' AND s.resource_type = 'Observation'
                AND s.resource_id = r.id AND s.param_name = 'code-value-quantity'
              GROUP BY s.composite_group
              HAVING MAX(CASE WHEN s.value_token_code = '8867-4' THEN 1 ELSE 0 END) = 1
                 AND MAX(CASE WHEN s.value_quantity_value > 100 THEN 1 ELSE 0 END) = 1)
ORDER BY last_updated DESC, id ASC LIMIT 21;

\echo ''
\echo '################ C. COMPOSITE EXISTS — ZERO-MATCH (the cliff) ################'
-- searchConfig fires `non-existent$gt0` in every composite list. This is the case
-- that made me reject EXISTS. Measure its real cost.
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT id, version_id, data, last_updated, fhir_version FROM resources r
WHERE tenant_id = 'default' AND resource_type = 'Observation' AND is_deleted = FALSE
  AND EXISTS (SELECT 1 FROM search_index s
              WHERE s.tenant_id = 'default' AND s.resource_type = 'Observation'
                AND s.resource_id = r.id AND s.param_name = 'code-value-quantity'
              GROUP BY s.composite_group
              HAVING MAX(CASE WHEN s.value_token_code = 'non-existent' THEN 1 ELSE 0 END) = 1
                 AND MAX(CASE WHEN s.value_quantity_value > 0 THEN 1 ELSE 0 END) = 1)
ORDER BY last_updated DESC, id ASC LIMIT 21;

DROP INDEX IF EXISTS tmp_idx_composite_param;

\echo ''
\echo '################ D. TOKEN ZERO-MATCH — Encounter?status=missing-status ################'
-- median 8ms / p95 27s. Textbook bimodal: fast when rows match, catastrophic when
-- none do, because the ordered idx_resources_search scan cannot know the set is
-- empty and walks every Encounter.
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT id, version_id, data, last_updated, fhir_version FROM resources
WHERE tenant_id = 'default' AND resource_type = 'Encounter' AND is_deleted = FALSE
  AND (id IN (SELECT resource_id FROM search_index
              WHERE tenant_id = 'default' AND resource_type = 'Encounter'
                AND param_name = 'status' AND (value_token_code = 'missing-status')))
ORDER BY last_updated DESC, id ASC LIMIT 21;

\echo ''
\echo '################ E. TOKEN MATCH — Encounter?status=finished (control) ################'
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT id, version_id, data, last_updated, fhir_version FROM resources
WHERE tenant_id = 'default' AND resource_type = 'Encounter' AND is_deleted = FALSE
  AND (id IN (SELECT resource_id FROM search_index
              WHERE tenant_id = 'default' AND resource_type = 'Encounter'
                AND param_name = 'status' AND (value_token_code = 'finished')))
ORDER BY last_updated DESC, id ASC LIMIT 21;

\echo ''
\echo '################ F. TOKEN ZERO-MATCH without the ordered index ################'
-- If disabling the ordered scan makes the zero-match case fast, idx_resources_search
-- is the cause of the p95 tail and the cliff is a planner-choice problem, not an
-- index-coverage one.
SET enable_indexscan = off;
SET enable_indexonlyscan = off;
EXPLAIN (ANALYZE, BUFFERS, VERBOSE OFF)
SELECT id, version_id, data, last_updated, fhir_version FROM resources
WHERE tenant_id = 'default' AND resource_type = 'Encounter' AND is_deleted = FALSE
  AND (id IN (SELECT resource_id FROM search_index
              WHERE tenant_id = 'default' AND resource_type = 'Encounter'
                AND param_name = 'status' AND (value_token_code = 'missing-status')))
ORDER BY last_updated DESC, id ASC LIMIT 21;
RESET enable_indexscan;
RESET enable_indexonlyscan;

\echo ''
\echo '################ INDEX USAGE SO FAR ################'
SELECT indexrelname, idx_scan FROM pg_stat_user_indexes
WHERE relname IN ('search_index','resources') AND idx_scan > 0
ORDER BY idx_scan DESC LIMIT 12;
