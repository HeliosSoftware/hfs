#!/bin/bash
# keeps Angela's stored session alive during long waits: one GET /ui with her cookie every 20 min (a browser tab would do the same)
W=/home/angela/pass-1182; S=$(cat $W/private/session.id)
while :; do c=$(curl -s -o /tmp/ka.html -w '%{http_code}' -b "hfs_session=$S" -H 'Sec-Fetch-Site: same-origin' http://localhost:18090/ui); echo "$(date -u +%FT%TZ) GET /ui $c signed_in=$(grep -c Angela /tmp/ka.html)" >> $W/run/keepalive.log; sleep 1200; done
