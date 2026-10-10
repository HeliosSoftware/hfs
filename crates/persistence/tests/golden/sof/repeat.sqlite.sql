WITH RECURSIVE rec_0(rid, node, ord_path) AS (
  SELECT r.id AS rid, je.value AS node, printf('%010d', je.key) AS ord_path
  FROM resources r, json_each(r.data, '$.item') je
  WHERE r.tenant_id = ?1
  AND r.resource_type = ?2
  AND r.is_deleted = 0
  UNION ALL
  SELECT r.id AS rid, je.value AS node, printf('%010d', je.key) AS ord_path
  FROM resources r, json_each(r.data, '$.answer.item') je
  WHERE r.tenant_id = ?1
  AND r.resource_type = ?2
  AND r.is_deleted = 0
  UNION ALL
  SELECT rec_0.rid, rs0.value AS node, rec_0.ord_path || '.' || printf('%010d', rs0.key) AS ord_path
  FROM rec_0, json_each(CASE WHEN json_type(rec_0.node, '$.item') = 'array' THEN json_extract(rec_0.node, '$.item') WHEN json_type(rec_0.node, '$.item') IN ('object', 'array') THEN json_array(json(json_extract(rec_0.node, '$.item'))) WHEN json_type(rec_0.node, '$.item') IS NOT NULL THEN json_array(json_extract(rec_0.node, '$.item')) ELSE '[]' END) rs0
  UNION ALL
  SELECT rec_0.rid, rs1.value AS node, rec_0.ord_path || '.' || printf('%010d', rs1.key) AS ord_path
  FROM rec_0, json_each(CASE WHEN json_type(rec_0.node, '$.answer') = 'array' THEN json_extract(rec_0.node, '$.answer') WHEN json_type(rec_0.node, '$.answer') IN ('object', 'array') THEN json_array(json(json_extract(rec_0.node, '$.answer'))) WHEN json_type(rec_0.node, '$.answer') IS NOT NULL THEN json_array(json_extract(rec_0.node, '$.answer')) ELSE '[]' END) rs0, json_each(CASE WHEN json_type(rs0.value, '$.item') = 'array' THEN json_extract(rs0.value, '$.item') WHEN json_type(rs0.value, '$.item') IN ('object', 'array') THEN json_array(json(json_extract(rs0.value, '$.item'))) WHEN json_type(rs0.value, '$.item') IS NOT NULL THEN json_array(json_extract(rs0.value, '$.item')) ELSE '[]' END) rs1
)
SELECT
  json_extract(r.data, '$.id') AS "id",
  CAST((SELECT json_group_array(ca1.value ORDER BY COALESCE(ca0.rowid, -1), COALESCE(ca1.rowid, -1)) FROM json_each(r.data, '$.item') ca0, json_each(CASE WHEN json_type(ca0.value, '$.linkId') = 'array' THEN json_extract(ca0.value, '$.linkId') ELSE json_array(json_extract(ca0.value, '$.linkId')) END) ca1 WHERE json_type(ca0.value, '$.linkId') IS NOT NULL) AS TEXT) AS "roots",
  json_extract(rec_0.node, '$.linkId') AS "link",
  CAST((SELECT json_group_array(ca1.value ORDER BY COALESCE(ca0.rowid, -1), COALESCE(ca1.rowid, -1)) FROM json_each(CASE WHEN json_type(rec_0.node, '$.answer') = 'array' THEN json_extract(rec_0.node, '$.answer') WHEN json_type(rec_0.node, '$.answer') IN ('object', 'array') THEN json_array(json(json_extract(rec_0.node, '$.answer'))) WHEN json_type(rec_0.node, '$.answer') IS NOT NULL THEN json_array(json_extract(rec_0.node, '$.answer')) ELSE '[]' END) ca0, json_each(CASE WHEN json_type(ca0.value, '$.valueString') = 'array' THEN json_extract(ca0.value, '$.valueString') ELSE json_array(json_extract(ca0.value, '$.valueString')) END) ca1 WHERE json_type(ca0.value, '$.valueString') IS NOT NULL) AS TEXT) AS "answers",
  CAST((SELECT group_concat(ja1.value, '|' ORDER BY COALESCE(ja0.rowid, -1), COALESCE(ja1.rowid, -1)) FROM json_each(CASE WHEN json_type(rec_0.node, '$.answer') = 'array' THEN json_extract(rec_0.node, '$.answer') WHEN json_type(rec_0.node, '$.answer') IN ('object', 'array') THEN json_array(json(json_extract(rec_0.node, '$.answer'))) WHEN json_type(rec_0.node, '$.answer') IS NOT NULL THEN json_array(json_extract(rec_0.node, '$.answer')) ELSE '[]' END) ja0, json_each(CASE WHEN json_type(ja0.value, '$.valueString') = 'array' THEN json_extract(ja0.value, '$.valueString') WHEN json_type(ja0.value, '$.valueString') IN ('object', 'array') THEN json_array(json(json_extract(ja0.value, '$.valueString'))) WHEN json_type(ja0.value, '$.valueString') IS NOT NULL THEN json_array(json_extract(ja0.value, '$.valueString')) ELSE '[]' END) ja1) AS TEXT) AS "joined",
  CAST((SELECT json_extract(w2.value, '$.valueString') FROM json_each(CASE WHEN json_type(rec_0.node, '$.answer') = 'array' THEN json_extract(rec_0.node, '$.answer') WHEN json_type(rec_0.node, '$.answer') IN ('object', 'array') THEN json_array(json(json_extract(rec_0.node, '$.answer'))) WHEN json_type(rec_0.node, '$.answer') IS NOT NULL THEN json_array(json_extract(rec_0.node, '$.answer')) ELSE '[]' END) w2 WHERE (json_extract(w2.value, '$.valueString') != 'skip') ORDER BY COALESCE(w2.rowid, -1) LIMIT 1) AS TEXT) AS "picked"
FROM rec_0 JOIN resources r ON r.id = rec_0.rid AND r.tenant_id = ?1
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
      "value": "QuestionnaireResponse"
    }
  ],
  "columns": [
    "id",
    "roots",
    "link",
    "answers",
    "joined",
    "picked"
  ],
  "decodes": [
    "Text",
    "Json",
    "Auto",
    "Json",
    "Text",
    "Text"
  ],
  "client_limit": null
}
*/
