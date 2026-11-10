import sys, json
import pyarrow.parquet as pq

src = sys.argv[1]
sample = sys.argv[2] if len(sys.argv) > 2 else None
pf = pq.ParquetFile(src)
schema = [[f.name, str(f.type)] for f in pf.schema_arrow]
out = {"rows": pf.metadata.num_rows, "schema": schema, "row_groups": pf.num_row_groups, "head": []}
if pf.metadata.num_rows:
    t = pf.read_row_group(0).slice(0, 20)
    out["head"] = t.slice(0, 3).to_pylist()
    if sample:
        pq.write_table(t, sample)
print(json.dumps(out, default=str))
