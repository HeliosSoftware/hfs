SELECT
  CAST(?3 AS TEXT) AS "value"
FROM resources r
WHERE r.tenant_id = ?1
  AND r.resource_type = ?2
  AND r.is_deleted = 0
  AND r.last_updated >= ?5
  AND r.id IN (SELECT value FROM json_each(?6))
UNION ALL
SELECT
  CAST(?4 AS TEXT) AS "value"
FROM resources r
WHERE r.tenant_id = ?1
  AND r.resource_type = ?2
  AND r.is_deleted = 0
  AND r.last_updated >= ?5
  AND r.id IN (SELECT value FROM json_each(?6))
ORDER BY 1
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
