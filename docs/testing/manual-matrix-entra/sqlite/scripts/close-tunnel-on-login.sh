#!/bin/bash
# close the SSH quick tunnel as soon as a web login completes, or after 60 min
W=/home/angela/pass-1182; L=$W/hfs-sqlite-entra.log; n0=$(grep -ac 'web login completed' $L); end=$(( $(date +%s) + 3600 )); out=$W/run/tunnel-close.txt
while [ $(date +%s) -lt $end ]; do
  [ $(grep -ac 'web login completed' $L) -gt $n0 ] && { reason="web login completed seen in the HFS log"; break; }
  sleep 5
done; reason=${reason:-"60-minute cap reached without a login"}
for p in $(pgrep -f '^/usr/local/bin/cloudflared tunnel --no-autoupdate --url ssh://localhost:22'); do kill $p; done; sleep 2
echo "$(date -u +%FT%TZ) quick tunnel closed: $reason; remaining quick-tunnel processes: $(ps -eo args | grep -c "^/usr/local/bin/cloudflared tunnel --no-autoupdate --url ssh")" >> $out
grep -a 'web login completed' $L | tail -1 | sed -E 's/subject=[^ ]+/subject=<redacted>/' | cut -c1-200 >> $out
