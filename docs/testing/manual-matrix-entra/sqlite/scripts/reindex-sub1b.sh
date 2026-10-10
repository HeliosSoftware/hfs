#!/bin/bash
# waits for the automatic reindex (submission 2 types) to finish, then reindexes the 16 types written by submission 1, one at a time, with the service token
source /home/angela/pass-1182/env.sh >/dev/null 2>&1; unset CURL_HOME; cd $WORK; out=run/reindex-sub1.log
# automatic reindex already finished (see above)
TYPES="ExplanationOfBenefit ImagingStudy Immunization Location Medication MedicationAdministration MedicationRequest"
T0=$(date +%s); echo "## submission-1 types reindex resume (after HFS restart) $(date -u +%FT%TZ)" >> $out; tmint=0
for ty in $TYPES; do
  now=$(date +%s); [ $((now-tmint)) -gt 3000 ] && { T=$(etok hfs); tmint=$now; }
  t1=$(date +%s); J=$(curl -s -X POST -H "Authorization: Bearer $T" -H 'Content-Type: application/fhir+json' -d '{"resourceType":"Parameters"}' "$HFS/$ty/\$reindex" | python3 -c 'import sys,json;d=json.load(sys.stdin);print(next((p.get("valueString") for p in d.get("parameter",[]) if p["name"]=="jobId"),"ERR "+json.dumps(d)[:200]))')
  while :; do now=$(date +%s); [ $((now-tmint)) -gt 3000 ] && { T=$(etok hfs); tmint=$now; }
    st=$(curl -s -H "Authorization: Bearer $T" "$HFS/\$reindex-status/$J" | python3 -c 'import sys,json
d=json.load(sys.stdin); p={x["name"]:(x.get("valueString") or x.get("valueInteger") or x.get("valueCode")) for x in d.get("parameter",[])}; print(p.get("status"),p.get("processed"),p.get("total"),p.get("entriesCreated"),p.get("errorCount"))' 2>/dev/null)
    case "$st" in completed*|failed*|cancelled*) break;; esac; sleep 20; done
  echo "$(date -u +%FT%TZ) $ty job ${J:0:8}… $st elapsed $(( $(date +%s)-t1 )) s" >> $out
done
echo "## submission-1 types reindex end $(date -u +%FT%TZ) total $(( $(date +%s)-T0 )) s" >> $out
