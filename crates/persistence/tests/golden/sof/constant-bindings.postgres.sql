SELECT
  ($3)::text AS "text",
  (($4)::bigint)::text AS "integer",
  (($5)::numeric)::text AS "decimal",
  CASE WHEN ($6)::boolean IS TRUE THEN 'true' WHEN ($6)::boolean IS FALSE THEN 'false' END AS "boolean",
  'resources r' AS "literal"
FROM resources r
WHERE r.tenant_id = $1
  AND r.resource_type = $2
  AND r.is_deleted = false
  AND r.last_updated >= $7
  AND r.id = ANY($8::text[])
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
    },
    {
      "type": "string",
      "value": "0123"
    },
    {
      "type": "integer",
      "value": 42
    },
    {
      "type": "decimal",
      "value": "1.25"
    },
    {
      "type": "boolean",
      "value": true
    },
    {
      "type": "timestamp",
      "value": "2024-01-01T00:00:00+00:00"
    },
    {
      "type": "text-list",
      "value": [
        "p1"
      ]
    }
  ],
  "columns": [
    "text",
    "integer",
    "decimal",
    "boolean",
    "literal"
  ],
  "decodes": [
    "Text",
    "Integer",
    "Decimal",
    "Boolean",
    "Text"
  ],
  "client_limit": null
}
*/
