SELECT
  ((SELECT coalesce(jsonb_agg(ca0.value ORDER BY COALESCE(ca0.ordinality, -1)), '[]'::jsonb) FROM LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(fe.value->'telecom') = 'array' THEN fe.value->'telecom' WHEN jsonb_typeof(fe.value->'telecom') IS NOT NULL THEN jsonb_build_array(fe.value->'telecom') ELSE '[]'::jsonb END)) WITH ORDINALITY AS ca0(value, ordinality)))::text AS "objects",
  ((SELECT coalesce(jsonb_agg(ca1.value ORDER BY COALESCE(ca0.ordinality, -1), COALESCE(ca1.ordinality, -1)), '[]'::jsonb) FROM LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(fe.value->'telecom') = 'array' THEN fe.value->'telecom' WHEN jsonb_typeof(fe.value->'telecom') IS NOT NULL THEN jsonb_build_array(fe.value->'telecom') ELSE '[]'::jsonb END)) WITH ORDINALITY AS ca0(value, ordinality) JOIN LATERAL jsonb_array_elements(CASE WHEN jsonb_typeof(ca0.value->'value') = 'array' THEN ca0.value->'value' ELSE jsonb_build_array(ca0.value->'value') END) WITH ORDINALITY AS ca1(value, ordinality) ON TRUE WHERE ca0.value->'value' IS NOT NULL))::text AS "values",
  ((SELECT string_agg((ja1.value #>> '{}'), '|' ORDER BY COALESCE(ja0.ordinality, -1), COALESCE(ja1.ordinality, -1)) FROM LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(fe.value->'telecom') = 'array' THEN fe.value->'telecom' WHEN jsonb_typeof(fe.value->'telecom') IS NOT NULL THEN jsonb_build_array(fe.value->'telecom') ELSE '[]'::jsonb END)) WITH ORDINALITY AS ja0(value, ordinality) JOIN LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(ja0.value->'value') = 'array' THEN ja0.value->'value' WHEN jsonb_typeof(ja0.value->'value') IS NOT NULL THEN jsonb_build_array(ja0.value->'value') ELSE '[]'::jsonb END)) WITH ORDINALITY AS ja1(value, ordinality) ON TRUE))::text AS "joined",
  ((SELECT w2.value->>'value' FROM LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(fe.value->'telecom') = 'array' THEN fe.value->'telecom' WHEN jsonb_typeof(fe.value->'telecom') IS NOT NULL THEN jsonb_build_array(fe.value->'telecom') ELSE '[]'::jsonb END)) WITH ORDINALITY AS w2(value, ordinality) WHERE (w2.value->>'system' = 'phone') ORDER BY COALESCE(w2.ordinality, -1) LIMIT 1))::text AS "pick"
FROM resources r
JOIN LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(r.data->'contact') = 'array' THEN r.data->'contact' WHEN jsonb_typeof(r.data->'contact') IS NOT NULL THEN jsonb_build_array(r.data->'contact') ELSE '[]'::jsonb END)) WITH ORDINALITY AS fe(value, ordinality) ON TRUE
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
      "value": "Patient"
    }
  ],
  "columns": [
    "objects",
    "values",
    "joined",
    "pick"
  ],
  "decodes": [
    "Json",
    "Json",
    "Text",
    "Text"
  ],
  "client_limit": null
}
*/
