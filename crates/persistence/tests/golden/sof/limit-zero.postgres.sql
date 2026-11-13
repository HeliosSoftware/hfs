SELECT
  r.data->>'id' AS "id",
  CASE WHEN (r.data->>'active')::boolean IS TRUE THEN 'true' WHEN (r.data->>'active')::boolean IS FALSE THEN 'false' END AS "active"
FROM resources res
CROSS JOIN LATERAL (SELECT res.data #> '{}' AS data, res.id AS id OFFSET 0) r
WHERE res.tenant_id = $1
  AND res.resource_type = $2
  AND res.is_deleted = false
ORDER BY res.last_updated, res.id COLLATE "C"
LIMIT 0

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
    "active"
  ],
  "decodes": [
    "Text",
    "Boolean"
  ],
  "client_limit": null
}
*/
