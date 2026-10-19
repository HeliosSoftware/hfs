SELECT
  json_extract(r.data, '$.id') AS "id",
  CAST(fhir_last_segment(coalesce(json_extract(r.data, '$.subject[0].reference'), json_extract(r.data, '$.subject.reference'))) AS TEXT) AS "any_key",
  CAST(CASE WHEN coalesce(json_extract(r.data, '$.subject[0].reference'), json_extract(r.data, '$.subject.reference')) LIKE 'Patient/%' OR coalesce(json_extract(r.data, '$.subject[0].reference'), json_extract(r.data, '$.subject.reference')) LIKE '%/Patient/%' THEN fhir_last_segment(coalesce(json_extract(r.data, '$.subject[0].reference'), json_extract(r.data, '$.subject.reference'))) ELSE NULL END AS TEXT) AS "patient_key"
FROM resources r
WHERE r.tenant_id = ?1
  AND r.resource_type = ?2
  AND r.is_deleted = 0
ORDER BY r.last_updated, r.id COLLATE BINARY

/* Runner bindings and output metadata:
{
  "bindings": [
    {
      "type": "text",
      "value": "golden-tenant"
    },
    {
      "type": "text",
      "value": "Observation"
    }
  ],
  "columns": [
    "id",
    "any_key",
    "patient_key"
  ],
  "decodes": [
    "Text",
    "Text",
    "Text"
  ],
  "client_limit": null
}
*/
