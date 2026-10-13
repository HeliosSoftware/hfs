SELECT
  rdoc.doc->>'id' AS "id",
  fe2.value->>'value' AS "value"
FROM resources r
CROSS JOIN LATERAL jsonb_extract_path(r.data, VARIADIC '{}'::text[]) AS rdoc(doc)
JOIN LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(COALESCE(rdoc.doc, r.data)->'contact') = 'array' THEN COALESCE(rdoc.doc, r.data)->'contact' WHEN jsonb_typeof(COALESCE(rdoc.doc, r.data)->'contact') IS NOT NULL THEN jsonb_build_array(COALESCE(rdoc.doc, r.data)->'contact') ELSE '[]'::jsonb END)) WITH ORDINALITY AS fe(value, ordinality) ON TRUE
JOIN LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(fe.value->'telecom') = 'array' THEN fe.value->'telecom' WHEN jsonb_typeof(fe.value->'telecom') IS NOT NULL THEN jsonb_build_array(fe.value->'telecom') ELSE '[]'::jsonb END)) WITH ORDINALITY AS fe2(value, ordinality) ON TRUE
WHERE r.tenant_id = $1
  AND r.resource_type = $2
  AND r.is_deleted = false
UNION ALL
SELECT
  rdoc.doc->>'id' AS "id",
  coalesce(fe3.value#>>'{name,0,family}', fe3.value#>>'{name,family}') AS "value"
FROM resources r
CROSS JOIN LATERAL jsonb_extract_path(r.data, VARIADIC '{}'::text[]) AS rdoc(doc)
JOIN LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(COALESCE(rdoc.doc, r.data)->'contact') = 'array' THEN COALESCE(rdoc.doc, r.data)->'contact' WHEN jsonb_typeof(COALESCE(rdoc.doc, r.data)->'contact') IS NOT NULL THEN jsonb_build_array(COALESCE(rdoc.doc, r.data)->'contact') ELSE '[]'::jsonb END)) WITH ORDINALITY AS fe3(value, ordinality) ON TRUE
WHERE r.tenant_id = $1
  AND r.resource_type = $2
  AND r.is_deleted = false
ORDER BY 1

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
