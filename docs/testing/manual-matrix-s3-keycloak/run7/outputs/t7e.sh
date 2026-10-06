#!/bin/bash
# run 7 — 7.e Cancel on observation_flat NDJSON through the page, after MinIO's CPU settles; tracks hfs RSS/swap and bytes from MinIO
W=/home/angela/pass-1178; cd $W; source env.sh >/dev/null 2>&1; unset CURL_HOME; out=$W/run7/t7e.out
M=$(pgrep -x minio); P=$(pgrep -f '^/home/angela/pass-1178/hfs-run7'); low=0
echo "## wait for MinIO CPU < 10 % for 10 consecutive 30 s samples ($(date -u +%FT%TZ))" >> $out
for i in $(seq 1 240); do c=$(top -b -n2 -d5 -p $M | tail -1 | awk '{print int($9)}'); echo "$(date -u +%T) minio_cpu=$c%" >> $out; [ $c -lt 10 ] && low=$((low+1)) || low=0; [ $low -ge 10 ] && break; sleep 25; done
[ $low -lt 10 ] && { echo "MinIO never settled within 2 h; 7.e not started" >> $out; exit 1; }
rx(){ ss -tinp 2>/dev/null | grep -A1 'hfs-run7' | grep -o 'bytes_received:[0-9]*' | cut -d: -f2 | awk '{s+=$1} END{print s+0}'; }
mem(){ awk '/VmRSS|VmSwap/{printf "%s=%dMB ",$1,$2/1024}' /proc/$P/status; }
echo "## baseline $(date -u +%FT%TZ): hfs $(mem)" >> $out
/home/angela/manual-test-1179/venv/bin/python ui.py login >/dev/null
S=$(python3 -c 'import json;print(";".join(c["name"]+"="+c["value"] for c in json.load(open("ui-state.json"))["cookies"]))')
ids(){ curl -s -b "$S" "$HFS/ui/sql/export" | grep -o 'id="job-[0-9a-f-]\{36\}"' | cut -d- -f2- | tr -d '"' | sort; }
card(){ curl -s -b "$S" "$HFS/ui/sql/export/$1/card" | python3 -c 'import sys,re,html; t=html.unescape(re.sub(r"\s+"," ",re.sub(r"<[^>]+>"," ",sys.stdin.read()))); print(t.strip()[:200])'; }
free=$(df -B1 /home/angela/minio | tail -1 | awk '{print $4}'); echo "disk free $((free/1073741824)) GiB (floor 135)" >> $out; [ $free -lt $((135*1024*1024*1024)) ] && { echo "below floor, not started" >> $out; exit 2; }
b=$(ids); curl -s -b "$S" -o /dev/null -X POST --data "name=cancel-me-7e&subject=ViewDefinition/$VD2&format=ndjson" "$HFS/ui/sql/export"; J=$(comm -13 <(echo "$b") <(ids) | head -1)
UI_JOB=$J; echo "## kick-off $(date -u +%FT%TZ) ui job $J: $(card $J)" >> $out
# the API job id behind the card, for the guard
sleep 3; AJ=$(curl -s -b "$S" "$HFS/ui/sql/export/$J" | python3 -c 'import sys,re,html; t=html.unescape(re.sub(r"\s+"," ",re.sub(r"<[^>]+>"," ",sys.stdin.read()))); m=re.search(r"Job id ([0-9a-f-]{36})",t); print(m.group(1) if m else "")'); echo "API job id: ${AJ:-not found}" >> $out; [ -n "$AJ" ] && echo $AJ > $W/run7/current-job.id
for t in $(seq 0 10 60); do echo "$(date -u +%T) t=${t}s card: $(card $J | cut -c1-90) | hfs $(mem) rx=$(( $(rx)/1048576 ))MB" >> $out; [ $t -lt 60 ] && sleep 10; done
echo "## Cancel $(date -u +%FT%TZ) -> $(curl -s -b "$S" -o /dev/null -w '%{http_code}' -X POST "$HFS/ui/sql/export/$J/cancel")" >> $out
r0=$(rx)
for t in $(seq 10 10 300); do sleep 10; r=$(rx); echo "$(date -u +%T) +${t}s card: $(card $J | cut -c1-80) | hfs $(mem) rx_since_cancel=$(( (r-r0)/1024 ))KB" >> $out; done
rm -f $W/run7/current-job.id; echo "## done $(date -u +%FT%TZ)" >> $out
