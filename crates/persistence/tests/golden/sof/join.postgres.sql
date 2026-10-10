SELECT
  ((SELECT string_agg((ja1.value #>> '{}'), '|' ORDER BY COALESCE(ja0.ordinality, -1), COALESCE(ja1.ordinality, -1)) FROM LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(r.data->'name') = 'array' THEN r.data->'name' WHEN jsonb_typeof(r.data->'name') IS NOT NULL THEN jsonb_build_array(r.data->'name') ELSE '[]'::jsonb END)) WITH ORDINALITY AS ja0(value, ordinality) JOIN LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(ja0.value->'given') = 'array' THEN ja0.value->'given' WHEN jsonb_typeof(ja0.value->'given') IS NOT NULL THEN jsonb_build_array(ja0.value->'given') ELSE '[]'::jsonb END)) WITH ORDINALITY AS ja1(value, ordinality) ON TRUE))::text AS "joined"
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
    "joined"
  ],
  "decodes": [
    "Text"
  ],
  "client_limit": null
}
*/
