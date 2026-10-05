#!/bin/bash
PID=7d24f7a0-6f2e-ce3b-5568-db7b14695583; cd /home/angela/pass-1178/corpus-obs
echo "start $(date -u +%FT%TZ)"
grep -F -e '8302-2' -e "$PID" /home/angela/data/Observation.ndjson \
 | jq -c --arg pid "$PID" 'select((.code.coding | any(.code=="8302-2")) or ((.subject.reference // "") | endswith($pid)))' > obs-subset.ndjson
echo "end $(date -u +%FT%TZ) lines $(wc -l < obs-subset.ndjson) pid $(grep -c "$PID" obs-subset.ndjson)"
