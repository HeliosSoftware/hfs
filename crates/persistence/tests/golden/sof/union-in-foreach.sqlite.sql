SELECT
  json_extract(r.data, '$.id') AS "id",
  json_extract(fe2.value, '$.value') AS "value"
FROM resources r
JOIN json_each(r.data, '$.contact') fe ON 1=1
JOIN json_each(fe.value, '$.telecom') fe2 ON 1=1
WHERE r.tenant_id = ?1
  AND r.resource_type = ?2
  AND r.is_deleted = 0
UNION ALL
SELECT
  json_extract(r.data, '$.id') AS "id",
  coalesce(json_extract(fe3.value, '$.name[0].family'), json_extract(fe3.value, '$.name.family')) AS "value"
FROM resources r
JOIN json_each(r.data, '$.contact') fe3 ON 1=1
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
    "id",
    "value"
  ],
  "decodes": [
    "Text",
    "Text"
  ],
  "client_limit": null
}
*/
