SELECT
  ((SELECT fe.value->>'family' FROM jsonb_array_elements(CASE WHEN jsonb_typeof((r.data)::jsonb->'name') = 'array' THEN (r.data)::jsonb->'name' WHEN jsonb_typeof((r.data)::jsonb->'name') IS NOT NULL THEN jsonb_build_array((r.data)::jsonb->'name') ELSE '[]'::jsonb END) AS fe(value) LIMIT 1 OFFSET 1))::text AS "family"
FROM resources r
WHERE r.tenant_id = $1
  AND r.resource_type = $2
  AND r.is_deleted = false
ORDER BY r.last_updated, r.id COLLATE "C"

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
    "Auto"
  ],
  "client_limit": null
}
*/
