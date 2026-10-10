#!/bin/bash
# index coverage per type: resources (live) vs search_index rows for _id (one per indexed resource)
cd /home/angela/pass-1182; out=run/index-coverage.txt; echo "# index coverage $(date -u +%FT%TZ): type | stored | indexed (_id rows, is_contained=0) | missing" > $out
for ty in $(sqlite3 -readonly data/hfs.db "select distinct resource_type from resources where tenant_id='default' order by 1"); do
  s=$(sqlite3 -readonly data/hfs.db "select count(*) from resources where tenant_id='default' and resource_type='$ty' and is_deleted=0")
  i=$(sqlite3 -readonly data/hfs.db "select count(*) from search_index where tenant_id='default' and resource_type='$ty' and param_name='_id' and is_contained=0")
  echo "$ty | $s | $i | $((s-i))" >> $out
done; echo "# end $(date -u +%FT%TZ)" >> $out
