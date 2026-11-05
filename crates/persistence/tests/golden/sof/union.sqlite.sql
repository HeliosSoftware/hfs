SELECT
  json_extract(CAST(r.data AS TEXT), '$.id') AS "value"
FROM resources r
WHERE r.tenant_id = ?1
  AND r.resource_type = ?2
  AND r.is_deleted = 0
UNION ALL
SELECT
  json_extract(fe.value, '$.family') AS "value"
FROM resources r
JOIN json_each(CAST(r.data AS TEXT), '$.name') fe ON 1=1
WHERE r.tenant_id = ?1
  AND r.resource_type = ?2
  AND r.is_deleted = 0

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
    "value"
  ],
  "decodes": [
    "Text"
  ],
  "client_limit": null
}
*/
