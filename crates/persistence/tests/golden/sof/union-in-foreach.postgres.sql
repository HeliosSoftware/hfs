SELECT
  r.data->>'id' AS "id",
  fe2.value->>'value' AS "value"
FROM resources res
CROSS JOIN LATERAL (SELECT res.data #> '{}' AS data, res.id AS id OFFSET 0) r
JOIN LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(r.data->'contact') = 'array' THEN r.data->'contact' WHEN jsonb_typeof(r.data->'contact') IS NOT NULL THEN jsonb_build_array(r.data->'contact') ELSE '[]'::jsonb END)) WITH ORDINALITY AS fe(value, ordinality) ON TRUE
JOIN LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(fe.value->'telecom') = 'array' THEN fe.value->'telecom' WHEN jsonb_typeof(fe.value->'telecom') IS NOT NULL THEN jsonb_build_array(fe.value->'telecom') ELSE '[]'::jsonb END)) WITH ORDINALITY AS fe2(value, ordinality) ON TRUE
WHERE res.tenant_id = $1
  AND res.resource_type = $2
  AND res.is_deleted = false
UNION ALL
SELECT
  r.data->>'id' AS "id",
  coalesce(fe3.value#>>'{name,0,family}', fe3.value#>>'{name,family}') AS "value"
FROM resources res
CROSS JOIN LATERAL (SELECT res.data #> '{}' AS data, res.id AS id OFFSET 0) r
JOIN LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(r.data->'contact') = 'array' THEN r.data->'contact' WHEN jsonb_typeof(r.data->'contact') IS NOT NULL THEN jsonb_build_array(r.data->'contact') ELSE '[]'::jsonb END)) WITH ORDINALITY AS fe3(value, ordinality) ON TRUE
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
    "id",
    "value"
  ],
  "decodes": [
    "Text",
    "Text"
  ],
  "client_limit": null
}
*/
