import time,re,sys
from playwright.sync_api import sync_playwright
B='http://localhost:8080'; STATE='/home/angela/pass-1178/ui-state.json'; tag=sys.argv[1] if len(sys.argv)>1 else 'first'
bad=re.compile(r'could not be loaded|returned \d{3}|Unauthorized|Not Implemented|outbound service token|self-call to',re.I)
with sync_playwright() as p:
    b=p.chromium.launch(); ctx=b.new_context(storage_state=STATE,viewport={'width':1400,'height':900}); pg=ctx.new_page()
    net=[]; pg.on('response',lambda r: net.append((r.status,r.url.replace(B,''))) if r.status>=400 else None)
    for path in ['/ui/search-parameters','/ui/compartments']:
        net.clear(); t0=time.time(); pg.goto(B+path,wait_until='networkidle',timeout=900000); dt=time.time()-t0
        if 'localhost:8180' in pg.url: print(path,'NOT SIGNED IN'); continue
        t=re.sub(r'\s+',' ',pg.locator('main').inner_text())
        hits=sorted(set(m.group(0) for m in bad.finditer(t)))
        print(f'{path} [{tag} load]: {dt:.1f} s to networkidle; notices={hits or "none"}; http>=400={net or "none"}')
        if 'search-parameters' in path:
            m=re.findall(r'([\d,]+)\s+parameters?',t); print('   counts shown:',m[:4])
            rail=pg.evaluate('''()=>[...document.querySelectorAll('.filter-rail__list a, aside a')].filter(a=>a.offsetParent!==null).length'''); print('   rail entries visible:',rail)
        else:
            names=pg.evaluate('''()=>[...document.querySelectorAll('main a, main button, main li, main [role=option]')].map(e=>e.textContent.trim()).filter(s=>/^[A-Z][A-Za-z]+$/.test(s))''')
            uniq=sorted(set(names)); print('   compartment-like names:',len(uniq),uniq[:12]); print('   has Patient/Encounter/RelatedPerson:',[x in uniq for x in ('Patient','Encounter','RelatedPerson')])
        pg.screenshot(path=f'/home/angela/pass-1178/run8/shots/{path.strip("/").replace("/","-")}-{tag}.png',full_page=False)
        print('   text head:',t[:260])
    ctx.storage_state(path=STATE); b.close()
