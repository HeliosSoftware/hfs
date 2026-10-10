#!/bin/bash
# T5 as Angela: POST /ui/bulk-export (session cookie), card fragment polled like the browser, ZIP download, line counts per type
source /home/angela/pass-1182/env.sh; unset CURL_HOME; cd $WORK; . run/ids.env; S=$(cat private/session.id); PID=7d24f7a0-6f2e-ce3b-5568-db7b14695583
H=(-b "hfs_session=$S" -H 'Sec-Fetch-Site: same-origin')
ids(){ curl -s "${H[@]}" "$HFS/ui/bulk-export" | grep -o '/ui/bulk-export/active/[0-9a-f-]\{36\}' | sort -u | cut -d/ -f5; }
card(){ curl -s "${H[@]}" -H 'HX-Request: true' "$HFS/ui/bulk-export/active/$1/card" | python3 -c 'import sys,re,html; t=html.unescape(re.sub(r"\s+"," ",re.sub(r"<[^>]+>"," ",sys.stdin.read()))); print(t.strip()[:260])'; }
start(){ local name=$1; shift; local b=$(ids); local code=$(curl -s -o /tmp/k5.html -w '%{http_code}' -X POST "${H[@]}" --data-urlencode "name=$name" "$@" "$HFS/ui/bulk-export"); J=$(comm -13 <(echo "$b") <(ids) | head -1); echo "  POST -> $code job ${J:-<none>}"; }
wait_done(){ for i in $(seq 1 ${2:-360}); do c=$(card $1); echo "$c" | grep -qE 'Complete|Failed|Cancelled' && break; sleep 5; done; echo "  card: $c"; }
zipcount(){ curl -s "${H[@]}" -o /tmp/e.zip "$HFS/ui/bulk-export/active/$1/download"; python3 - "$2" <<'PY'
import zipfile,sys,json,collections
z=zipfile.ZipFile('/tmp/e.zip'); per=collections.Counter(); chk=sys.argv[1]
for n in z.namelist():
    lines=[l for l in z.read(n).decode().splitlines() if l.strip()]; per[n.split('-')[0].split('.')[0]]+=len(lines)
    if chk=='elements' and lines: print('   sample keys',n,sorted(json.loads(lines[0]).keys()),json.loads(lines[0]).get('meta',{}).get('tag'))
    if chk=='active' and n.startswith('Condition'): print('   Condition clinicalStatus set',{c['code'] for l in lines for c in json.loads(l)['clinicalStatus']['coding']},'subjects',{json.loads(l)['subject']['reference'] for l in lines})
print('   files',len(z.namelist()),dict(per))
PY
}
echo "## T5 $(date -u +%FT%TZ) (T3 start $T3START, T3 end $T3END)"
echo "5.1 everything-small"; start everything-small --data scope=system --data types=Organization --data types=Practitioner --data types=Location; J51=$J; wait_done $J; zipcount $J
echo "5.2 one-patient"; start one-patient --data scope=patient --data-urlencode "patient=$PID" --data types=Patient --data types=Condition --data types=Observation; wait_done $J; zipcount $J
echo "5.3 group-active-conditions"; start group-active-conditions --data scope=group --data group_id=manual-group --data types=Patient --data types=Condition --data-urlencode 'type_filter=Condition?clinical-status=active' --data since_preset=; wait_done $J; zipcount $J active
echo "5.4 elements-subset"; start elements-subset --data scope=system --data types=Patient --data-urlencode 'elements=id,gender'; wait_done $J; zipcount $J elements
echo "5.6 negative: empty name"; curl -s -o /tmp/n.html -w '  POST -> %{http_code}\n' -X POST "${H[@]}" --data name= --data scope=system "$HFS/ui/bulk-export"; grep -o 'Enter a name for this export[^<]*' /tmp/n.html | head -1 | sed 's/^/  /'
echo "5.7 since-import"; start since-import --data scope=system --data types=Patient --data since_preset=custom --data-urlencode "since_custom=$T3START"; wait_done $J; zipcount $J
echo "5.8 until-import"; start until-import --data scope=system --data types=Patient --data since_preset= --data-urlencode "until=$T3START"; wait_done $J; zipcount $J
echo "5.9 since-until"; start since-until --data scope=system --data types=Patient --data since_preset=custom --data-urlencode "since_custom=$T3START" --data-urlencode "until=$T3END"; wait_done $J; zipcount $J
echo "5.10 since-fixtures"; start since-fixtures --data scope=system --data types=Patient --data types=RiskAssessment --data types=ValueSet --data types=Group --data since_preset=custom --data-urlencode "since_custom=$T3END"; wait_done $J; zipcount $J
echo "5.11 last-day"; start last-day --data scope=system --data types=Organization --data since_preset=day; wait_done $J; zipcount $J
echo "5.12 negative: bad instant"; curl -s -o /tmp/n.html -w '  POST -> %{http_code}\n' -X POST "${H[@]}" --data name=bad-instant --data scope=system --data since_preset=custom --data since_custom=yesterday "$HFS/ui/bulk-export"; grep -o 'Enter a valid FHIR instant[^<]*' /tmp/n.html | head -1 | sed 's/^/  /'
echo "5.13 bad-group"; start bad-group --data scope=group --data group_id=does-not-exist --data types=Patient; J13=$J; sleep 3; echo "  card: $(card $J13)"; echo "  retry -> $(curl -s -o /dev/null -w '%{http_code}' -X POST "${H[@]}" $HFS/ui/bulk-export/active/$J13/retry)"; sleep 5; echo "  card after retry: $(card $J13)"; echo "  delete -> $(curl -s -o /dev/null -w '%{http_code}' -X POST "${H[@]}" $HFS/ui/bulk-export/active/$J13/delete)"; echo "  card GET after delete -> $(curl -s -o /dev/null -w '%{http_code}' "${H[@]}" $HFS/ui/bulk-export/active/$J13/card)"
echo "5.5 cancel-me"; start cancel-me --data scope=system --data all_types=on; J5=$J; sleep 4; echo "  card while running: $(card $J5)"; echo "  cancel -> $(curl -s -o /dev/null -w '%{http_code}' -X POST "${H[@]}" $HFS/ui/bulk-export/active/$J5/cancel)"; sleep 4; echo "  card: $(card $J5)"; echo "  delete -> $(curl -s -o /dev/null -w '%{http_code}' -X POST "${H[@]}" $HFS/ui/bulk-export/active/$J5/delete)"; echo "  card GET after delete -> $(curl -s -o /dev/null -w '%{http_code}' "${H[@]}" $HFS/ui/bulk-export/active/$J5/card)"
echo "## T5 done $(date -u +%FT%TZ)"
