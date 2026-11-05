SELECT
  r.data->>'id' AS "id",
  CASE WHEN (r.data->>'active')::boolean IS TRUE THEN 'true' WHEN (r.data->>'active')::boolean IS FALSE THEN 'false' END AS "active"
FROM resources r
WHERE r.tenant_id = $1
  AND r.resource_type = $2
  AND r.is_deleted = false
  AND r.id = ANY($3::text[])
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
    },
    {
      "type": "text-list",
      "value": [
        "p2",
        "p1"
      ]
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
