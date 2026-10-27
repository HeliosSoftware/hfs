WITH RECURSIVE rec_1(rid, node, ord_path, pkey) AS (
  SELECT r.id AS rid, je.value AS node, printf('%010d', je.key) AS ord_path, '' || '/' || printf('%010d', COALESCE(fe.rowid, -1)) AS pkey
  FROM resources r
JOIN json_each(CAST(r.data AS TEXT), '$.item') fe ON 1=1
  JOIN json_each(CASE WHEN json_type(fe.value, '$.item') = 'array' THEN json_extract(fe.value, '$.item') WHEN json_type(fe.value, '$.item') IN ('object', 'array') THEN json_array(json(json_extract(fe.value, '$.item'))) WHEN json_type(fe.value, '$.item') IS NOT NULL THEN json_array(json_extract(fe.value, '$.item')) ELSE '[]' END) je ON 1=1
  WHERE r.tenant_id = ?1
  AND r.resource_type = ?2
  AND r.is_deleted = 0
  UNION ALL
  SELECT rec_1.rid, rs0.value AS node, rec_1.ord_path || '.' || printf('%010d', rs0.key) AS ord_path, rec_1.pkey
  FROM rec_1, json_each(CASE WHEN json_type(rec_1.node, '$.item') = 'array' THEN json_extract(rec_1.node, '$.item') WHEN json_type(rec_1.node, '$.item') IN ('object', 'array') THEN json_array(json(json_extract(rec_1.node, '$.item'))) WHEN json_type(rec_1.node, '$.item') IS NOT NULL THEN json_array(json_extract(rec_1.node, '$.item')) ELSE '[]' END) rs0
)
SELECT
  json_extract(CAST(r.data AS TEXT), '$.id') AS "id",
  json_extract(fe.value, '$.linkId') AS "top",
  json_extract(rec_1.node, '$.linkId') AS "child"
FROM resources r
JOIN json_each(CAST(r.data AS TEXT), '$.item') fe ON 1=1
JOIN rec_1 ON rec_1.rid = r.id AND rec_1.pkey = '' || '/' || printf('%010d', COALESCE(fe.rowid, -1))
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
      "value": "Questionnaire"
    }
  ],
  "columns": [
    "id",
    "top",
    "child"
  ],
  "decodes": [
    "Text",
    "Text",
    "Text"
  ],
  "client_limit": null
}
*/
