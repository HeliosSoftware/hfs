SELECT
  r.data->>'id' AS "r",
  fe.value->>'family' AS "u0"
FROM resources res
CROSS JOIN LATERAL (SELECT res.data #> '{}' AS data, res.id AS id OFFSET 0) r
JOIN LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(r.data->'name') = 'array' THEN r.data->'name' WHEN jsonb_typeof(r.data->'name') IS NOT NULL THEN jsonb_build_array(r.data->'name') ELSE '[]'::jsonb END)) WITH ORDINALITY AS fe(value, ordinality) ON TRUE
WHERE res.tenant_id = $1
  AND res.resource_type = $2
  AND res.is_deleted = false
  AND res.last_updated >= $3
  AND res.id COLLATE "C" > $4
  AND res.id COLLATE "C" <= $5
ORDER BY res.id COLLATE "C", COALESCE(fe.ordinality, -1)

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
