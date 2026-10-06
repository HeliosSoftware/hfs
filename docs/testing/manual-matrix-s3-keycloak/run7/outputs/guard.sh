#!/bin/bash
# run 7 guard, every 30 s: MemAvailable < 2 GiB or sustained swap growth (run-5 rule) -> DELETE the running $sql-export job (id in run7/current-job.id); disk floor logged
W=/home/angela/pass-1178; source $W/env.sh >/dev/null 2>&1; unset CURL_HOME; out=$W/run7/guard.log
FLOOR=$((135*1024*1024*1024)); MEMMIN=$((2*1024*1024)); hist=""; tmint=0
st0=$(awk '/^SwapTotal/{print $2}' /proc/meminfo); sf0=$(awk '/^SwapFree/{print $2}' /proc/meminfo); base_swap=$((st0-sf0))
echo "$(date -u +%FT%TZ) guard start, swap baseline $((base_swap/1024)) MB" >> $out
while :; do
  now=$(date +%s); [ $((now-tmint)) -gt 240 ] && { TOKEN=$($KC/get-token.sh 2>/dev/null); tmint=$now; }
  free=$(df -B1 /home/angela/minio | tail -1 | awk '{print $4}')
  ma=$(awk '/^MemAvailable/{print $2}' /proc/meminfo); st=$(awk '/^SwapTotal/{print $2}' /proc/meminfo); sf=$(awk '/^SwapFree/{print $2}' /proc/meminfo); su=$((st-sf))
  hp=$(pgrep -f '^/home/angela/pass-1178/hfs-run7' | head -1); hr=$(ps -o rss= -p $hp 2>/dev/null | tr -d ' '); mr=$(ps -o rss= -C minio | head -1 | tr -d ' ')
  hist="$hist $su"; hist=$(echo $hist | awk '{n=NF; s=(n>11)?n-10:1; for(i=s;i<=n;i++) printf "%s ",$i}'); inc=$(echo $hist | awk '{c=0; for(i=2;i<=NF;i++) if($i>$(i-1)) c++; print c}'); delta=$(echo $hist | awk '{print $NF-$1}')
  job=$(cat $W/run7/current-job.id 2>/dev/null)
  echo "$(date -u +%FT%TZ) free_gib=$((free/1073741824)) memavail_mb=$((ma/1024)) swap_used_mb=$((su/1024)) hfs_rss_mb=$((${hr:-0}/1024)) minio_rss_mb=$((${mr:-0}/1024)) job=${job:--}" >> $out
  reason=""
  [ $free -lt $FLOOR ] && reason="disk free $((free/1073741824)) GiB < floor 135 GiB"
  [ $ma -lt $MEMMIN ] && reason="MemAvailable $((ma/1024)) MB < 2048 MB"
  [ "$inc" -ge 10 ] && [ "$delta" -gt 262144 ] && reason="swap growing in each of the last 10 samples (+$((delta/1024)) MB in 5 min), now $((su/1024)) MB"
  [ $su -gt $((base_swap + 3145728)) ] && reason="swap used $((su/1024)) MB > baseline $((base_swap/1024)) MB + 3 GiB"
  if [ -n "$reason" ]; then
    if [ -n "$job" ]; then code=$(curl -s -o /dev/null -w '%{http_code}' -X DELETE -H "Authorization: Bearer $TOKEN" "$HFS/export/$job/status"); else code="no job running"; fi
    echo "$(date -u +%FT%TZ) GUARD TRIGGERED: $reason -> DELETE /export/$job/status $code" >> $out; rm -f $W/run7/current-job.id; sleep 60
  fi
  sleep 30
done
