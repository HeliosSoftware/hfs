SELECT
  rdoc.doc->>'id' AS "r",
  fe.value->>'family' AS "u0"
FROM resources r
CROSS JOIN LATERAL jsonb_extract_path(r.data, VARIADIC '{}'::text[]) AS rdoc(doc)
JOIN LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(COALESCE(rdoc.doc, r.data)->'name') = 'array' THEN COALESCE(rdoc.doc, r.data)->'name' WHEN jsonb_typeof(COALESCE(rdoc.doc, r.data)->'name') IS NOT NULL THEN jsonb_build_array(COALESCE(rdoc.doc, r.data)->'name') ELSE '[]'::jsonb END)) WITH ORDINALITY AS fe(value, ordinality) ON TRUE
WHERE r.tenant_id = $1
  AND r.resource_type = $2
  AND r.is_deleted = false
  AND r.last_updated >= $3
  AND r.id COLLATE "C" > $4
  AND r.id COLLATE "C" <= $5
ORDER BY r.id COLLATE "C", COALESCE(fe.ordinality, -1)

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
    },
    {
      "type": "timestamp",
      "value": "2024-01-01T00:00:00+00:00"
    },
    {
      "type": "text",
      "value": ""
    },
    {
      "type": "text",
      "value": ""
    }
  ],
  "columns": [
    "r",
    "u0"
  ],
  "decodes": [
    "Text",
    "Text"
  ],
  "client_limit": 50
}
*/
