SELECT
  r.data->>'id' AS "id",
  (split_part(coalesce(r.data#>>'{subject,0,reference}', r.data#>>'{subject,reference}'), '/', -1))::text AS "any_key",
  (CASE WHEN coalesce(r.data#>>'{subject,0,reference}', r.data#>>'{subject,reference}') LIKE 'Patient/%' OR coalesce(r.data#>>'{subject,0,reference}', r.data#>>'{subject,reference}') LIKE '%/Patient/%' THEN split_part(coalesce(r.data#>>'{subject,0,reference}', r.data#>>'{subject,reference}'), '/', -1) ELSE NULL END)::text AS "patient_key"
FROM resources res
CROSS JOIN LATERAL (SELECT res.data #> '{}' AS data, res.id AS id OFFSET 0) r
WHERE res.tenant_id = $1
  AND res.resource_type = $2
  AND res.is_deleted = false
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
