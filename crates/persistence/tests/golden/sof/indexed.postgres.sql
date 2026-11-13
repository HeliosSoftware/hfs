SELECT
  ((SELECT fe.value->>'family' FROM jsonb_array_elements(CASE WHEN jsonb_typeof((r.data)::jsonb->'name') = 'array' THEN (r.data)::jsonb->'name' WHEN jsonb_typeof((r.data)::jsonb->'name') IS NOT NULL THEN jsonb_build_array((r.data)::jsonb->'name') ELSE '[]'::jsonb END) AS fe(value) LIMIT 1 OFFSET 1))::text AS "family"
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
    "family"
  ],
  "decodes": [
    "Text"
  ],
  "client_limit": null
}
*/
