#!/bin/bash
# 4.11 timed on its own, no client limit below 1 h, once HFS is idle
source /home/angela/pass-1182/env.sh >/dev/null 2>&1; unset CURL_HOME; cd $WORK; out=run/t4-411.txt
until [ $(top -b -n2 -d5 -p $(ps -eo pid,comm | awk '$2=="hfs"{print $1}') | tail -1 | awk '{print int($9)}') -lt 5 ]; do sleep 20; done
T=$(etok hfs); echo "start $(date -u +%FT%TZ) GET /Patient?_has:Observation:patient:code=http://loinc.org|8302-2&_count=5&_total=accurate" > $out
curl -s -m 3500 -o run/t4-411.json -w 'HTTP %{http_code} time %{time_total}s\n' -H "Authorization: Bearer $T" "$HFS/Patient?_has:Observation:patient:code=http://loinc.org%7C8302-2&_count=5&_total=accurate" >> $out
python3 -c 'import json;d=json.load(open("run/t4-411.json"));print(d.get("resourceType"),"total",d.get("total"),"entries",len(d.get("entry",[])),(d.get("issue") or [{}])[0].get("diagnostics","")[:200])' >> $out 2>&1
echo "end $(date -u +%FT%TZ)" >> $out
