"""T2 Batch / Transaction page driver (#1171 §6): uploads a bundle on /ui/batch, reads the plan strip, clicks Execute, and reports the outcome or error. Usage: t2.py <file> <label> [--state ui-state.json]"""
import sys, time, json
from playwright.sync_api import sync_playwright
W='/home/angela/pass-1182'; B='http://localhost:18090'
f, label = sys.argv[1], sys.argv[2]; state = sys.argv[4] if len(sys.argv) > 4 else None
with sync_playwright() as p:
    b = p.chromium.launch(); ctx = b.new_context(storage_state=state, viewport={'width':1400,'height':1000}) if state else b.new_context(viewport={'width':1400,'height':1000})
    pg = ctx.new_page(); errors=[]; pg.on('console', lambda m: errors.append(m.text) if m.type=='error' else None)
    pg.goto(B+'/ui/batch', wait_until='networkidle'); print('url', pg.url)
    if '/ui/batch' not in pg.url: print('NOT ON BATCH PAGE'); pg.screenshot(path=f'{W}/shots/t2-{label}-redirect.png'); b.close(); sys.exit(1)
    pg.set_input_files('#batch-file', f); time.sleep(1.5)
    line = pg.inner_text('#batch-request-line') if pg.is_visible('#batch-request-line') else '(no plan)'
    sem = pg.inner_text('#batch-semantics') if pg.is_visible('#batch-semantics') else ''
    print('plan:', line); print('notice:', sem)
    up_err = pg.inner_text('#batch-upload-error') if pg.is_visible('#batch-upload-error') else ''
    if up_err: print('upload error:', up_err); pg.screenshot(path=f'{W}/shots/t2-{label}.png'); b.close(); sys.exit(0)
    rows = pg.locator('#batch-rows > li'); print('actions:', rows.count(), 'first:', rows.nth(0).inner_text().split('\n')[0][:80], '| second:', rows.nth(1).inner_text().split('\n')[0][:80] if rows.count()>1 else '')
    pg.click('#batch-execute-top'); t0=time.time()
    while time.time()-t0 < 900:
        if pg.is_visible('#batch-execute-error') and pg.inner_text('#batch-execute-error').strip(): break
        if pg.is_visible('#batch-response'): break
        time.sleep(0.5)
    print('elapsed %.1f s' % (time.time()-t0))
    if pg.is_visible('#batch-execute-error') and pg.inner_text('#batch-execute-error').strip():
        print('EXECUTE ERROR:', pg.inner_text('#batch-execute-error').strip())
    if pg.is_visible('#batch-response'):
        print('outcome badge:', pg.inner_text('#batch-overall').strip(), '| head:', pg.inner_text('#batch-created').strip(), '| summary:', pg.inner_text('#batch-summary').strip()[:120])
        outs = pg.locator('#batch-outcomes > li'); statuses={}
        for i in range(outs.count()):
            t = outs.nth(i).inner_text().split('\n'); s=[x for x in t if x.strip().startswith(('201','200','400','404','409','422','500'))]
            key=(s[0].strip()[:12] if s else t[0][:20]); statuses[key]=statuses.get(key,0)+1
        print('rows:', outs.count(), 'statuses:', statuses)
    pg.screenshot(path=f'{W}/shots/t2-{label}.png', full_page=True)
    print('console errors:', len(errors), errors[:3])
    b.close()
