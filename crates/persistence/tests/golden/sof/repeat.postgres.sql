WITH RECURSIVE rec_0(rid, node) AS (
  (SELECT r.id AS rid, je.value AS node
  FROM resources r JOIN LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(r.data->'item') = 'array' THEN r.data->'item' WHEN jsonb_typeof(r.data->'item') IS NOT NULL THEN jsonb_build_array(r.data->'item') ELSE '[]'::jsonb END)) AS je(value) ON TRUE
  WHERE r.tenant_id = $1
  AND r.resource_type = $2
  AND r.is_deleted = false
  UNION ALL
  SELECT r.id AS rid, je.value AS node
  FROM resources r JOIN LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(r.data#>'{answer,item}') = 'array' THEN r.data#>'{answer,item}' WHEN jsonb_typeof(r.data#>'{answer,item}') IS NOT NULL THEN jsonb_build_array(r.data#>'{answer,item}') ELSE '[]'::jsonb END)) AS je(value) ON TRUE
  WHERE r.tenant_id = $1
  AND r.resource_type = $2
  AND r.is_deleted = false)
  UNION ALL
  SELECT rec_0.rid, _step.value AS node
  FROM rec_0, LATERAL (SELECT rs0.value FROM LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(rec_0.node->'item') = 'array' THEN rec_0.node->'item' WHEN jsonb_typeof(rec_0.node->'item') IS NOT NULL THEN jsonb_build_array(rec_0.node->'item') ELSE '[]'::jsonb END)) AS rs0(value)
    UNION ALL
    SELECT rs1.value FROM LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(rec_0.node->'answer') = 'array' THEN rec_0.node->'answer' WHEN jsonb_typeof(rec_0.node->'answer') IS NOT NULL THEN jsonb_build_array(rec_0.node->'answer') ELSE '[]'::jsonb END)) AS rs0(value) JOIN LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(rs0.value->'item') = 'array' THEN rs0.value->'item' WHEN jsonb_typeof(rs0.value->'item') IS NOT NULL THEN jsonb_build_array(rs0.value->'item') ELSE '[]'::jsonb END)) AS rs1(value) ON TRUE) AS _step(value)
)
SELECT
  r.data->>'id' AS "id",
  ((SELECT coalesce(jsonb_agg(ca1.value ORDER BY COALESCE(ca0.ordinality, -1), COALESCE(ca1.ordinality, -1)), '[]'::jsonb) FROM LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(r.data->'item') = 'array' THEN r.data->'item' WHEN jsonb_typeof(r.data->'item') IS NOT NULL THEN jsonb_build_array(r.data->'item') ELSE '[]'::jsonb END)) WITH ORDINALITY AS ca0(value, ordinality) JOIN LATERAL jsonb_array_elements(CASE WHEN jsonb_typeof(ca0.value->'linkId') = 'array' THEN ca0.value->'linkId' ELSE jsonb_build_array(ca0.value->'linkId') END) WITH ORDINALITY AS ca1(value, ordinality) ON TRUE WHERE ca0.value->'linkId' IS NOT NULL))::text AS "roots",
  rec_0.node->>'linkId' AS "link",
  ((SELECT coalesce(jsonb_agg(ca1.value ORDER BY COALESCE(ca0.ordinality, -1), COALESCE(ca1.ordinality, -1)), '[]'::jsonb) FROM LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(rec_0.node->'answer') = 'array' THEN rec_0.node->'answer' WHEN jsonb_typeof(rec_0.node->'answer') IS NOT NULL THEN jsonb_build_array(rec_0.node->'answer') ELSE '[]'::jsonb END)) WITH ORDINALITY AS ca0(value, ordinality) JOIN LATERAL jsonb_array_elements(CASE WHEN jsonb_typeof(ca0.value->'valueString') = 'array' THEN ca0.value->'valueString' ELSE jsonb_build_array(ca0.value->'valueString') END) WITH ORDINALITY AS ca1(value, ordinality) ON TRUE WHERE ca0.value->'valueString' IS NOT NULL))::text AS "answers",
  ((SELECT string_agg((ja1.value #>> '{}'), '|' ORDER BY COALESCE(ja0.ordinality, -1), COALESCE(ja1.ordinality, -1)) FROM LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(rec_0.node->'answer') = 'array' THEN rec_0.node->'answer' WHEN jsonb_typeof(rec_0.node->'answer') IS NOT NULL THEN jsonb_build_array(rec_0.node->'answer') ELSE '[]'::jsonb END)) WITH ORDINALITY AS ja0(value, ordinality) JOIN LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(ja0.value->'valueString') = 'array' THEN ja0.value->'valueString' WHEN jsonb_typeof(ja0.value->'valueString') IS NOT NULL THEN jsonb_build_array(ja0.value->'valueString') ELSE '[]'::jsonb END)) WITH ORDINALITY AS ja1(value, ordinality) ON TRUE))::text AS "joined",
  ((SELECT w2.value->>'valueString' FROM LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(rec_0.node->'answer') = 'array' THEN rec_0.node->'answer' WHEN jsonb_typeof(rec_0.node->'answer') IS NOT NULL THEN jsonb_build_array(rec_0.node->'answer') ELSE '[]'::jsonb END)) WITH ORDINALITY AS w2(value, ordinality) WHERE (w2.value->>'valueString' != 'skip') ORDER BY COALESCE(w2.ordinality, -1) LIMIT 1))::text AS "picked"
FROM rec_0 JOIN resources r ON r.id = rec_0.rid AND r.tenant_id = $1
  AND r.resource_type = $2
  AND r.is_deleted = false
ORDER BY 1

/* Runner bindings and output metadata:
{
  "bindings": [
    {
      "type": "text",
      "value": "golden-tenant"
    },
    {
      "type": "text",
      "value": "QuestionnaireResponse"
    }
  ],
  "columns": [
    "id",
    "roots",
    "link",
    "answers",
    "joined",
    "picked"
  ],
  "decodes": [
    "Text",
    "Json",
    "Auto",
    "Json",
    "Text",
    "Text"
  ],
  "client_limit": null
}
*/
