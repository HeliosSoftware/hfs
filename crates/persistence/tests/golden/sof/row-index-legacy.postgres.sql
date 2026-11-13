SELECT
  fe.value->>'family' AS "family",
  ((COALESCE(CAST(fe.ordinality AS INTEGER) - 1, 0))::bigint)::text AS "index"
FROM resources res
CROSS JOIN LATERAL (SELECT res.data #> '{}' AS data, res.id AS id OFFSET 0) r
JOIN LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(r.data->'name') = 'array' THEN r.data->'name' WHEN jsonb_typeof(r.data->'name') IS NOT NULL THEN jsonb_build_array(r.data->'name') ELSE '[]'::jsonb END)) WITH ORDINALITY AS fe(value, ordinality) ON TRUE
WHERE res.tenant_id = $1
  AND res.resource_type = $2
  AND res.is_deleted = false
ORDER BY res.last_updated, res.id COLLATE "C", COALESCE(fe.ordinality, -1)

/* Runner bindings and output metadata:
{
  "bindings": [
    {
      "type": "text",
      "value": "golden-tenant"
    },
    {
      "type": "text",
      "value": "Patient"
    }
  ],
  "columns": [
    "family",
    "index"
  ],
  "decodes": [
    "Text",
    "Integer"
  ],
  "client_limit": null
}
*/
