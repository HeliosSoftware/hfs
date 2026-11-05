SELECT
  (fe2.value)#>>'{}' AS "given"
FROM resources r
JOIN LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(r.data->'name') = 'array' THEN r.data->'name' WHEN jsonb_typeof(r.data->'name') IS NOT NULL THEN jsonb_build_array(r.data->'name') ELSE '[]'::jsonb END)) WITH ORDINALITY AS fe(value, ordinality) ON TRUE
JOIN LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(fe.value->'given') = 'array' THEN fe.value->'given' WHEN jsonb_typeof(fe.value->'given') IS NOT NULL THEN jsonb_build_array(fe.value->'given') ELSE '[]'::jsonb END)) WITH ORDINALITY AS fe2(value, ordinality) ON TRUE
WHERE r.tenant_id = $1
  AND r.resource_type = $2
  AND r.is_deleted = false
ORDER BY r.last_updated, r.id COLLATE "C", COALESCE(fe.ordinality, -1), COALESCE(fe2.ordinality, -1)

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
    "given"
  ],
  "decodes": [
    "Text"
  ],
  "client_limit": null
}
*/
