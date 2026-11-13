SELECT
  ((SELECT w0.value->>'family' FROM LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(r.data->'name') = 'array' THEN r.data->'name' WHEN jsonb_typeof(r.data->'name') IS NOT NULL THEN jsonb_build_array(r.data->'name') ELSE '[]'::jsonb END)) WITH ORDINALITY AS w0(value, ordinality) WHERE (w0.value->>'use' = 'official') ORDER BY COALESCE(w0.ordinality, -1) LIMIT 1))::text AS "pick"
FROM resources res
CROSS JOIN LATERAL (SELECT res.data #> '{}' AS data, res.id AS id OFFSET 0) r
WHERE res.tenant_id = $1
  AND res.resource_type = $2
  AND res.is_deleted = false
ORDER BY res.last_updated, res.id COLLATE "C"

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
    "pick"
  ],
  "decodes": [
    "Text"
  ],
  "client_limit": null
}
*/
