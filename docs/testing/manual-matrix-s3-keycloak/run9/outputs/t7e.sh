#!/bin/bash
# run 9 — 7.e Cancel on observation_flat NDJSON (page), with mc admin trace of S3 calls, hfs CPU, rx and RSS
W=/home/angela/pass-1178; cd $W; source env.sh >/dev/null 2>&1; unset CURL_HOME; out=$W/run9/t7e.out; MC=/home/angela/minio/bin/mc
P=$(pgrep -f '^/home/angela/pass-1178/hfs-run9'); M=$(pgrep -x minio)
rx(){ ss -tinp 2>/dev/null | grep -A1 'hfs-run9' | grep -o 'bytes_received:[0-9]*' | cut -d: -f2 | awk '{s+=$1} END{print s+0}'; }
mem(){ awk '/VmRSS|VmSwap/{printf "%s=%dMB ",$1,$2/1024}' /proc/$P/status; }
cpu(){ a=$(awk '{print $14+$15}' /proc/$P/stat); sleep 1; b=$(awk '{print $14+$15}' /proc/$P/stat); echo $(( b-a ))%; }
echo "## baseline $(date -u +%FT%TZ): hfs $(mem) cpu $(cpu) minio_cpu $(ps -o %cpu= -p $M)" >> $out
($MC admin trace --call s3 local 2>&1 | while IFS= read -r l; do echo "$(date -u +%T) $l"; done > $W/run9/mc-trace.log) & TR=$!
/home/angela/manual-test-1179/venv/bin/python ui.py login >/dev/null
S=$(python3 -c 'import json;print(";".join(c["name"]+"="+c["value"] for c in json.load(open("ui-state.json"))["cookies"]))')
ids(){ curl -s -b "$S" "$HFS/ui/sql/export" | grep -o 'id="job-[0-9a-f-]\{36\}"' | cut -d- -f2- | tr -d '"' | sort; }
card(){ curl -s -b "$S" "$HFS/ui/sql/export/$1/card" | python3 -c 'import sys,re,html; t=html.unescape(re.sub(r"\s+"," ",re.sub(r"<[^>]+>"," ",sys.stdin.read()))); print(t.strip()[:200])'; }
free=$(df -B1 /home/angela/minio | tail -1 | awk '{print $4}'); echo "disk free $((free/1073741824)) GiB" >> $out
b=$(ids); curl -s -b "$S" -o /dev/null -X POST --data "name=cancel-me-run9&subject=ViewDefinition/$VD2&format=ndjson" "$HFS/ui/sql/export"; J=$(comm -13 <(echo "$b") <(ids) | head -1)
echo "## kick-off $(date -u +%FT%TZ) ui job $J: $(card $J)" >> $out
sleep 2; AJ=$(curl -s -b "$S" "$HFS/ui/sql/export/$J" | python3 -c 'import sys,re,html; t=html.unescape(re.sub(r"\s+"," ",re.sub(r"<[^>]+>"," ",sys.stdin.read()))); m=re.search(r"Job id ([0-9a-f-]{36})",t); print(m.group(1) if m else "")'); echo "API job id: ${AJ:-?}" >> $out; [ -n "$AJ" ] && echo $AJ > $W/run9/current-job.id
for t in 0 10 20 30 40 50; do echo "$(date -u +%T) t=${t}s card: $(card $J | cut -c1-90) | hfs $(mem) cpu $(cpu) rx=$(( $(rx)/1048576 ))MB s3calls_10s=$(awk -v s="$(date -u -d '-10 sec' +%T)" '$1>=s' $W/run9/mc-trace.log | grep -c .)" >> $out; sleep 8; done
c0=$(date +%s.%N); echo "## Cancel $(date -u +%FT%T.%3NZ) -> $(curl -s -b "$S" -o /dev/null -w '%{http_code}' -X POST "$HFS/ui/sql/export/$J/cancel")" >> $out; r0=$(rx)
for t in $(seq 1 30); do sleep 1; echo "$(date -u +%T.%2N) +${t}s rx_since_cancel=$(( ($(rx)-r0)/1024 ))KB s3calls_this_s=$(grep -c "^$(date -u -d '-1 sec' +%T) " $W/run9/mc-trace.log) card: $(card $J | cut -c1-30)" >> $out; done
for t in $(seq 40 20 240); do sleep 18; echo "$(date -u +%T) +${t}s rx_since_cancel=$(( ($(rx)-r0)/1024 ))KB s3calls_20s=$(awk -v s="$(date -u -d '-20 sec' +%T)" '$1>=s' $W/run9/mc-trace.log | grep -c .) hfs $(mem) cpu $(cpu)" >> $out; done
kill $TR 2>/dev/null; pkill -f 'mc admin trace' 2>/dev/null; rm -f $W/run9/current-job.id
echo "UIJOB=$J APIJOB=$AJ" >> $out; echo "## done $(date -u +%FT%TZ)" >> $out
