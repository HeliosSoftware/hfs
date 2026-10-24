SELECT
  r.data->>'id' AS "id",
  (split_part(coalesce(r.data#>>'{subject,0,reference}', r.data#>>'{subject,reference}'), '/', -1))::text AS "any_key",
  (CASE WHEN coalesce(r.data#>>'{subject,0,reference}', r.data#>>'{subject,reference}') LIKE 'Patient/%' OR coalesce(r.data#>>'{subject,0,reference}', r.data#>>'{subject,reference}') LIKE '%/Patient/%' THEN split_part(coalesce(r.data#>>'{subject,0,reference}', r.data#>>'{subject,reference}'), '/', -1) ELSE NULL END)::text AS "patient_key"
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
      "value": "Observation"
    }
  ],
  "columns": [
    "id",
    "any_key",
    "patient_key"
  ],
  "decodes": [
    "Text",
    "Text",
    "Text"
  ],
  "client_limit": null
}
*/
