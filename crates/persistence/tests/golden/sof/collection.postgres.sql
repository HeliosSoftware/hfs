SELECT
  ((SELECT coalesce(jsonb_agg(ca0.value ORDER BY COALESCE(ca0.ordinality, -1)), '[]'::jsonb) FROM LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(r.data->'name') = 'array' THEN r.data->'name' WHEN jsonb_typeof(r.data->'name') IS NOT NULL THEN jsonb_build_array(r.data->'name') ELSE '[]'::jsonb END)) WITH ORDINALITY AS ca0(value, ordinality)))::text AS "objects",
  ((SELECT coalesce(jsonb_agg(ca1.value ORDER BY COALESCE(ca0.ordinality, -1), COALESCE(ca1.ordinality, -1)), '[]'::jsonb) FROM LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(r.data->'name') = 'array' THEN r.data->'name' WHEN jsonb_typeof(r.data->'name') IS NOT NULL THEN jsonb_build_array(r.data->'name') ELSE '[]'::jsonb END)) WITH ORDINALITY AS ca0(value, ordinality) JOIN LATERAL jsonb_array_elements(CASE WHEN jsonb_typeof(ca0.value->'family') = 'array' THEN ca0.value->'family' ELSE jsonb_build_array(ca0.value->'family') END) WITH ORDINALITY AS ca1(value, ordinality) ON TRUE WHERE ca0.value->'family' IS NOT NULL))::text AS "families",
  ((SELECT coalesce(jsonb_agg(ca1.value ORDER BY COALESCE(ca0.ordinality, -1), COALESCE(ca1.ordinality, -1)), '[]'::jsonb) FROM LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(r.data->'name') = 'array' THEN r.data->'name' WHEN jsonb_typeof(r.data->'name') IS NOT NULL THEN jsonb_build_array(r.data->'name') ELSE '[]'::jsonb END)) WITH ORDINALITY AS ca0(value, ordinality) JOIN LATERAL jsonb_array_elements(CASE WHEN jsonb_typeof(ca0.value->'given') = 'array' THEN ca0.value->'given' ELSE jsonb_build_array(ca0.value->'given') END) WITH ORDINALITY AS ca1(value, ordinality) ON TRUE WHERE ca0.value->'given' IS NOT NULL))::text AS "givens"
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
    "objects",
    "families",
    "givens"
  ],
  "decodes": [
    "Json",
    "Json",
    "Json"
  ],
  "client_limit": null
}
*/
