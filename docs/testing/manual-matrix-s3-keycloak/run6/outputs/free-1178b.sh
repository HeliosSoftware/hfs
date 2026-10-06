#!/bin/bash
# run 6 step 1: size and delete data-1178b (authorized by Angela 2026-10-06), never data-1178c
o=/home/angela/pass-1178/run6/free-space.txt; T=/home/angela/minio/data-1178b
[ "$T" = /home/angela/minio/data-1178c ] && exit 1
echo "$(date -u +%FT%TZ) start; MinIO serves: $(ps -eo args | grep '[m]inio server' | awk '{print $3}')" >> $o
echo "$(date -u +%FT%TZ) df before: $(df -B1 /home/angela/minio | tail -1 | awk '{printf "%.1f GiB free, %.1f%% used",$4/2^30,100*$3/$2}')" >> $o
echo "$(date -u +%FT%TZ) du -s (disk usage) data-1178b: $(du -s --block-size=1 $T | cut -f1) B" >> $o
echo "$(date -u +%FT%TZ) du -sh: $(du -sh $T | cut -f1)" >> $o
echo "$(date -u +%FT%TZ) rm -rf $T start" >> $o
rm -rf "$T"; echo "$(date -u +%FT%TZ) rm exit $? ; exists after: $([ -e $T ] && echo yes || echo no)" >> $o
sync; sleep 30
echo "$(date -u +%FT%TZ) df after: $(df -B1 /home/angela/minio | tail -1 | awk '{printf "%.1f GiB free (%d B), %.1f%% used",$4/2^30,$4,100*$3/$2}')" >> $o
echo "$(date -u +%FT%TZ) data-1178c still present: $([ -d /home/angela/minio/data-1178c/hfs ] && echo yes || echo NO)" >> $o
echo DONE >> $o
