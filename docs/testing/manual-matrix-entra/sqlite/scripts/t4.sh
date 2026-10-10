#!/bin/bash
# T4: fixtures saved as Angela (Resource Editor PUT with her session cookie), then the section-8 searches via the API (service token), as #1170 did
source /home/angela/pass-1182/env.sh; unset CURL_HOME; cd $WORK; . run/ids.env; S=$(cat private/session.id)
echo "## T4 fixtures (Resource Editor PUT as Angela) $(date -u +%FT%TZ)"
for f in risk:RiskAssessment:manual-risk valueset:ValueSet:manual-test-vs group:Group:manual-group; do IFS=: read fx ty id <<<"$f"
  printf 'PUT /%s/%s -> ' $ty $id; curl -s -o /tmp/p.out -w '%{http_code} ' -X PUT -b "hfs_session=$S" -H 'Sec-Fetch-Site: same-origin' -H 'Content-Type: application/fhir+json' -H 'Accept: application/fhir+json' --data-binary @fixtures/$fx.json "$HFS/$ty/$id"; python3 -c 'import json;d=json.load(open("/tmp/p.out"));print(d.get("resourceType"),d.get("id"),"v"+str(d.get("meta",{}).get("versionId")))'; done
echo "## T4 searches $(date -u +%FT%TZ) (LPID=$LPID T3START=$T3START)"
export TOKEN=$(etok hfs) HFS LPID T3START
python3 t4.py t4-queries.tsv; python3 t4.py t4-lpid.tsv
