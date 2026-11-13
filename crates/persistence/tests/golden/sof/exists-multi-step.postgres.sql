SELECT
  r.data->>'id' AS "id"
FROM resources res
CROSS JOIN LATERAL (SELECT res.data #> '{}' AS data, res.id AS id OFFSET 0) r
WHERE res.tenant_id = $1
  AND res.resource_type = $2
  AND res.is_deleted = false
  AND (EXISTS (SELECT 1 FROM LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(r.data->'contact') = 'array' THEN r.data->'contact' WHEN jsonb_typeof(r.data->'contact') IS NOT NULL THEN jsonb_build_array(r.data->'contact') ELSE '[]'::jsonb END)) AS w0_0(value), LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(w0_0.value->'telecom') = 'array' THEN w0_0.value->'telecom' WHEN jsonb_typeof(w0_0.value->'telecom') IS NOT NULL THEN jsonb_build_array(w0_0.value->'telecom') ELSE '[]'::jsonb END)) AS w0(value) WHERE (((w0.value->>'system' = 'phone'))::boolean AND ((w0.value->>'value' = '555'))::boolean)))::boolean IS TRUE
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
    "id"
  ],
  "decodes": [
    "Text"
  ],
  "client_limit": null
}
*/
