#!/bin/bash
# T8 + T9 as Angela: topic/subscription saved with the Resource Editor PUT (session), Encounters uploaded on the Batch page (session), dashboard read with the session
source /home/angela/pass-1182/env.sh; unset CURL_HOME; cd $WORK; S=$(cat private/session.id); PY=/home/angela/manual-test-1179/venv/bin/python; L=hfs-sqlite-entra.log
H=(-b "hfs_session=$S" -H 'Sec-Fetch-Site: same-origin')
put(){ curl -s -o /tmp/p.out -w '%{http_code}' -X PUT "${H[@]}" -H 'Content-Type: application/fhir+json' --data-binary @$1 "$HFS/$2"; }
sub(){ curl -s "${H[@]}" -H 'Accept: application/fhir+json' "$HFS/Subscription/manual-sub" | python3 -c 'import sys,json;d=json.load(sys.stdin);print(d.get("status"),"v"+d["meta"]["versionId"])'; }
dash(){ curl -s "${H[@]}" "$HFS/ui/subscriptions" | python3 -c 'import sys,re,html; t=re.sub(r"<script.*?</script>|<!--.*?-->","",sys.stdin.read(),flags=re.S); s=html.unescape(re.sub(r"\s+"," ",re.sub(r"<[^>]+>"," ",t))); i=s.find("Active"); print(s[i:i+330])'; }
batch(){ $PY t2.py fixtures/encounters.json $1 x private/ui-state.json 2>&1 | grep 'outcome badge\|EXECUTE ERROR'; }
recv_on(){ fuser -k 9999/tcp >/dev/null 2>&1; (setsid nohup python3 webhook.py < /dev/null >> run/webhook.log 2>&1 &); sleep 1; }
recv_off(){ fuser -k 9999/tcp >/dev/null 2>&1; }
since(){ awk -v t="$1" '$1>=t' $L; }
echo "## T8 $(date -u +%FT%TZ)"; recv_on; n0=$(grep -c . run/webhook.log 2>/dev/null); n0=${n0:-0}
echo "PUT Basic/manual-topic -> $(put fixtures/topic.json Basic/manual-topic)"
echo "PUT Subscription/manual-sub -> $(put fixtures/subscription.json Subscription/manual-sub)"; sleep 8; echo "subscription: $(sub); webhook lines +$(( $(grep -c . run/webhook.log) - n0 )) (handshake), auth header: $(tail -1 run/webhook.log | python3 -c 'import sys,json;print(json.loads(sys.stdin.read()).get("auth"))')"
n1=$(grep -c . run/webhook.log); echo "3 Encounters via the Batch page: $(batch t8-enc)"; sleep 12; echo "  notifications +$(( $(grep -c . run/webhook.log) - n1 ))"
n2=$(grep -c . run/webhook.log); echo "Condition via the editor PUT: $(curl -s -o /dev/null -w '%{http_code}' -X POST "${H[@]}" -H 'Content-Type: application/fhir+json' -d '{"resourceType":"Condition","subject":{"reference":"Patient/7d24f7a0-6f2e-ce3b-5568-db7b14695583"},"code":{"text":"t8 probe"}}' $HFS/Condition)"; sleep 10; echo "  notifications +$(( $(grep -c . run/webhook.log) - n2 )) (expected 0)"
T=$(etok hfs); printf '$status (service token) -> '; curl -s -o /tmp/o.out -w '%{http_code} ' -H "Authorization: Bearer $T" "$HFS/Subscription/manual-sub/\$status"; python3 -c 'import json;d=json.load(open("/tmp/o.out"));print([ (p["name"],p.get("valueCode") or p.get("valueString") or p.get("valueInteger") or p.get("valueUnsignedInt")) for e in d.get("entry",[]) for p in e.get("resource",{}).get("parameter",[]) if p["name"] in ("status","events-since-subscription-start")] or str(d)[:160])'
printf '$events -> %s\n' $(curl -s -o /dev/null -w '%{http_code}' -H "Authorization: Bearer $T" "$HFS/Subscription/manual-sub/\$events"); printf '$status without token -> %s\n' $(curl -s -o /dev/null -w '%{http_code}' "$HFS/Subscription/manual-sub/\$status")
echo "receiver down, 3 Encounters, receiver back after 10 s"; recv_off; n3=$(grep -c . run/webhook.log); echo "  $(batch t8-down)"; sleep 10; recv_on; sleep 25; echo "  delivered after the receiver returned: +$(( $(grep -c . run/webhook.log) - n3 ))"
echo "## T9 $(date -u +%FT%TZ)"; echo "dashboard: $(dash)"
echo "+3 Encounters: $(batch t9-enc)"; sleep 15; echo "dashboard: $(dash)"
echo "failure path: receiver down, 3 Encounters, wait for Max retries exhausted"; t0=$(date -u +%FT%TZ); recv_off; echo "  $(batch t9-fail)"; for i in $(seq 1 72); do since $t0 | grep -q 'Max retries exhausted' && break; sleep 10; done; echo "  log: $(since $t0 | grep -ci 'connection failed') connection-failure lines; $(since $t0 | grep -m1 'Max retries exhausted' | cut -c1-120)"; sleep 5; echo "  dashboard: $(dash)"; echo "  subscription: $(sub)"
for s in sent fails status; do echo "  sort=$s -> $(curl -s "${H[@]}" "$HFS/ui/subscriptions?sort=$s" | grep -o 'selected[^>]*>[^<]*' | head -1)"; done
echo "reactivation"; recv_on; echo "  PUT Subscription (status=requested) -> $(put fixtures/subscription.json Subscription/manual-sub)"; sleep 10; echo "  subscription: $(sub)"; echo "  +3 Encounters: $(batch t9-recover)"; sleep 15; echo "  dashboard: $(dash)"
echo "## T9 step 5: restart"; L0=$(wc -l < $L); ./t1-start.sh >/dev/null 2>&1; echo "  $(tail -n +$((L0+1)) $L | grep -a 'rehydrated' | cut -c1-200)"; echo "  'Failed to persist' lines: $(tail -n +$((L0+1)) $L | grep -ac 'Failed to persist')"; echo "  session still valid after restart: $(curl -s "${H[@]}" $HFS/ui | grep -c Angela)"; echo "  dashboard: $(dash)"; echo "  +3 Encounters: $(batch t9-after-restart)"; sleep 15; echo "  dashboard: $(dash)"
echo "## T9 step 6: HFS_SUBSCRIPTIONS_ENABLED=false"; ./t1-start.sh HFS_SUBSCRIPTIONS_ENABLED=false >/dev/null 2>&1; echo "  page: $(curl -s "${H[@]}" $HFS/ui/subscriptions | grep -o 'The subscriptions engine is not enabled[^<]*' | head -1)"; echo "  sidebar entry present: $(curl -s "${H[@]}" $HFS/ui | grep -c 'href="/ui/subscriptions"')"
./t1-start.sh >/dev/null 2>&1; echo "  back to normal: $(grep -a 'rehydrated' $L | tail -1 | cut -c1-160)"
echo "## T8/T9 done $(date -u +%FT%TZ)"
