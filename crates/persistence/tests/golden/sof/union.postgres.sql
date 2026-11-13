SELECT
  r.data->>'id' AS "value"
FROM resources res
CROSS JOIN LATERAL (SELECT res.data #> '{}' AS data, res.id AS id OFFSET 0) r
WHERE res.tenant_id = $1
  AND res.resource_type = $2
  AND res.is_deleted = false
UNION ALL
SELECT
  fe.value->>'family' AS "value"
FROM resources res
CROSS JOIN LATERAL (SELECT res.data #> '{}' AS data, res.id AS id OFFSET 0) r
JOIN LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(r.data->'name') = 'array' THEN r.data->'name' WHEN jsonb_typeof(r.data->'name') IS NOT NULL THEN jsonb_build_array(r.data->'name') ELSE '[]'::jsonb END)) WITH ORDINALITY AS fe(value, ordinality) ON TRUE
WHERE res.tenant_id = $1
  AND res.resource_type = $2
  AND res.is_deleted = false

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
    "value"
  ],
  "decodes": [
    "Text"
  ],
  "client_limit": null
}
*/
