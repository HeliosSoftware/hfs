#!/bin/bash
# poll $reindex-status/<job> every 60 s with the service token until completed/failed
source /home/angela/pass-1182/env.sh >/dev/null 2>&1; unset CURL_HOME; J=$1; out=$2; tmint=0
while :; do now=$(date +%s); [ $((now-tmint)) -gt 3000 ] && { T=$(etok hfs); tmint=$now; }
  s=$(curl -s -H "Authorization: Bearer $T" "$HFS/\$reindex-status/$J" | python3 -c 'import sys,json
try:
  d=json.load(sys.stdin); p={x["name"]:(x.get("valueString") or x.get("valueInteger") or x.get("valueCode") or x.get("valueDecimal")) for x in d.get("parameter",[])}; print(p.get("status"),"processed",p.get("processed"),"/",p.get("total"),"entries",p.get("entriesCreated"),"errors",p.get("errorCount"),"pct",p.get("percentage"),"started",p.get("startedAt"),"completed",p.get("completedAt"))
except Exception as e: print("ERR",e)')
  echo "$(date -u +%FT%TZ) $s" >> $out; echo "$s" | grep -qE '^(completed|failed|cancelled)' && break; sleep 60; done
