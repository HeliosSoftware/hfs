#!/bin/bash
# T1: mint the outbound service token (client credentials, api://<HFS_CLIENT_ID>/.default), start hfs from the repo root on :18090
source /home/angela/pass-1182/env.sh; cd /home/angela/src/hfs
fuser -k 18090/tcp >/dev/null 2>&1; sleep 2
export HFS_OUTBOUND_BEARER_TOKEN=$(etok hfs)
echo "$(date -u +%FT%TZ) outbound token: $(echo -n "$HFS_OUTBOUND_BEARER_TOKEN" | python3 $WORK/claims.py)" >> $WORK/run/outbound-token.txt
for v in "$@"; do export "$v"; done
L0=$(wc -l < $WORK/hfs-sqlite-entra.log 2>/dev/null || echo 0)
echo "=== start $(date -u +%FT%TZ) extra: $*" >> $WORK/hfs-sqlite-entra.log
t0=$(date +%s.%N); (setsid nohup ${HFS_BIN:-$WORK/hfs} < /dev/null >> $WORK/hfs-sqlite-entra.log 2>&1 &)
for i in $(seq 1 600); do [ "$(curl -s -o /dev/null -w '%{http_code}' -m 2 $HFS/metadata)" = 200 ] && break; sleep 0.5; done; t1=$(date +%s.%N)
echo "/metadata 200 after $(python3 -c "print(f'{$t1-$t0:.1f}')") s"; tail -1 $WORK/run/outbound-token.txt
tail -n +$((L0+1)) $WORK/hfs-sqlite-entra.log | grep -a 'Authentication ENABLED\|Interactive browser login\|Subscriptions engine\|Initializing SQLite\|Seeded\|Server listening\|ERROR\|WARN' | sed -E 's/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/<id>/g' | cut -c1-230
