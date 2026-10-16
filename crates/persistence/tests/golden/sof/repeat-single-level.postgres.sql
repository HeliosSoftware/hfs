SELECT
  (r.id)::text AS "observation_id",
  coalesce(fe.value#>>'{code,0,text}', fe.value#>>'{code,text}') AS "component_name",
  fe.value#>>'{code,coding,0,code}' AS "component_code"
FROM resources r
JOIN LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(r.data->'component') = 'array' THEN r.data->'component' WHEN jsonb_typeof(r.data->'component') IS NOT NULL THEN jsonb_build_array(r.data->'component') ELSE '[]'::jsonb END)) WITH ORDINALITY AS fe(value, ordinality) ON TRUE
WHERE r.tenant_id = $1
  AND r.resource_type = $2
  AND r.is_deleted = false
ORDER BY r.last_updated, r.id COLLATE "C", COALESCE(fe.ordinality, -1)

/* Runner bindings and output metadata:
{
  "bindings": [
    {
      "type": "text",
      "value": "golden-tenant"
    },
    {
      "type": "text",
      "value": "Observation"
    }
  ],
  "columns": [
    "observation_id",
    "component_name",
    "component_code"
  ],
  "decodes": [
    "Text",
    "Text",
    "Text"
  ],
  "client_limit": null
}
*/
