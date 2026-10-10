# claims.py: reads a JWT on stdin, prints non-secret claims (ids shortened)
import sys,json,base64,datetime
t=sys.stdin.read().strip()
if t.startswith('ERROR'): print(t); sys.exit()
p=t.split('.')[1]; p+='='*(-len(p)%4); c=json.loads(base64.urlsafe_b64decode(p))
short=lambda v: (v[:8]+'…') if isinstance(v,str) and len(v)>12 else v
print({'iss_host':c.get('iss','').split('/')[2] if c.get('iss') else None,'ver':c.get('ver'),'aud':short(c.get('aud')),'roles':c.get('roles'),'scp':c.get('scp'),'exp':datetime.datetime.fromtimestamp(c['exp'],datetime.UTC).strftime('%Y-%m-%dT%H:%M:%SZ'),'lifetime_min':round((c['exp']-c['iat'])/60)})
