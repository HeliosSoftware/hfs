SELECT
  fe.value->>'family' AS "family",
  fe2.value->>'city' AS "city"
FROM resources r
CROSS JOIN LATERAL jsonb_extract_path(r.data, VARIADIC '{}'::text[]) AS rdoc(doc)
JOIN LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(COALESCE(rdoc.doc, r.data)->'name') = 'array' THEN COALESCE(rdoc.doc, r.data)->'name' WHEN jsonb_typeof(COALESCE(rdoc.doc, r.data)->'name') IS NOT NULL THEN jsonb_build_array(COALESCE(rdoc.doc, r.data)->'name') ELSE '[]'::jsonb END)) WITH ORDINALITY AS fe(value, ordinality) ON TRUE
JOIN LATERAL jsonb_array_elements((CASE WHEN jsonb_typeof(COALESCE(rdoc.doc, r.data)->'address') = 'array' THEN COALESCE(rdoc.doc, r.data)->'address' WHEN jsonb_typeof(COALESCE(rdoc.doc, r.data)->'address') IS NOT NULL THEN jsonb_build_array(COALESCE(rdoc.doc, r.data)->'address') ELSE '[]'::jsonb END)) WITH ORDINALITY AS fe2(value, ordinality) ON TRUE
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
    "family",
    "city"
  ],
  "decodes": [
    "Text",
    "Text"
  ],
  "client_limit": null
}
*/
