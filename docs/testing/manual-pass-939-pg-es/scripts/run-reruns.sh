#!/bin/bash
# Re-run T5 5.1/5.2 (fixed script) and T7 narrowed rows while T9 is in its long polling window (no restart before ~15 min after 13.4a).
O=$HOME/tmp/939/out; P=$HOME/tmp/939/pw
echo "$(date -u +%T) reruns waiting for T9 13.4a" >> $O/rest-progress.log
until grep -q 'T9 13.4a\]' $O/t9.out 2>/dev/null; do sleep 10; done
echo "$(date -u +%T) reruns start (T9 13.4a seen)" >> $O/rest-progress.log
export HFS_BASE=http://127.0.0.1:18080 HFS_LOG=$O/hfs-pg-es.log CORPUS=full BACKEND=pg-es PID=7d24f7a0-6f2e-ce3b-5568-db7b14695583
export T3_START=$(sed -n 1p $O/t3-start.txt) T3_END=2026-11-09T15:16:36Z
# T5 rerun into its own OUT so the original evidence stays untouched; only 5.1/5.2 are copied afterwards
export OUT=$HOME/tmp/939/out-t5rerun; mkdir -p $OUT; rm -rf $OUT/*
cd $P && node steps/t5.mjs > $OUT/t5.out 2>&1; echo "$(date -u +%T) end t5 rerun exit=$?" >> $O/rest-progress.log
# T7 narrowed rows write into the main OUT (they are new rows with -narrow ids)
export OUT=$O
cd $P && timeout 1500 node steps/t7n.mjs > $O/t7n.out 2>&1; echo "$(date -u +%T) end t7n exit=$?" >> $O/rest-progress.log
echo "$(date -u +%T) reruns DONE" >> $O/rest-progress.log
