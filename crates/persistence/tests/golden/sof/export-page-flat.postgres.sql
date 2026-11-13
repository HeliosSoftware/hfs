SELECT
  r.data->>'id' AS "id",
  CASE WHEN (r.data->>'active')::boolean IS TRUE THEN 'true' WHEN (r.data->>'active')::boolean IS FALSE THEN 'false' END AS "active"
FROM resources res
CROSS JOIN LATERAL (SELECT res.data #> '{}' AS data, res.id AS id OFFSET 0) r
WHERE res.tenant_id = $1
  AND res.resource_type = $2
  AND res.is_deleted = false
  AND res.last_updated >= $3
  AND res.id COLLATE "C" > $4
  AND res.id COLLATE "C" <= $5
ORDER BY res.id COLLATE "C"

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
      "type": "timestamp",
      "value": "2024-01-01T00:00:00+00:00"
    },
    {
      "type": "text",
      "value": ""
    },
    {
      "type": "text",
      "value": ""
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
  "client_limit": 50
}
*/
