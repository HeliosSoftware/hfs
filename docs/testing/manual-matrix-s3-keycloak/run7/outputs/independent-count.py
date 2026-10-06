import json
fem={}
for l in open('/home/angela/data/Patient.ndjson'):
    p=json.loads(l); fem[p['id']]=p.get('gender')=='female'
hits=set(); h_all=0; obs=0
with open('/home/angela/data/Observation.ndjson') as f:
    for l in f:
        obs+=1
        if '8302-2' not in l: continue
        o=json.loads(l)
        if not any(c.get('code')=='8302-2' for c in o.get('code',{}).get('coding',[])): continue
        h_all+=1
        v=o.get('valueQuantity',{}).get('value')
        if v is not None and v>150:
            pid=o['subject']['reference'].split('/')[-1].replace('urn:uuid:','')
            if fem.get(pid): hits.add(pid)
print('corpus Observation lines',obs)
print('8302-2 Observations',h_all)
print('female corpus patients with a height Observation > 150 =',len(hits))
open('/home/angela/pass-1178/run7/independent-ids.txt','w').write('\n'.join(sorted(hits))+'\n')
