#!/bin/bash
source /home/angela/pass-1182/env.sh >/dev/null 2>&1; unset CURL_HOME; cd $WORK; . run/ids.env
echo "## T4 searches resumed after 4.11 (#1930) $(date -u +%FT%TZ)"
export TOKEN=$(etok hfs) HFS LPID T3START
python3 t4.py t4-queries-rest.tsv; python3 t4.py t4-lpid.tsv; echo "## T4 end $(date -u +%FT%TZ)"
