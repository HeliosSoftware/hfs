#!/bin/bash
# companion to guard.sh while reindex-sub1.sh runs: guard.sh's trip action (abort the import) no longer applies, so on a trip
# stop the per-type loop and DELETE the running $reindex job (data kept), then log; same thresholds as guard.sh
W=/home/angela/pass-1182; source $W/env.sh >/dev/null 2>&1; unset CURL_HOME; G=$W/run/guard.log; out=$W/run/reindex-sub1.log
g0=$(grep -c 'GUARD TRIGGERED' $G)
while :; do
  grep -q 'reindex end' $out && exit 0
  if [ $(grep -c 'GUARD TRIGGERED' $G) -gt $g0 ]; then
    J=""; for i in $(seq 1 40); do J=$(ps -eo args | grep -o 'reindex-status/[0-9a-f-]\{36\}' | head -1 | cut -d/ -f2); [ -n "$J" ] && break; sleep 1; done
    for p in $(ps -eo pid,args | awk '$3 ~ /reindex-sub1b?.sh$/ {print $1}'); do kill $p; done
    T=$(etok hfs); code=$([ -n "$J" ] && curl -s -o /dev/null -w '%{http_code}' -X DELETE -H "Authorization: Bearer $T" "$HFS/\$reindex-status/$J" || echo "no job id")
    echo "$(date -u +%FT%TZ) GUARD TRIP during reindex: $(grep 'GUARD TRIGGERED' $G | tail -1 | cut -d' ' -f3- | cut -d'>' -f1) -> loop stopped, DELETE \$reindex-status/${J:0:8}… $code (data kept)" >> $out
    exit 0
  fi
  sleep 10
done
