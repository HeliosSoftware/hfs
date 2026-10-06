#!/bin/bash
# run7/sqlexport.sh <name> <params.json>: disk-floor check, POST $sql-export, poll (bearer re-minted), track hfs max RSS, stream-download outputs
W=/home/angela/pass-1178; source $W/env.sh >/dev/null 2>&1; unset CURL_HOME
name=$1; params=$2; out=$W/sql-exports-dl/run7-$name; rm -rf $out; mkdir -p $out
free=$(df -B1 /home/angela/minio | tail -1 | awk '{print $4}'); floor=$((135*1024*1024*1024))
echo "[$name] disk free $((free/1073741824)) GiB (floor 135)"; [ $free -lt $floor ] && { echo "[$name] NOT STARTED: below floor"; exit 2; }
TOKEN=$($KC/get-token.sh 2>/dev/null); tmint=$(date +%s)
hdr=$(curl -sS -D - -o $out/kickoff.body -w "%{http_code}" -X POST -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/fhir+json" -H "Accept: application/fhir+json" -H "Prefer: respond-async" --data-binary @$params "$HFS/\$sql-export")
code=$(echo "$hdr" | tail -1); loc=$(echo "$hdr" | grep -i "^content-location:" | awk '{print $2}' | tr -d '\r'); id=$(echo "$loc" | grep -o '[0-9a-f-]\{36\}')
echo "[$name] kick-off HTTP $code job=$id at $(date -u +%FT%TZ)"; [ "$code" != "202" ] && { head -c 500 $out/kickoff.body; echo; exit 1; }
echo $id > $W/run7/current-job.id; t0=$(date +%s); maxrss=0; minma=999999999
while :; do
  now=$(date +%s); [ $((now-tmint)) -gt 200 ] && { TOKEN=$($KC/get-token.sh 2>/dev/null); tmint=$now; }
  r=$(ps -o rss= -p $(pgrep -f '^/home/angela/pass-1178/hfs-run7' | head -1) 2>/dev/null | tr -d ' '); [ "${r:-0}" -gt $maxrss ] && maxrss=$r
  ma=$(awk '/^MemAvailable/{print $2}' /proc/meminfo); [ $ma -lt $minma ] && minma=$ma
  st=$(curl -s -L -o $out/poll.body -w "%{http_code}" -H "Authorization: Bearer $TOKEN" "$HFS/export/$id/status")
  case $st in 200) break;; 202) sleep 5;; *) echo "[$name] poll HTTP $st after $(( $(date +%s)-t0 )) s: $(head -c 600 $out/poll.body)"; rm -f $W/run7/current-job.id; echo "[$name] hfs max RSS $((maxrss/1024)) MB, min MemAvailable $((minma/1024)) MB"; exit 1;; esac
done
rm -f $W/run7/current-job.id; el=$(( $(date +%s)-t0 ))
cp $out/poll.body $out/manifest.json
echo "[$name] complete in $el s; hfs max RSS during job $((maxrss/1024)) MB; min MemAvailable $((minma/1024)) MB"
python3 - $out <<'PY' > $out/files.tsv
import json,sys; m=json.load(open(sys.argv[1]+'/manifest.json'))
P={x['name']:x for x in m['parameter'] if x['name']!='output'}
print('#', P['status'].get('valueCode'), P.get('_format',{}).get('valueCode'), P.get('exportDuration',{}).get('valueInteger'))
for x in m['parameter']:
    if x['name']=='output':
        p={q['name']:q for q in x['part']}; print(p['name']['valueString']+'\t'+p['location']['valueUri'])
PY
head -1 $out/files.tsv | sed "s/^/[$name] status|format|duration s: /"
tail -n +2 $out/files.tsv | while IFS=$'\t' read nm url; do
  ext=${url##*.}; f=$out/$nm.$ext; TOKEN=$($KC/get-token.sh 2>/dev/null)
  curl -s -H "Authorization: Bearer $TOKEN" -o $f "$url"
  if [ "$ext" = parquet ]; then rows=$(/home/angela/manual-test-1179/venv/bin/python -c "import pyarrow.parquet as pq,sys;print(pq.ParquetFile(sys.argv[1]).metadata.num_rows)" $f); else rows=$(grep -c . $f); fi
  echo "[$name]   $nm $(stat -c %s $f) bytes, $rows rows/lines -> $(basename $f)"
done
