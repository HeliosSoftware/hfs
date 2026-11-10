#!/bin/bash
# start the real pg-es hfs detached and wait for /health (HFS_SUBSCRIPTIONS_ENABLED passes through)
cd $HOME/tmp/939/work && (setsid nohup ./start-hfs.sh >> $HOME/tmp/939/out/hfs-pg-es.log 2>&1 < /dev/null &)
for i in $(seq 1 180); do curl -sf localhost:18080/health >/dev/null && exit 0; sleep 2; done; exit 1
