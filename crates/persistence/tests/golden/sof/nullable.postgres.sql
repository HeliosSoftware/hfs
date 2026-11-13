SELECT
  fe.value->>'family' AS "family"
FROM resources res
CROSS JOIN LATERAL (SELECT res.data #> '{}' AS data, res.id AS id OFFSET 0) r
LEFT JOIN LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(r.data->'name') = 'array' THEN r.data->'name' WHEN jsonb_typeof(r.data->'name') IS NOT NULL THEN jsonb_build_array(r.data->'name') ELSE '[]'::jsonb END)) WITH ORDINALITY AS fe(value, ordinality) ON TRUE AND (fe.value->>'use' = 'official')
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
    "family"
  ],
  "decodes": [
    "Text"
  ],
  "client_limit": null
}
*/
