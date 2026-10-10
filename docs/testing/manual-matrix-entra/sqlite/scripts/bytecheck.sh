#!/bin/bash
out=/home/angela/pass-1182/run/corpus-served-bytes.txt; : > $out
echo "## 7.1 byte check $(date -u +%FT%TZ)" >> $out
for f in manifest.json $(python3 -c "import json;[print(o['url'].rsplit('/',1)[-1]) for o in json.load(open('/home/angela/data/manifest.json'))['output']]"); do
  local=$(stat -c %s /home/angela/data/$f); hdr=$(curl -sI http://localhost:8000/$f | grep -i content-length | tr -dc 0-9); got=$(curl -s http://localhost:8000/$f | wc -c)
  echo "$([ "$local" = "$got" ] && [ "$local" = "$hdr" ] && echo ok || echo MISMATCH) $f local=$local content-length=$hdr served=$got" >> $out
done
echo "http version: $(curl -s -o /dev/null -w '%{http_version}' http://localhost:8000/manifest.json)" >> $out; echo "## done $(date -u +%FT%TZ)" >> $out
