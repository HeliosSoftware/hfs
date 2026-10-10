#!/bin/bash
# T3 monitor: every 30 s poll the Import page's status fragment (as the browser does) and the recipient's x-progress (service token, re-minted every 50 min)
source /home/angela/pass-1182/env.sh; unset CURL_HOME; SID=$1; R=$2; out=$WORK/run/t3-monitor.log; S=$(cat $WORK/private/session.id); tmint=0
while :; do
  now=$(date +%s); [ $((now-tmint)) -gt 3000 ] && { TOKEN=$(etok hfs); tmint=$now; }
  frag=$(curl -s -b "hfs_session=$S" -H 'Sec-Fetch-Site: same-origin' -H 'HX-Request: true' "$HFS/ui/bulk-import/$SID/status" | python3 -c 'import sys,re,html; t=html.unescape(re.sub(r"\s+"," ",re.sub(r"<[^>]+>"," ",sys.stdin.read()))); print(t.strip()[:160])')
  hdr=$(curl -s -D - -o /tmp/bss1182.body -H "Authorization: Bearer $TOKEN" "$HFS/bulk-submit-status/$R" | tr -d '\r'); code=$(echo "$hdr" | head -1 | awk '{print $2}'); prog=$(echo "$hdr" | grep -i '^x-progress' | cut -d' ' -f2-)
  echo "$(date -u +%FT%TZ) recipient=$code progress=\"$prog\" | page: $frag" >> $out
  [ "$code" = 200 ] && { cp /tmp/bss1182.body $WORK/run/t3-final-status.json; echo "$(date -u +%FT%TZ) COMPLETED" >> $out; }
  sleep 30
done
