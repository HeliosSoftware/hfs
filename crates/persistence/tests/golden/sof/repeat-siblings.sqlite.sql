WITH RECURSIVE rec_0(rid, node, ord_path, pkey) AS (
  SELECT r.id AS rid, je.value AS node, printf('%010d', je.key) AS ord_path, '' AS pkey
  FROM resources r
  JOIN json_each(CASE WHEN json_type(r.data, '$.item') = 'array' THEN json_extract(r.data, '$.item') WHEN json_type(r.data, '$.item') IN ('object', 'array') THEN json_array(json(json_extract(r.data, '$.item'))) WHEN json_type(r.data, '$.item') IS NOT NULL THEN json_array(json_extract(r.data, '$.item')) ELSE '[]' END) je ON 1=1
  WHERE r.tenant_id = ?1
  AND r.resource_type = ?2
  AND r.is_deleted = 0
  UNION ALL
  SELECT rec_0.rid, rs0.value AS node, rec_0.ord_path || '.' || printf('%010d', rs0.key) AS ord_path, rec_0.pkey
  FROM rec_0, json_each(CASE WHEN json_type(rec_0.node, '$.item') = 'array' THEN json_extract(rec_0.node, '$.item') WHEN json_type(rec_0.node, '$.item') IN ('object', 'array') THEN json_array(json(json_extract(rec_0.node, '$.item'))) WHEN json_type(rec_0.node, '$.item') IS NOT NULL THEN json_array(json_extract(rec_0.node, '$.item')) ELSE '[]' END) rs0
),
rec_1(rid, node, ord_path, pkey) AS (
  SELECT r.id AS rid, je.value AS node, printf('%010d', je.key) AS ord_path, '' AS pkey
  FROM resources r
  JOIN json_each(CASE WHEN json_type(r.data, '$.item') = 'array' THEN json_extract(r.data, '$.item') WHEN json_type(r.data, '$.item') IN ('object', 'array') THEN json_array(json(json_extract(r.data, '$.item'))) WHEN json_type(r.data, '$.item') IS NOT NULL THEN json_array(json_extract(r.data, '$.item')) ELSE '[]' END) je ON 1=1
  WHERE r.tenant_id = ?1
  AND r.resource_type = ?2
  AND r.is_deleted = 0
  UNION ALL
  SELECT rec_1.rid, rs0.value AS node, rec_1.ord_path || '.' || printf('%010d', rs0.key) AS ord_path, rec_1.pkey
  FROM rec_1, json_each(CASE WHEN json_type(rec_1.node, '$.item') = 'array' THEN json_extract(rec_1.node, '$.item') WHEN json_type(rec_1.node, '$.item') IN ('object', 'array') THEN json_array(json(json_extract(rec_1.node, '$.item'))) WHEN json_type(rec_1.node, '$.item') IS NOT NULL THEN json_array(json_extract(rec_1.node, '$.item')) ELSE '[]' END) rs0
)
SELECT
  json_extract(rec_0.node, '$.linkId') AS "a",
  json_extract(rec_1.node, '$.linkId') AS "b"
FROM resources r
JOIN rec_0 ON rec_0.rid = r.id AND rec_0.pkey = ''
JOIN rec_1 ON rec_1.rid = r.id AND rec_1.pkey = ''
WHERE r.tenant_id = ?1
  AND r.resource_type = ?2
  AND r.is_deleted = 0
ORDER BY 1

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
    "a",
    "b"
  ],
  "decodes": [
    "Text",
    "Text"
  ],
  "client_limit": null
}
*/
