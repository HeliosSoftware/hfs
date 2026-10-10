import json, sys, urllib.request, urllib.parse, os, time, subprocess
_tok={'v':os.environ['TOKEN'],'t':time.time()}
def tok():
    if time.time()-_tok['t']>3000: _tok['v']=subprocess.run(['/home/angela/pass-1182/entra-token.sh','hfs'],capture_output=True,text=True).stdout.strip(); _tok['t']=time.time()
    return _tok['v']
HFS=os.environ.get('HFS','http://localhost:8080'); PID='7d24f7a0-6f2e-ce3b-5568-db7b14695583'; LPID=os.environ['LPID']; T3=os.environ['T3START']
def run(q):
    url=HFS+'/'+q+('&' if '?' in q else '?')+'_total=accurate'
    url=url.replace('|','%7C').replace('$','%24').replace(' ','%20')
    try:
        with urllib.request.urlopen(urllib.request.Request(url,headers={'Authorization':'Bearer '+tok()}), timeout=600) as r: d=json.load(r); code=r.status
    except urllib.error.HTTPError as e: return e.code, None, None, e.read()[:200].decode(), []
    except (TimeoutError, OSError) as e: return 'TIMEOUT', None, None, repr(e)[:200], []
    if d.get('resourceType')!='Bundle': return code, None, None, d, []
    ents=d.get('entry',[]); inc=sum(1 for e in ents if e.get('search',{}).get('mode')=='include'); rows=[e['resource'] for e in ents if e.get('search',{}).get('mode')!='include']
    return code, d.get('total'), inc, None, rows
def main():
    rows_out=[]
    for line in open(sys.argv[1]):
        line=line.rstrip('\n')
        if not line or line.startswith('#'): continue
        tid, q, expect = line.split('\t')
        q=q.replace('LPID',LPID).replace('PID',PID).replace('T3START',T3)
        t0=time.time(); code,total,inc,err,rows=run(q); el=time.time()-t0
        shown=f"{total}" + (f" · {inc} included" if inc else "") + (f" (rows {len(rows)})" if rows and total is None else "")
        print(f"{tid:<6} HTTP {code}  total={shown:<24} expect={expect:<28} {el:6.1f}s GET /{q}")
        if err: print("       ERROR:", str(err)[:200])
main()
