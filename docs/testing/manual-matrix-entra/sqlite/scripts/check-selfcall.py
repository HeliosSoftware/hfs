import time,re,sys
from playwright.sync_api import sync_playwright
B='http://localhost:18090'; STATE='/home/angela/pass-1182/private/ui-state.json'
bad=re.compile(r'could not be loaded|returned \d{3}|Unauthorized|outbound service token|self-call to',re.I)
with sync_playwright() as p:
    b=p.chromium.launch(); ctx=b.new_context(storage_state=STATE,viewport={'width':1400,'height':900}); pg=ctx.new_page()
    net=[]; pg.on('response',lambda r: net.append((r.status,r.url.replace(B,''))) if r.status>=400 else None)
    for path in ['/ui/search-parameters','/ui/compartments','/ui/sql/view-definitions','/ui/sql/queries','/ui/sql/views','/ui/sql/export/new','/ui/capability-statement']:
        net.clear(); t0=time.time(); pg.goto(B+path,wait_until='networkidle',timeout=300000); dt=time.time()-t0
        t=re.sub(r'\s+',' ',pg.locator('main').inner_text())
        hits=sorted(set(m.group(0) for m in bad.finditer(t))); extra=''
        if 'search-param' in path: m=re.findall(r'([\d,]+)\s+parameters?',t); extra=f' counts={m[:2]}'
        if 'compartments' in path: extra=' '+str({k:(k in t) for k in ('Patient','Encounter','RelatedPerson')})
        print(f'{path}: {dt:.1f} s notices={hits or "none"} http>=400={net or "none"}{extra}')
        pg.screenshot(path=f'/home/angela/pass-1182/shots/selfcall{path.replace("/","-")}.png')
    b.close()
