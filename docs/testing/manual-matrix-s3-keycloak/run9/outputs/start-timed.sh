#!/bin/bash
# run9/start-timed.sh <label>: kill hfs, mint outbound token, launch hfs-run9, time until /metadata answers 200
source /home/angela/pass-1178/env.sh >/dev/null 2>&1; unset CURL_HOME; cd /home/angela/src/hfs; label=$1; out=$WORK/run9/start-$label.txt
fuser -k 8080/tcp >/dev/null 2>&1; sleep 2
export HFS_OUTBOUND_BEARER_TOKEN=$($KC/get-token.sh 2>/dev/null); export HFS_LOG_LEVEL=info HFS_UI_LOGIN_CLIENT_ID=hfs-web HFS_UI_LOGIN_COOKIE_SECURE=false
L0=$(wc -l < $WORK/hfs-s3-keycloak.log); echo "=== start $(date -u +%FT%T.%3NZ) run9 $label" >> $WORK/hfs-s3-keycloak.log
t0=$(date +%s.%N); (setsid nohup /home/angela/pass-1178/hfs-run9 < /dev/null >> $WORK/hfs-s3-keycloak.log 2>&1 &)
until [ "$(curl -s -o /dev/null -w '%{http_code}' -m 2 $HFS/metadata)" = 200 ]; do sleep 0.2; done; t1=$(date +%s.%N)
{ echo "launch $(date -u -d @${t0%.*} +%FT%TZ)  /metadata 200 after $(python3 -c "print(f'{$t1-$t0:.2f}')") s"
  tail -n +$((L0+1)) $WORK/hfs-s3-keycloak.log | grep -ai 'seed\|conformance\|Server listening\|ERROR\|WARN' | grep -v 'HFS_AUTH_AUDIENCE\|COOKIE_SECURE' | cut -c1-260; } | tee $out
