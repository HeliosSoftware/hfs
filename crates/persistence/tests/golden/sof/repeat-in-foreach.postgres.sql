WITH RECURSIVE rec_1(rid, node, ord_path, pkey) AS (
  SELECT r.id AS rid, je.value AS node, lpad((je.ord - 1)::text, 10, '0') AS ord_path, ''::text || '/' || lpad(COALESCE(fe.ordinality, -1)::text, 10, '0') AS pkey
  FROM resources r
JOIN LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(r.data->'item') = 'array' THEN r.data->'item' WHEN jsonb_typeof(r.data->'item') IS NOT NULL THEN jsonb_build_array(r.data->'item') ELSE '[]'::jsonb END)) WITH ORDINALITY AS fe(value, ordinality) ON TRUE
  JOIN LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(fe.value->'item') = 'array' THEN fe.value->'item' WHEN jsonb_typeof(fe.value->'item') IS NOT NULL THEN jsonb_build_array(fe.value->'item') ELSE '[]'::jsonb END)) WITH ORDINALITY AS je(value, ord) ON TRUE
  WHERE r.tenant_id = $1
  AND r.resource_type = $2
  AND r.is_deleted = false
  UNION ALL
  SELECT rec_1.rid, rs0.value AS node, rec_1.ord_path || '.' || lpad((rs0.ord - 1)::text, 10, '0') AS ord_path, rec_1.pkey
  FROM rec_1 JOIN LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(rec_1.node->'item') = 'array' THEN rec_1.node->'item' WHEN jsonb_typeof(rec_1.node->'item') IS NOT NULL THEN jsonb_build_array(rec_1.node->'item') ELSE '[]'::jsonb END)) WITH ORDINALITY AS rs0(value, ord) ON TRUE
)
SELECT
  r.data->>'id' AS "id",
  fe.value->>'linkId' AS "top",
  rec_1.node->>'linkId' AS "child"
FROM resources r
JOIN LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(r.data->'item') = 'array' THEN r.data->'item' WHEN jsonb_typeof(r.data->'item') IS NOT NULL THEN jsonb_build_array(r.data->'item') ELSE '[]'::jsonb END)) WITH ORDINALITY AS fe(value, ordinality) ON TRUE
JOIN rec_1 ON rec_1.rid = r.id AND rec_1.pkey = ''::text || '/' || lpad(COALESCE(fe.ordinality, -1)::text, 10, '0')
WHERE r.tenant_id = $1
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
      "value": "Questionnaire"
    }
  ],
  "columns": [
    "id",
    "top",
    "child"
  ],
  "decodes": [
    "Text",
    "Text",
    "Text"
  ],
  "client_limit": null
}
*/
