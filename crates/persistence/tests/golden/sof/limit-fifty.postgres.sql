SELECT
  r.data->>'id' AS "id",
  CASE WHEN (r.data->>'active')::boolean IS TRUE THEN 'true' WHEN (r.data->>'active')::boolean IS FALSE THEN 'false' END AS "active"
FROM resources r
WHERE r.tenant_id = $1
  AND r.resource_type = $2
  AND r.is_deleted = false
ORDER BY r.last_updated, r.id COLLATE "C"
LIMIT 50

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
