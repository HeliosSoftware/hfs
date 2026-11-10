#!/bin/bash
# T5 -> T6 -> T7 -> T8 -> T9 on the real server, sequential (shared state), each logged
cd $HOME/tmp/939/pw
export HFS_BASE=http://127.0.0.1:18080 OUT=$HOME/tmp/939/out HFS_LOG=$HOME/tmp/939/out/hfs-pg-es.log CORPUS=full BACKEND=pg-es
export T3_START=$(sed -n 1p $OUT/t3-start.txt) T3_END=$(date -u -d "$(sed -n 1p $OUT/t3-end.txt)" +%FT%TZ)
export T3_END=2026-11-09T15:16:36Z   # data searchable (deferred reindex completed); UI "finished" instant was 15:51:02
export PID=7d24f7a0-6f2e-ce3b-5568-db7b14695583
export HFS_RESTART_CMD=$HOME/tmp/939/work/restart-hfs.sh T7_RESTART_CMD=$HOME/tmp/939/work/restart-hfs.sh
export HFS_RESTART_DISABLED_CMD=$HOME/tmp/939/work/restart-hfs-subs-off.sh
for s in t5 t6 t7 t8 t9; do echo "$(date -u +%T) start $s" >> $OUT/rest-progress.log; node steps/$s.mjs > $OUT/$s.out 2>&1; echo "$(date -u +%T) end $s exit=$?" >> $OUT/rest-progress.log; done
echo "$(date -u +%T) ALL DONE" >> $OUT/rest-progress.log
