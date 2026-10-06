# run 6: remaining Observation (after the 6.5 M prefix) and ExplanationOfBenefit, minus ids already stored in data-1178c
import os,json
R='/home/angela/minio/data-1178c/hfs/default/resources'
def ids_of(line):
    k=line.find('"id":"'); return line[k+6:line.index('"',k+6)]
stored_eob=set(os.listdir(R+'/ExplanationOfBenefit'))
stored_obs=set(json.loads(l)['id'] for l in open('/home/angela/pass-1178/corpus-obs/obs-subset.ndjson'))  # the only stored Observations that can lie past the prefix
stats={}
with open('/home/angela/data/Observation.ndjson') as f, open('/home/angela/data/obs-final.ndjson','w') as o:
    seen=kept=skip=0
    for i,l in enumerate(f):
        if i<6500000: continue
        seen+=1
        if ids_of(l) in stored_obs: skip+=1; continue
        o.write(l); kept+=1
    stats['Observation']=dict(after_prefix=seen,already_stored=skip,kept=kept)
with open('/home/angela/data/ExplanationOfBenefit.ndjson') as f, open('/home/angela/data/eob-final.ndjson','w') as o:
    seen=kept=skip=0
    for l in f:
        seen+=1
        if ids_of(l) in stored_eob: skip+=1; continue
        o.write(l); kept+=1
    stats['ExplanationOfBenefit']=dict(total=seen,already_stored=skip,kept=kept,stored_dirs=len(stored_eob))
m=json.load(open('/home/angela/data/manifest.json'))
m['output']=[{'type':'Observation','url':'http://localhost:8000/obs-final.ndjson','count':stats['Observation']['kept'],'fileSize':os.path.getsize('/home/angela/data/obs-final.ndjson')},
             {'type':'ExplanationOfBenefit','url':'http://localhost:8000/eob-final.ndjson','count':stats['ExplanationOfBenefit']['kept'],'fileSize':os.path.getsize('/home/angela/data/eob-final.ndjson')}]
json.dump(m,open('/home/angela/data/manifest-final.json','w'),indent=2)
stats['manifest_total']=sum(o['count'] for o in m['output'])
print(json.dumps(stats,indent=1))
