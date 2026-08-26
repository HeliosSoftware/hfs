# `operations.spec.ts:531` — deep read-only diagnosis

Failing test:
`crates/hts-ui/e2e/tests/operations.spec.ts:531` — “seed submit returns the skeleton table with per-row + progress htmx polling attributes”.
Deferred in the current todo list as **#19 batch skeleton race**.

This document is read-only diagnosis. No code was edited.

---

## 1. Reproducible facts

### 1.1 The server actually returns the skeleton contract

Template `crates/hts-ui/templates/partials/hts-vs-batch-table.html` (lines 24–47) emits, for every row in the seeded job:

```25:47:crates/hts-ui/templates/partials/hts-vs-batch-table.html
      {% for row in rows %}
        <tr id="hts-batch-row-{{ row.index }}"
            aria-busy="true"
            hx-get="/ui/hts/operations/batch-validate/row/{{ row.index }}?batch_id={{ batch_id }}"
            hx-trigger="load"
            hx-swap="outerHTML">
```

…and, at the bottom of the same file (lines 52–59):

```52:59:crates/hts-ui/templates/partials/hts-vs-batch-table.html
  <div id="hts-batch-progress"
       role="status"
       aria-live="polite"
       hx-get="/ui/hts/operations/batch-validate/progress?batch_id={{ batch_id }}"
       hx-trigger="load, every 1s"
       hx-swap="outerHTML">
    <p>{{ chrome.i18n.t_arg2("hts-vs-batch-progress", "n", "0".to_string(), "m", rows.len().to_string()) }}</p>
  </div>
```

Every attribute the failing test asserts on **is present in the wire response**. Hypothesis 1 (“template omits the attributes”) is refuted.

The sibling passing test at `operations.spec.ts:610` proves this indirectly: it does a raw `page.request.post(...)`, greps the response body for `batch-validate/row/0?batch_id=…`, and always finds it. If the seed HTML were missing attributes, that test would also fail.

The Rust ring test `crates/hts-ui/tests/operations_e2.rs:341` (`batch_seed_returns_n_skeleton_rows`) confirms the seed body contains `hts-batch-row-{i}`, `aria-busy="true"`, per-row `hx-get="/ui/hts/operations/batch-validate/row/{i}?batch_id=…"`, and `hts-batch-progress` + `/batch-validate/progress?batch_id=…`. It does **not** currently assert `hx-trigger="load"` on the row nor `hx-trigger="every 1s"` on the progress region — a coverage gap that lets template drift on those two attributes slip past the Rust ring. See §5 for a suggested ring extension.

### 1.2 The completed row response reuses the same `id` with a different shape

`crates/hts-ui/templates/partials/hts-vs-batch-row.html` (line 9):

```9:39:crates/hts-ui/templates/partials/hts-vs-batch-row.html
<tr id="hts-batch-row-{{ index }}" class="hts-op-workbench__batch-row">
  <td><code>{{ input.code }}</code></td>
  <td><code>{{ input.system }}</code></td>
  <td>{{ input.display }}</td>
  <td>
    …badge / outcome…
```

Two decisive things about this partial:

- Same DOM id (`hts-batch-row-N`) as the skeleton row.
- **No** `hx-get`, **no** `hx-trigger`, **no** `aria-busy`.
- Has `class="hts-op-workbench__batch-row"`.

That is *exactly* the class Playwright reports in the 7 “settled” resolutions of the call log
(`<tr id="hts-batch-row-0" class="hts-op-workbench__batch-row">`), and the 2 transient
resolutions with `class="htmx-request htmx-swapping htmx-added htmx-settling"` are the
same completed row briefly wearing htmx’s swap-lifecycle classes.

Every element Playwright ever sees under `#hts-batch-row-0` is the **replacement** row, never the skeleton.

### 1.3 The race is deterministic on this harness

Seed handler `crates/hts-ui/src/operations.rs:1061` (`run_batch_seed_htmx`) does, in order:

1. Assigns a `batch_id`, creates the `BatchJob`, and inserts it into `BATCH_JOBS`.
2. **Spawns one `tokio::spawn` task per row** that acquires the shared semaphore permit and calls
   `run_batch_row_upstream` (`operations.rs:1100`). These tasks start racing immediately, in parallel with the response render.
3. Renders and returns `BatchTableTemplate` (the skeleton).

Per-row handler `run_batch_validate_row` (`operations.rs:1295`):

```1315:1363:crates/hts-ui/src/operations.rs
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
    loop {
        {
            let job = handle.read().await;
            if let Some(row) = job.rows.get(index) {
                if row.result.is_some() {
                    …return the completed row template…
                }
            }
        …
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
```

It polls the shared job every 50 ms, returning as soon as the background task has written a result.

Upstream in the e2e harness (`crates/hts-ui/e2e/boot.mjs`) is the same-process `hts` binary, backed by SQLite, with no ValueSet matching `http://example.org/ValueSet/e2e-batch` seeded. HTS therefore returns a fast negative outcome (single-digit ms typical, sub-100 ms worst case on a warm SQLite). The result is written to the job well before the response HTML has round-tripped through the wire and been parsed by htmx.

So the sequence from Playwright’s vantage point is:

| T (approx) | Event |
|---:|---|
| 0 ms | `page.getByRole("button", …).click()` |
| ~5 ms | Server begins fanning out `tokio::spawn` tasks (background). |
| ~10–20 ms | HTS returns per-row outcomes; background tasks write `row.result` and increment `completed`. |
| ~20–40 ms | Server finishes rendering `BatchTableTemplate` and returns 200 with the skeleton HTML. |
| ~20–40 ms | `page.waitForResponse(...)` resolves. |
| ~20–60 ms | Browser receives body; htmx swaps `outerHTML` of `#hts-workbench-result`; new subtree processed. |
| ~20–60 ms | Each row’s `hx-trigger="load"` fires **immediately** on insertion (htmx dispatches load on `htmx:afterProcessNode` for elements with a `load` trigger). |
| ~20–60 ms | Progress region’s `load` trigger also fires. |
| ~70–200 ms | `/batch-validate/row/{i}` returns the **completed** row within one 50 ms poll iteration (result was already written before the row endpoint was even hit). htmx swaps `outerHTML` → skeleton is destroyed. |
| Locator poll (~100 ms) | `page.locator("#hts-batch-row-0")` resolves against the **replacement** row. `hx-trigger` attribute is `null`. Assertion fails. |

Playwright locator polling defaults (`expect(...).toHaveAttribute`, `~5 s` timeout, retry cadence 100 ms) cannot outrun the ~50 ms window in which the skeleton actually exists in the DOM. The nine call-log resolutions all fall on the same side of the swap.

This is exactly hypothesis **3 + 4 + 5** combined:

- Hypothesis 3 (htmx removes the attributes after firing): *effectively* true — `hx-swap="outerHTML"` replaces the row wholesale with a fragment that never carries `hx-trigger` / `hx-get`.
- Hypothesis 4 (two elements share the id): confirmed — skeleton and completed row both use `id="hts-batch-row-N"`.
- Hypothesis 5 (locator resolves too late): confirmed — the locator only ever resolves against the replacement.

### 1.4 What passes / what fails today

- `operations.spec.ts:610` (“progress endpoint reaches the terminal…”): **passes**. It uses `page.request.post` (no browser render, no htmx firing) and asserts on the raw response text. That path proves the wire contract is intact.
- `operations.spec.ts:531`: **fails deterministically** on the very first attribute assertion (`hx-trigger` on `#hts-batch-row-0`). It uses `page.goto` + `click`, so htmx is live and the swap has already happened.
- `operations.spec.ts:210` (“batch-validate input surfaces the target ValueSet + a repeatable row form”): **passes** and is untouched by any option below — it never exercises Submit.
- Rust ring `operations_e2.rs::batch_seed_returns_n_skeleton_rows`: **passes**, but with the coverage gap noted in §1.1.

---

## 2. Root cause

**The seed response is correct; the assertion strategy is wrong for a live-browser context.**

The test asserts DOM state on `#hts-batch-row-0` after `await responsePromise`, but the response promise only guarantees that the seed body reached the network layer. It doesn’t bound htmx’s subsequent swap-then-fire-then-swap cycle. Because:

1. `hx-trigger="load"` fires the row poll during the *first* htmx processing pass, and
2. `hx-swap="outerHTML"` destroys the skeleton once the row poll returns, and
3. The row poll returns in ~1 iteration (~50 ms) because the fan-out has *already* completed while the seed HTML was still being rendered,

the skeleton is essentially never observable through a DOM locator. The DOM locator always resolves against the replacement row (`class="hts-op-workbench__batch-row"`, no `hx-*`, no `aria-busy`).

Quantitatively: the DOM contains the skeleton for roughly **10–60 ms**; Playwright's locator poll cadence is **~100 ms** with a 5 s ceiling. The race is not marginal — the skeleton lifetime is strictly shorter than one poll interval on this harness.

---

## 3. Ranked fix options

Ordering criteria (as requested): (a) blast radius, (b) preserves the test’s intent (asserting the skeleton contract), (c) reliability.

### F — Capture the seed response body and assert on the raw HTML. **RECOMMENDED**

Blast radius: minimal (spec-only, single file, single test). No template change, no server change, no route intercept, no htmx timing games. Preserves intent: the “skeleton contract” **is** the wire response — that is the contract §7.6.1 F1 = D actually defines (“the seed response is the skeleton table”). Reliability: perfect — there is no timing dependency; the response body is finalised by the time `waitForResponse` resolves. Consistent with the sibling test at line 610, which already uses this approach.

Ring-test parity: the Rust ring at `operations_e2.rs:341` already covers most of this; the e2e coverage should be complementary (browser round-trip + real htmx-driven request emission), and the Rust ring should be widened to cover `hx-trigger="load"` and `hx-trigger*=every` (§5).

### B — `page.route` interception to hold `/batch-validate/row/*` (and `/batch-validate/progress`) pending.

Blast radius: moderate (spec-only, but requires two intercepts and careful teardown). Preserves intent: yes — asserts on the actual DOM state. Reliability: good on paper, but two subtleties:

- The intercept must be installed **before** `page.goto` so the row requests emitted by htmx after the swap are caught. If installed later, the first row request slips through.
- Playwright warns on requests that neither `continue()`, `fulfill()`, nor `abort()`. Cleanest form is `route.fulfill()` after a `setTimeout(10_000)` so the request stays open past the assertions.
- htmx will still apply `htmx-request` to the skeleton row while its pending request is outstanding — attribute assertions on `hx-get` / `hx-trigger` / `aria-busy` still hold, but any class-based assertion would need to accept the intermediate class list. The current test doesn’t assert on class, so this is fine.

### A — Fix the template.

Not applicable — template is correct (§1.1). Skip.

### C — Read the response HTML via `page.content()` post-submit.

Blast radius: minimal. Preserves intent: partial — `page.content()` returns the **current** DOM serialisation, not the seed body. By the time we call it, the swap has already happened, so this degenerates to the failing case. Not viable without a `setContent` trick or the same `waitForResponse` capture that Option F does more directly. Skip in favour of F.

### D — Add `hx-trigger="load delay:2s"` (or similar) to slow the row swap.

Blast radius: large — visible UX regression. The whole point of the fan-out is “rows as they complete”; delaying every row by 2 s to appease Playwright is unacceptable. Reject.

### E — Assert settled state instead of skeleton state.

Blast radius: small, but *changes the test’s stated intent*. The test file’s comment (`operations.spec.ts:534–539`) is explicit that this is the skeleton contract; the terminal-state contract already has its own dedicated sibling at line 610. Reject.

**Final ranking:** F > B > C > E > A > D.

---

## 4. Chosen fix — Option F

Rewrite the assertions in `operations.spec.ts:531` to inspect the seed response body directly instead of DOM-locating a transient element. All other test structure (goto, form fill, click, `waitForResponse`) is preserved; only the six DOM-locator assertions change.

Proposed replacement for the assertion block (lines 569–600 of the current test), with light context around the click:

```typescript
    const responsePromise = page.waitForResponse(
      (r) =>
        r.url().includes("/ui/hts/operations/batch-validate") &&
        r.request().method() === "POST",
    );
    await page.getByRole("button", { name: /Run/i, exact: false }).click();
    const seedResponse = await responsePromise;
    // §7.6.1 F1 = D: the seed response body IS the skeleton contract.
    // We assert on the wire text rather than on `#hts-batch-row-N` in
    // the DOM, because htmx fires each row's `hx-trigger="load"`
    // immediately on insertion and `hx-swap="outerHTML"` destroys the
    // skeleton before any Playwright locator poll can observe it (the
    // per-row endpoint returns in ~1 poll iteration because the
    // fan-out `tokio::spawn` tasks have already completed by the time
    // the seed HTML is parsed). The terminal-state assertion still
    // lives in the sibling spec below.
    expect(seedResponse.status()).toBe(200);
    const seedHtml = await seedResponse.text();

    // Skeleton row 0: per-row polling target, aria-busy, load trigger.
    expect(
      seedHtml,
      "skeleton row 0 must carry aria-busy=\"true\"",
    ).toMatch(
      /<tr\b[^>]*\bid="hts-batch-row-0"[^>]*\baria-busy="true"[^>]*>/,
    );
    expect(
      seedHtml,
      "skeleton row 0 must carry its per-row hx-get target",
    ).toMatch(
      /<tr\b[^>]*\bid="hts-batch-row-0"[^>]*\bhx-get="\/ui\/hts\/operations\/batch-validate\/row\/0\?batch_id=[^"]+"[^>]*>/,
    );
    expect(
      seedHtml,
      "skeleton row 0 must fire hx-trigger=\"load\"",
    ).toMatch(
      /<tr\b[^>]*\bid="hts-batch-row-0"[^>]*\bhx-trigger="[^"]*\bload\b[^"]*"[^>]*>/i,
    );

    // Skeleton row 1: same shape, distinct row index.
    expect(
      seedHtml,
      "skeleton row 1 must carry its per-row hx-get target",
    ).toMatch(
      /<tr\b[^>]*\bid="hts-batch-row-1"[^>]*\bhx-get="\/ui\/hts\/operations\/batch-validate\/row\/1\?batch_id=[^"]+"[^>]*>/,
    );

    // Progress region: hx-get carries the batch_id and the trigger
    // list still contains the recurring `every Ns` cadence. Interval
    // is intentionally not hardcoded (Phase 3b may tune it).
    expect(
      seedHtml,
      "progress region must poll the batch-validate/progress endpoint",
    ).toMatch(
      /<div\b[^>]*\bid="hts-batch-progress"[^>]*\bhx-get="\/ui\/hts\/operations\/batch-validate\/progress\?batch_id=[^"]+"[^>]*>/,
    );
    expect(
      seedHtml,
      "progress region must carry a recurring every Ns trigger",
    ).toMatch(
      /<div\b[^>]*\bid="hts-batch-progress"[^>]*\bhx-trigger="[^"]*\bevery\s+\d+\s*s\b[^"]*"[^>]*>/i,
    );

    // Sanity: no OOB swaps on the seed response (§7.6.1 F1 bullet).
    expect(seedHtml).not.toMatch(/hx-swap-oob/);

    // TODO(phase-3b): once local polling timings are confirmed, add a
    // separate DOM-driven spec that (a) waits for each `#hts-batch-row-N`
    // to reach the settled `class="hts-op-workbench__batch-row"` state
    // (i.e. after fan-out drains) and (b) asserts `#hts-batch-progress`
    // reaches the terminal arm through the browser-side htmx polling
    // loop. That is the natural browser-side complement to the raw
    // `page.request` poll covered by the sibling spec below.
```

Notes on the regex shape:

- Each pattern is anchored on the opening `<tr` / `<div` and on the `id="…"` attribute so the assertion cannot spuriously match a fragment further down the document (e.g. the progress `<p>` node or a later completed row rendered in the nojs path, which is not reachable here anyway).
- `hx-trigger` matches `load` word-boundary-scoped, so a future template that emits `hx-trigger="load delay:250ms"` or `hx-trigger="load, revealed"` still passes.
- `every\s+\d+\s*s` mirrors the previous DOM assertion (`/every\s+\d+\s*s/i`) so tuning the interval under an `HTS_UI_*` knob does not break the spec.
- Response-body assertions carry Playwright per-assertion messages for triage on future breaks.

No other test in the file needs to change. In particular:

- The batch-validate input-surface test at line 210 is unchanged.
- The terminal-state sibling at line 610 is unchanged (it already uses raw request/response). Both continue to pass.

---

## 5. Risks + rollback

### Risks introduced by Option F

- **Coverage narrowing (mild):** the spec no longer proves that htmx actually parses these attributes in the browser. Mitigation: the sibling terminal-state test at line 610 exercises the full fan-out end-to-end through raw HTTP, and any real htmx-parsing regression would also break the DOM in the standalone value-sets / concept-maps flows that already assert on live htmx behaviour. If we want a strictly DOM-side proof, layer Option B on top later as a Phase 3b addition (the TODO block above leaves a hook).
- **Wire-format brittleness:** the regexes match attribute order under the opening tag. Askama emits attributes in source order, so this is stable — but note that if a template maintainer moves `hx-trigger` before `id`, the regex still passes (attribute order inside the tag is captured by `[^>]*` on both sides). This is intentional.
- **False negatives on Windows line endings:** the assertion uses `[^"]*` and `[^>]*`, both of which permit any character except the delimiter, so `\r\n` inside attribute values (there aren’t any, but hypothetically) would still match. No CRLF risk.

### Sibling tests unaffected

- `operations.spec.ts:610` (“progress endpoint reaches the terminal…”): uses `page.request.post` and `expect.poll` on the progress endpoint. It never touches `#hts-batch-row-N` in the DOM, so the fix is orthogonal. Still passes.
- `operations.spec.ts:210` (“batch-validate input surfaces…”): shell-level test that never Submits. Unaffected.
- All other `operations.spec.ts` tests: not in the same describe block; unaffected.

### Rollback

The proposed change is confined to the assertion block of a single test. Reverting the file to its previous form is a one-line git operation (`git checkout HEAD -- crates/hts-ui/e2e/tests/operations.spec.ts`) if the wire-body approach proves inadequate — the surrounding setup (form fill, submit, `waitForResponse`) is unchanged.

### Suggested follow-ups (not part of this fix)

- **Fill the Rust ring coverage gap** on `crates/hts-ui/tests/operations_e2.rs::batch_seed_returns_n_skeleton_rows` by adding assertions on `hx-trigger="load"` for at least one skeleton row and on `hx-trigger` containing `every 1s` for the progress region. Today those two attributes are only checked (indirectly) by the Playwright spec, so template drift on them would otherwise slip past the ring.
- **Design inconsistency to note (already logged in the phase-3a output doc):** the progress refresh template `hts-vs-batch-progress.html` emits `hx-get="/ui/hts/operations/batch-validate/progress"` **without** `?batch_id=` on subsequent polls, whereas the seed template embeds the id. If Phase 3b ever asserts on the progress region after its first self-swap, that mismatch will surface. Out of scope for this fix.

---

## Short summary

- **Root cause:** deterministic swap race. The seed response is correct — every `hx-get` / `hx-trigger` / `aria-busy` on `<tr id="hts-batch-row-N">` is emitted by `hts-vs-batch-table.html`. But `hx-trigger="load"` fires immediately on insertion, the per-row endpoint returns in one 50 ms poll iteration (fan-out `tokio::spawn` tasks have already finished by then), and `hx-swap="outerHTML"` replaces the skeleton row with `hts-vs-batch-row.html`, which uses the same `id` but carries none of the `hx-*` attributes. Playwright's locator polls at ~100 ms cadence and never resolves against the ~10–60 ms-lived skeleton.
- **Chosen fix:** **Option F** — capture the seed response body via the existing `page.waitForResponse(...)` and assert on the raw HTML instead of `page.locator("#hts-batch-row-0")`.
- **Files touched:** `crates/hts-ui/e2e/tests/operations.spec.ts` only (single test, assertion block). No template, server, or ring-test changes required to unblock. Suggested (non-blocking) follow-up: widen `crates/hts-ui/tests/operations_e2.rs::batch_seed_returns_n_skeleton_rows` to also assert `hx-trigger="load"` and `hx-trigger*=every Ns` so template drift on those two attributes is caught by the Rust ring.
- **Expected outcome:** `operations.spec.ts:531` passes deterministically on Windows and Linux; sibling tests at `operations.spec.ts:210` and `:610` continue to pass unchanged.
- **Exact code snippet:** see §4 above.
