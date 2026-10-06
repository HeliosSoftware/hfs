#!/bin/bash
# run 5 guards, every 30 s: disk floor, MemAvailable, sustained swap growth -> DELETE the job (data kept) and log GUARD
W=/home/angela/pass-1178; source $W/env.sh >/dev/null 2>&1; unset CURL_HOME
R=$(cat $W/run6/t3-recipient.id); out=$W/run6/guard.log
FLOOR=$((135*1024*1024*1024)); MEMMIN=$((2*1024*1024)); base_swap=921600; grow=0; hist=""; tmint=0
while :; do
  now=$(date +%s); [ $((now-tmint)) -gt 240 ] && { TOKEN=$($KC/get-token.sh 2>/dev/null); tmint=$now; }
  free=$(df -B1 /home/angela/minio | tail -1 | awk '{print $4}')
  ma=$(awk '/^MemAvailable/{print $2}' /proc/meminfo); st=$(awk '/^SwapTotal/{print $2}' /proc/meminfo); sf=$(awk '/^SwapFree/{print $2}' /proc/meminfo); su=$((st-sf))
    hp=$(pgrep -f '^/home/angela/pass-1178/hfs-run4' | head -1); hr=$(ps -o rss= -p $hp 2>/dev/null | tr -d ' '); mr=$(ps -o rss= -C minio | head -1 | tr -d ' ')
  hist="$hist $su"; hist=$(echo $hist | awk '{n=NF; s=(n>11)?n-10:1; for(i=s;i<=n;i++) printf "%s ",$i}'); inc=$(echo $hist | awk '{c=0; for(i=2;i<=NF;i++) if($i>$(i-1)) c++; print c}'); delta=$(echo $hist | awk '{print $NF-$1}')
  echo "$(date -u +%FT%TZ) free_gib=$((free/1073741824)) memavail_mb=$((ma/1024)) swap_used_mb=$((su/1024)) hfs_rss_mb=$((${hr:-0}/1024)) minio_rss_mb=$((${mr:-0}/1024))" >> $out
  reason=""
  [ $free -lt $FLOOR ] && reason="disk free $((free/1073741824)) GiB < floor 135 GiB"
  [ $ma -lt $MEMMIN ] && reason="MemAvailable $((ma/1024)) MB < 2048 MB"
  [ "$inc" -ge 10 ] && [ "$delta" -gt 262144 ] && reason="swap growing in each of the last 10 samples (+$((delta/1024)) MB in 5 min), now $((su/1024)) MB"
  [ $su -gt $((base_swap + 3145728)) ] && reason="swap used $((su/1024)) MB > baseline 900 MB + 3 GiB"
  if [ -n "$reason" ]; then
    code=$(curl -s -o /dev/null -w '%{http_code}' -X DELETE -H "Authorization: Bearer $TOKEN" "$HFS/bulk-submit-status/$R")
    echo "$(date -u +%FT%TZ) GUARD TRIGGERED: $reason -> DELETE /bulk-submit-status/$R $code (data kept)" >> $out; break
  fi
  grep -q COMPLETED $W/run6/t3-monitor.log 2>/dev/null && { echo "$(date -u +%FT%TZ) job completed, guard stops" >> $out; break; }
  sleep 30
done
