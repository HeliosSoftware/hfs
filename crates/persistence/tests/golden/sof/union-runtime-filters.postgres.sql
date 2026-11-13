SELECT
  ($3)::text AS "value"
FROM resources res
CROSS JOIN LATERAL (SELECT res.data #> '{}' AS data, res.id AS id OFFSET 0) r
WHERE res.tenant_id = $1
  AND res.resource_type = $2
  AND res.is_deleted = false
  AND res.last_updated >= $5
  AND res.id = ANY($6::text[])
UNION ALL
SELECT
  ($4)::text AS "value"
FROM resources res
CROSS JOIN LATERAL (SELECT res.data #> '{}' AS data, res.id AS id OFFSET 0) r
WHERE res.tenant_id = $1
  AND res.resource_type = $2
  AND res.is_deleted = false
  AND res.last_updated >= $5
  AND res.id = ANY($6::text[])

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
      "type": "string",
      "value": "A"
    },
    {
      "type": "string",
      "value": "B"
    },
    {
      "type": "timestamp",
      "value": "2024-01-01T00:00:00+00:00"
    },
    {
      "type": "text-list",
      "value": [
        "explicit",
        "p1",
        "p2"
      ]
    }
  ],
  "columns": [
    "value"
  ],
  "decodes": [
    "Auto"
  ],
  "client_limit": 50
}
*/
