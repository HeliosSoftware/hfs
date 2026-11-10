#!/bin/bash
cd /home/edson/hfs-t3/corpus
jq -r '.output[].url' $HOME/tmp/939/work/manifest.json | sed 's|.*/||' | sort -u |
while read -r f; do
  want=$(stat -c %s "$f"); got=$(curl -s -o /dev/null -w '%{size_download}' "http://localhost:18801/$f")
  if [ "$want" = "$got" ]; then echo "ok    $f $want"; else echo "TRUNC $f want=$want got=$got"; fi
done | tee $HOME/tmp/939/work/corpus-served-bytes.txt
