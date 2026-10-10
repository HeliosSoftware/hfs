#!/bin/bash
# guards (run 5 of #1178): disk floor 135 GiB, MemAvailable < 2 GiB, sustained swap growth -> abort the import from the page (Angela's session); data kept
W=/home/angela/pass-1182; source $W/env.sh >/dev/null 2>&1; unset CURL_HOME; out=$W/run/guard.log; S=$(cat $W/private/session.id)
FLOOR=$((135*1024*1024*1024)); MEMMIN=$((2*1024*1024)); hist=""
base=$((1889*1024)); echo "$(date -u +%FT%TZ) guard start, swap baseline $((base/1024)) MB" >> $out
while :; do
  free=$(df -B1 $W/data | tail -1 | awk '{print $4}'); ma=$(awk '/^MemAvailable/{print $2}' /proc/meminfo); st=$(awk '/^SwapTotal/{print $2}' /proc/meminfo); sf=$(awk '/^SwapFree/{print $2}' /proc/meminfo); su=$((st-sf))
  hp=$(pgrep -f '^/home/angela/pass-1182/hfs' | head -1); hr=$(ps -o rss= -p $hp 2>/dev/null | tr -d ' '); db=$(du -s --apparent-size --block-size=1M $W/data 2>/dev/null | cut -f1); dbr=$(du -s --block-size=1M $W/data 2>/dev/null | cut -f1)
  hist="$hist $su"; hist=$(echo $hist | awk '{n=NF; s=(n>11)?n-10:1; for(i=s;i<=n;i++) printf "%s ",$i}'); inc=$(echo $hist | awk '{c=0; for(i=2;i<=NF;i++) if($i>$(i-1)) c++; print c}'); delta=$(echo $hist | awk '{print $NF-$1}')
  sid=$(cat $W/run/t3-submission.id 2>/dev/null)
  echo "$(date -u +%FT%TZ) free_gib=$((free/1073741824)) memavail_mb=$((ma/1024)) swap_used_mb=$((su/1024)) hfs_rss_mb=$((${hr:-0}/1024)) data_apparent_mb=$db data_disk_mb=$dbr" >> $out
  reason=""
  [ $free -lt $FLOOR ] && reason="disk free $((free/1073741824)) GiB < floor 135 GiB"
  [ $ma -lt $MEMMIN ] && reason="MemAvailable $((ma/1024)) MB < 2048 MB"
  # sustained-growth rule removed 2026-10-09 (Angela): idle-page swap-out, not pressure — see deviations.txt
  [ $su -gt $((base + 3145728)) ] && reason="swap used $((su/1024)) MB > baseline $((base/1024)) MB + 3 GiB"
  if [ -n "$reason" ]; then
    code=$([ -n "$sid" ] && curl -s -o /dev/null -w '%{http_code}' -X POST -b "hfs_session=$S" -H 'Sec-Fetch-Site: same-origin' "$HFS/ui/bulk-import/$sid/abort" || echo "no import")
    echo "$(date -u +%FT%TZ) GUARD TRIGGERED: $reason -> POST /ui/bulk-import/<id>/abort $code (data kept)" >> $out; sleep 120
  fi
  sleep 30
done
