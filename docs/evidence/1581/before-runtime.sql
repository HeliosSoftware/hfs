SELECT
  r.data->>'id' AS "id",
  (CASE WHEN coalesce(r.data#>>'{subject,0,reference}', r.data#>>'{subject,reference}') LIKE 'Patient/%' OR coalesce(r.data#>>'{subject,0,reference}', r.data#>>'{subject,reference}') LIKE '%/Patient/%' THEN regexp_replace(coalesce(r.data#>>'{subject,0,reference}', r.data#>>'{subject,reference}'), '.*/', '') ELSE NULL END)::text AS "patient_id",
  r.data#>>'{code,coding,0,code}' AS "code",
  ((coalesce(r.data#>>'{valueQuantity,0,value}', r.data#>>'{valueQuantity,value}'))::numeric)::text AS "value",
  r.data->>'effectiveDateTime' AS "effective"
FROM resources r
WHERE r.tenant_id = $1
  AND r.resource_type = $2
  AND r.is_deleted = false
ORDER BY r.last_updated, r.id;
