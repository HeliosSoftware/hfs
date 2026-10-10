#!/bin/bash
# T7 as Angela: Libraries saved through the SQL Views / SQL Queries pages, exports through POST /ui/sql/export, cards polled like the browser, files downloaded with her session
source /home/angela/pass-1182/env.sh; unset CURL_HOME; cd $WORK; . run/ids.env; . run/t6-ids.env; S=$(cat private/session.id); PID=7d24f7a0-6f2e-ce3b-5568-db7b14695583
H=(-b "hfs_session=$S" -H 'Sec-Fetch-Site: same-origin'); OUT=$WORK/sql-exports-dl; mkdir -p $OUT
txt(){ python3 -c 'import sys,re,html; t=sys.stdin.read(); t=re.sub(r"<script.*?</script>","",t,flags=re.S); print(html.unescape(re.sub(r"\s+"," ",re.sub(r"<[^>]+>"," ",t))).strip()[:int(sys.argv[1])])' ${1:-300}; }
lib_save(){ # $1 page (views|queries) $2 library json  $3 sql
  python3 - "$2" "$3" > /tmp/lib.json <<'PY'
import json,sys,base64; d=json.load(open(sys.argv[1])); d.pop('content',None); print(json.dumps(d))
PY
  curl -s -o /dev/null -D /tmp/ls.hdr -w '%{http_code}' -X POST "${H[@]}" --data-urlencode "json@/tmp/lib.json" --data-urlencode "sql=$3" --data action=save --data id= "$HFS/ui/sql/$1"; }
ids(){ curl -s "${H[@]}" "$HFS/ui/sql/export" | grep -o 'id="job-[0-9a-f-]\{36\}"' | cut -d- -f2- | tr -d '"' | sort; }
card(){ curl -s "${H[@]}" -H 'HX-Request: true' "$HFS/ui/sql/export/$1/card" | txt 240; }
waitdone(){ for i in $(seq 1 ${2:-720}); do c=$(card $1); echo "$c" | grep -qE 'Complete|Failed|Cancelled' && break; sleep 5; done; echo "  card: $c"; }
files(){ # download every output link on the detail page; count rows
  curl -s "${H[@]}" "$HFS/ui/sql/export/$1" -o /tmp/d.html; for u in $(grep -o 'href="/export/[^"]*"' /tmp/d.html | cut -d'"' -f2 | sort -u); do f=$OUT/$2-$(basename $u); curl -s "${H[@]}" -o $f "$HFS$u"
    case $f in *.parquet) r=$(/home/angela/manual-test-1179/venv/bin/python -c "import pyarrow.parquet as pq,sys;m=pq.ParquetFile(sys.argv[1]);print(m.metadata.num_rows,[c.name for c in m.schema_arrow])" $f);; *.json) r=$(python3 -c "import json,sys;d=json.load(open(sys.argv[1]));print(len(d),'(array)')" $f);; *.csv) r="$(wc -l < $f) lines, header: $(head -1 $f)";; *) r="$(grep -c . $f) lines";; esac; echo "  file $(basename $u): $(stat -c %s $f) B, $r"; done; }
run(){ local name=$1; shift; local b=$(ids); local c=$(curl -s -o /tmp/k7.html -w '%{http_code}' -X POST "${H[@]}" --data-urlencode "name=$name" "$@" "$HFS/ui/sql/export"); J=$(comm -13 <(echo "$b") <(ids) | head -1); echo "$name: POST -> $c job ${J:-<none>}"; [ -z "$J" ] && { txt 200 < /tmp/k7.html; return; }; waitdone $J; files $J $name; }
echo "## T7 $(date -u +%FT%TZ)"
printf '11.1 save SQL View female_patients -> %s ' $(lib_save views fixtures/lib-view.json "SELECT id, birth_date, city FROM pd WHERE gender = 'female'"); QV=$(grep -i '^location' /tmp/ls.hdr | grep -o 'lib=[^&]*' | cut -d= -f2); echo "lib $QV"
printf '11.2 save SQL Query tall_female_patients -> %s ' $(lib_save queries fixtures/lib-query.json "SELECT fp.id, fp.city, MAX(obs.value) AS height FROM fp JOIN obs ON obs.patient_id = fp.id WHERE obs.code = '8302-2' AND obs.value > :min_height GROUP BY fp.id, fp.city"); QQ=$(grep -i '^location' /tmp/ls.hdr | grep -o 'lib=[^&]*' | cut -d= -f2); echo "lib $QQ"
echo "VD=$VD VD2=$VD2 QV=$QV QQ=$QQ" | tee run/t7-ids.env
printf '11.2 negative: sql-query Library saved on SQL Views -> '; sed 's/"sql-query"/"sql-view"/' fixtures/lib-query.json > /tmp/lq-wrong.json; curl -s -X POST "${H[@]}" --data-urlencode "json@/tmp/lq-wrong.json" --data-urlencode "sql=SELECT 1" --data action=save --data id= "$HFS/ui/sql/queries" | grep -o 'must be "sql-query"[^<]*' | head -1
run vd-ndjson --data subject=ViewDefinition/$VD --data format=ndjson
run query-csv --data subject=Library/$QQ --data "param:Library/$QQ:min_height=150" --data format=csv --data header=on
run view-parquet --data subject=Library/$QV --data format=parquet
run all-three-json --data subject=ViewDefinition/$VD --data subject=Library/$QQ --data "param:Library/$QQ:min_height=150" --data subject=Library/$QV --data format=json
run one-patient --data subject=ViewDefinition/$VD --data-urlencode "patient=$PID" --data format=ndjson
run one-group --data subject=ViewDefinition/$VD --data group=manual-group --data format=json
run since-import --data subject=ViewDefinition/$VD --data since_preset=custom --data-urlencode "since_custom=$T3START" --data format=ndjson
run since-nothing --data subject=ViewDefinition/$VD --data since_preset=custom --data-urlencode "since_custom=$T3END" --data format=ndjson
run tracked-csv-noheader --data subject=Library/$QV --data format=csv --data client_tracking_id=release-check-01
run vd-csv --data subject=ViewDefinition/$VD --data format=csv --data header=on
run vd-parquet --data subject=ViewDefinition/$VD --data format=parquet
run query-ndjson --data subject=Library/$QQ --data "param:Library/$QQ:min_height=150" --data format=ndjson
run query-parquet --data subject=Library/$QQ --data "param:Library/$QQ:min_height=150" --data format=parquet
run view-ndjson --data subject=Library/$QV --data format=ndjson
echo "7.f / 7.l negatives"; for d in "name=neg" "name=neg&subject=Library/$QQ" "name=neg&subject=ViewDefinition/$VD&since_preset=custom&since_custom=yesterday" "name=neg&subject=ViewDefinition/$VD&client_tracking_id=$(printf 'x%.0s' $(seq 201))" "name=neg&subject=ViewDefinition/$VD&patient=not+a+valid+id!"; do curl -s -X POST "${H[@]}" --data "$d&format=ndjson" "$HFS/ui/sql/export" | grep -oE 'Select at least one subject\.|This value is required\.|Enter a valid FHIR instant[^<]*|Tracking id must be 200 characters or fewer\.|Enter only valid logical Patient IDs[^<]*' | head -1 | sed 's/^/  /'; done
echo "7.e cancel-me (observation_flat)"; b=$(ids); curl -s -o /dev/null -X POST "${H[@]}" --data name=cancel-me --data subject=ViewDefinition/$VD2 --data format=ndjson "$HFS/ui/sql/export"; J=$(comm -13 <(echo "$b") <(ids) | head -1); sleep 5; echo "  card while running: $(card $J)"; echo "  cancel -> $(curl -s -o /dev/null -w '%{http_code}' -X POST "${H[@]}" $HFS/ui/sql/export/$J/cancel)"; sleep 5; echo "  card: $(card $J)"
echo "## T7 done $(date -u +%FT%TZ)"
