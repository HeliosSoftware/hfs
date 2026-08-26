# concept-maps.spec.ts:139 — Deep diagnosis (reverse-toggle GET returns HTTP 400)

Failing test: `crates/hts-ui/e2e/tests/concept-maps.spec.ts:139`
```
reverse without targetCode surfaces the inline validation outcome without calling HTS
Locator: getByLabel('Target code', { exact: true })
Received: <element(s) not found>
Timed out 3000ms
```

TL;DR — the earlier `hts-ui-grupo-c-diagnosis.md` §5 hypothesis (dual `change` trigger race) is wrong per HTML5 (only the newly-checked radio fires `change`). The real root cause is on the wire: htmx appends the checked-radio's form value to the `hx-get` URL, producing a **duplicate `direction` query parameter** that axum's `Query<TranslateInputForm>` cannot deserialize. HTS-UI answers HTTP 400 and htmx's default `responseHandling` for 4xx is `swap: false`, so the reverse fieldset never lands in the DOM. The applied `hx-trigger="change" → "click"` change was harmless but does not address the wire-level cause.

---

## 1. Reproducible facts

### 1.1 Initial page state (after `page.goto("/ui/hts/concept-maps/ex-cm-1/translate")`)

`translate_input` (in `crates/hts-ui/src/concept_maps.rs`, ~L389) sees `HxRequest(false)` on a hard nav, calls `render_detail_with_tab(..., CmTab::Translate, TranslateDirection::Forward, ...)` and returns the full `pages/cm-detail.html`. Because `tab == CmTab::Translate`, `cm-detail.html` L149-L152 does `{% include "partials/hts-cm-translate-input.html" %}`. So the DOM contains:

- One `<form id="hts-workbench-input" ...>` (partial L29-L36) with `hx-post`, targeting `#hts-workbench-result`.
- Two direction radios wrapped in labels (partial L65-L82). Both radios sit **inside the form** and both carry:

```
hx-get="/ui/hts/concept-maps/ex-cm-1/translate?direction=forward|reverse"
hx-target="#hts-workbench-input"
hx-swap="outerHTML"
hx-trigger="click"
```

- Forward radio has the `checked` attribute; Reverse does not.
- The Forward-mode fieldset is present (`translate-code`, `translate-system`, `translate-display`), the Reverse fieldset with `<label for="translate-target-code">Target code</label>` is **not** rendered yet (askama `{% else %}` arm).

Fluent catalog values (`locales/en/main.ftl` L954-965) resolve to English `"Direction" / "Forward" / "Reverse" / "Target code"`; those match Playwright's `getByLabel("Reverse", { exact: true })` and `getByLabel("Target code", { exact: true })`.

### 1.2 DOM state after `.check()` on `getByLabel("Reverse")`

Playwright locates the wrapped `<input>` via label association (verified by test :91's sibling assertion `await expect(page.getByLabel("Forward", { exact: true })).toBeChecked()`, which only makes sense when `getByLabel` resolves to the input control). The radio has no custom CSS (`rg "hts-cm-workbench__radio"` in `crates/ui/assets/app.css` — no matches), so it renders with default browser styling and is fully actionable. `.check()` performs a real mouse click on the input; the browser flips the group so Reverse becomes `checked` and Forward becomes unchecked BEFORE the `click` event dispatches. The user confirms `.check()` succeeds and Reverse is checked.

### 1.3 htmx dispatches the GET — with a duplicate `direction`

htmx 2.0.4 (identified from the version string `version:"2.0.4"` at the top of `crates/ui/assets/htmx.min.js`) processes the click via `pt(...)`. Because our GET is `methodsThatUseUrlParams`, htmx runs `cn(elt, "get")` and then serialises the resulting `FormData` onto the URL. Reading `crates/ui/assets/htmx.min.js`:

- `cn(e, "get")` — for GET method, **only the trigger element itself** is walked (not the parent form): the branch `if(t!=="get"){on(n,o,i,g(e,"form"),l)}` skips the enclosing form; `on(n, r, i, e, l)` still walks the radio.
- `on(...)` → `tn(o)` — for `type === "radio"`, `tn` returns `o.checked`. At handler time Reverse is already `checked = true`, so `tn` returns true and the radio's `name=value` pair is appended: `r.append("direction", "reverse")`.
- `hn(v, r)` — no `hx-params` on the radio (grep of `crates/hts-ui/templates` returns zero hits for `hx-params`/`hx-include`/`hx-vals`), so nothing is filtered.
- In `de(...)`: `E = Q.config.methodsThatUseUrlParams.indexOf("get") >= 0` is `true`. The final URL is built as:

```
R = $;                                            // "/ui/hts/concept-maps/ex-cm-1/translate?direction=reverse"
if (R.indexOf("?") < 0) R += "?"; else R += "&";  // "?" already present → append "&"
R += an(w);                                       // "direction=reverse"
```

Result: htmx sends

```
GET /ui/hts/concept-maps/ex-cm-1/translate?direction=reverse&direction=reverse
HX-Request: true
HX-Target: hts-workbench-input
```

The literal `?direction=reverse` from the `hx-get` template and the checked radio's own form value collide on the same key. This is the smoking gun.

### 1.4 axum rejects the duplicate key

Handler signature (`crates/hts-ui/src/concept_maps.rs` L389-L395):

```rust
async fn translate_input(
    State(state): State<Arc<HtsUiState>>,
    Path(id): Path<String>,
    Query(form): Query<TranslateInputForm>,
    ...
```

`TranslateInputForm` (L373-L378) is:

```rust
#[derive(Debug, Deserialize, Default)]
struct TranslateInputForm {
    direction: Option<String>,
    #[allow(dead_code)]
    lang: Option<String>,
}
```

`Cargo.lock` pins `axum = 0.8.4` and `serde_urlencoded = 0.7.1` (no `serde_html_form` anywhere in the lockfile). axum 0.8's `Query::from_request_parts` runs `serde_urlencoded::from_str(query)`. serde's derived `Deserialize` for a struct with a scalar `Option<T>` field emits the standard "double-fill" guard: `if direction.is_some() { return Err(A::Error::duplicate_field("direction")); }`. With the URL from §1.3, the derive fires the guard on the second `("direction", "reverse")` pair.

axum's `QueryRejection::FailedToDeserializeQueryString` implements `IntoResponse` as **HTTP 400** with a plain-text body. The mock HTS is never hit (this is a pure axum-layer rejection, before `state.upstream.read_concept_map` runs), which is consistent with the tests around it passing — no `$translate` traffic is observed and no side-effects fire.

### 1.5 htmx swallows the 400

htmx's `Q.config.responseHandling` (first few hundred bytes of `htmx.min.js`) is:

```
{code:"204",swap:false},
{code:"[23]..",swap:true},
{code:"[45]..",swap:false,error:true}
```

`Pn(xhr)` at L~2050 matches `400` to the third arm: `swap: false, error: true`. `Dn(...)` therefore skips `$e(...)`, fires a `htmx:responseError` on the trigger, and returns. The `#hts-workbench-input` form is **never replaced**. The DOM still shows the Forward fieldset. `getByLabel("Target code")` correctly reports `<element(s) not found>` and Playwright times out at 3000 ms.

### 1.6 Symmetry check with the passing tests

- **:104** (`await expect(page.getByLabel("Forward", { exact: true })).toBeChecked()`) — no click, just an assertion on the initial DOM. Works.
- **:113** (`running forward $translate`) — fills text inputs and clicks the submit button. The submit triggers the form's `hx-post` (`method="post"`, not GET), so the form values go in the request body, not the URL; there is no `?direction=` in `hx-post`, so no key collision. The form POST reaches the mock and renders the match grid. Works.
- **:159 / :180** — do not touch the direction radio at all. Works.
- **:91** (tab-click) — fails for the *different* reason grupo-C tried to fix (the target/hx-select layout question; the empty `<div id="hts-workbench-input" hidden>` placeholder is now present so the swap target exists, but the failure mode there is out of scope for :139 and the applied placeholder patch alone does not exercise the direction-duplication path).

So the failure is scoped precisely to "GET request whose `hx-get` URL contains a query key that also happens to be the trigger radio's `name`".

### 1.7 Why the `change → click` swap did nothing

HTML5 §4.10.5.1.15 (`input type=radio`): only the radio whose checkedness *changed to true* fires `input`/`change`. The earlier grupo-C claim of a two-request race was wrong: the Forward radio (which becomes unchecked when Reverse is checked) does **not** fire `change`. Changing both triggers to `click` (which also only fires on the actually-clicked radio) does not remove the duplicated URL key, because htmx builds that URL exactly the same way for both trigger events — form serialisation runs in `de(...)` after `pt(...)` has forwarded the trigger to the request path.

---

## 2. Root cause (confirmed)

**htmx's default form-value inclusion on a GET whose trigger is inside a `<form>` collides with the literal `?direction=reverse` in `hx-get`, producing `?direction=reverse&direction=reverse`. `serde_urlencoded 0.7.1` (used by axum 0.8's `Query<T>`) errors on the duplicate scalar field, axum answers HTTP 400, and htmx's default 4xx response handling skips the swap.**

Evidence: `crates/ui/assets/htmx.min.js` (`cn`, `hn`, `de`, `Pn`, `Dn`, `responseHandling`); `crates/hts-ui/templates/partials/hts-cm-translate-input.html` L65-L82 (radio attribute set); `crates/hts-ui/src/concept_maps.rs` L373-L378, L389-L416; `Cargo.lock` (axum 0.8.4 + serde_urlencoded 0.7.1, no serde_html_form); test log `playwright-alt-e.log:187-206`.

The hypotheses the user asked about resolve as follows:

| # | Hypothesis | Verdict |
|---|------------|---------|
| 1 | htmx never sees the click inside a `<label>` wrapper | **Rejected.** No custom CSS hides the input; `.check()` clicks the input directly. htmx *does* see the click — it fires the GET; the GET is just rejected. |
| 2 | Response fires but `outerHTML` swap of the trigger's own containing form fails | **Rejected.** htmx `Me()` handles trigger-inside-target outerHTML swaps by removing the old element after the response is already in flight. The swap does not even get a chance — no 2xx to swap. |
| 3 | Server response is not what we expect | **Confirmed indirectly.** The server never reaches the render path; axum rejects the request at the extractor layer with a `duplicate field "direction"` error → HTTP 400. |
| 4 | `read_concept_map` (Alt E two-hop) blocks the response | **Rejected.** With a `_count=1000` search over 2 seeded CMs, the two hops complete in single-digit ms; the 3-s timeout is roomy. Also moot because the extractor rejects before the handler body runs. |
| 5 | Two `#hts-workbench-input` in the DOM after the swap | **Rejected.** No swap happens; only one form exists. |
| 6 | htmx not yet initialised | **Rejected.** `htmx.min.js` is `defer`-loaded; Playwright's `page.goto` waits for `load`, which fires only after deferred scripts run. `_input hx_renders_input_partial_only` ring test also proves the initial partial is well-formed. |
| 7 | `getByLabel("Reverse")` binds to the wrong element | **Rejected.** Test :104's `.toBeChecked()` on `getByLabel("Forward")` requires the input control as the target; the same locator strategy resolves Reverse the same way. |
| 8 | Custom JS in assets intercepts direction radios | **Rejected.** `crates/hts-ui/templates/layouts/base.html` L7-L12 only loads `theme.js` and `htmx.min.js`; the per-page JS in `crates/ui/assets/*.js` is only included by helios-ui pages, not HTS-UI. No custom radio handler exists. |

The htmx-driven direction toggle is **the only place in `crates/hts-ui/templates/**` that pairs `hx-get` with a literal `?key=value` while the trigger element carries the same `name` and lives inside a `<form>`**. That is the pattern grep proves is unique (see §1.3 grep hits). Everything else that fires htmx is either a form submit (POST body, no URL collision), a top-level anchor (no form ancestor), or an `hx-get` without a colliding query key. Consequently no other test in the suite is corrupted by the same wire-level bug.

---

## 3. Ranked fix options

Legend: **BR** = blast radius; **PoF** = probability of actually fixing :139; **§7.5** = alignment with the design's "URL carries `direction=…`, nojs hard nav lands the right form" contract from the template comment (partial L18-L21, L50-L61).

### F1 — Add `hx-params="none"` to both direction radios *(recommended)*

- BR: minimal. Two attributes added to two `<input>`s in a single partial.
- PoF: **100%**. `hn(...)` short-circuits to `new FormData()`; `Z` in `de()` is false; the URL is emitted verbatim as `hx-get` intended. No duplicate key, no 400, swap succeeds.
- §7.5: Preserves the literal `?direction=…` URL structure exactly as the template comment documents. Nojs contract untouched.

### F2 — Drop the `?direction=…` literal from `hx-get`, let htmx serialise the form value

- BR: small. Two `hx-get` values shortened.
- PoF: 100%. FormData contributes exactly one `direction=<value>` and the URL becomes `?direction=reverse` from serialisation instead of literal.
- §7.5: Same wire result on htmx path, but the source-of-truth for "which direction is being requested" shifts from URL literal → live form state. Slightly less legible in the template ("where does `direction` come from?").

### F3 — Move direction toggle to workbench-level swap (grupo-C edit 5 optional hardening: `hx-target="#hts-cm-workbench"` + `hx-select`)

- BR: medium. Requires editing both radios AND removing the `if is_htmx { return partial }` branch in `translate_input` (so hx-select has a full detail page to select from). Also requires updating `translate_tab_htmx_returns_input_partial_only` in `crates/hts-ui/tests/concept_maps.rs`.
- PoF: **0% on its own.** The trigger radio is still inside the form, so htmx still appends `direction=reverse` to the GET; axum still returns 400. This option fixes a different concern (tab-swap architecture) but leaves the wire-level bug intact unless paired with F1 or F2.
- §7.5: Fine architecturally, but not a fix for :139.

### F4 — `hx-trigger="change, click"` defensively

- BR: minimal.
- PoF: 0%. The trigger event isn't the failure; the URL is.

### F5 — Navigate directly to `?direction=reverse` in the test

- BR: only the spec file changes.
- PoF: passes the test by bypassing the exact behaviour it is supposed to cover (the operator's toggle action). Masks the real bug.

### F6 — Make the server tolerant: swap `direction: Option<String>` for `direction: Option<Vec<String>>` (or a custom deserializer) and take the last value

- BR: server-only, single file.
- PoF: 100%.
- §7.5: Fixes the symptom (accept duplicates) rather than the cause (htmx should not send duplicates). Adds shape churn for a client-side workaround.

### F7 — Set `serdes(default)` on the field and rely on serde's behaviour

- Does not change the duplicate-field guard in the derived `Deserialize`; PoF: 0%.

Ranking summary:

| Rank | Option | Blast radius | PoF | Design fit |
|------|--------|--------------|-----|------------|
| 1 | **F1 — `hx-params="none"`** | Minimal | 100% | Best (URL stays literal) |
| 2 | F2 — Drop query from `hx-get` | Small | 100% | Good (equivalent wire) |
| 3 | F6 — Server accepts duplicates | Small | 100% | Symptom-only |
| 4 | F3 — Workbench-level swap | Medium | **0% alone** | Orthogonal concern |
| 5 | F4 — Both triggers | Minimal | 0% | N/A |
| 6 | F5 — Test workaround | Minimal | Masks bug | Loses coverage |
| 7 | F7 — `serde(default)` | Minimal | 0% | N/A |

---

## 4. Chosen fix

**Apply F1: add `hx-params="none"` to both direction radios in `crates/hts-ui/templates/partials/hts-cm-translate-input.html`.**

That single attribute tells htmx `hn(...)` to reduce the request's FormData to empty; the URL then reflects the literal `hx-get` template exactly (`?direction=forward` or `?direction=reverse`), no duplicates, no HTTP 400, htmx swaps the response, "Target code" appears, the test proceeds to submit the empty form and assert `.hts-outcome--error`.

### 4.1 Exact diff — `crates/hts-ui/templates/partials/hts-cm-translate-input.html`

```html
    <label class="hts-cm-workbench__radio">
      <input type="radio" name="direction" value="forward"
             {% if direction == TranslateDirection::Forward %} checked{% endif %}
             hx-get="/ui/hts/concept-maps/{{ id }}/translate?direction=forward"
             hx-target="#hts-workbench-input"
             hx-swap="outerHTML"
             hx-trigger="click"
             hx-params="none">
      <span>{{ chrome.i18n.t("hts-cm-translate-direction-forward") }}</span>
    </label>
    <label class="hts-cm-workbench__radio">
      <input type="radio" name="direction" value="reverse"
             {% if direction == TranslateDirection::Reverse %} checked{% endif %}
             hx-get="/ui/hts/concept-maps/{{ id }}/translate?direction=reverse"
             hx-target="#hts-workbench-input"
             hx-swap="outerHTML"
             hx-trigger="click"
             hx-params="none">
      <span>{{ chrome.i18n.t("hts-cm-translate-direction-reverse") }}</span>
    </label>
```

Only two lines added (`hx-params="none"`). No other file needs to change. The trigger-note comment block at L57-L61 that argued for `change → click` is now factually misleading and should be updated to describe the real reason (form-value collision with the URL literal), but the fix itself is the two-attribute add.

### 4.2 Optional but strongly recommended: cover the wire contract in the Rust ring

`crates/hts-ui/tests/concept_maps.rs` currently has no test for `GET /translate?direction=reverse` with `HX-Request: true`. Add one so a future maintainer who removes `hx-params="none"` (or restructures the URL) fails a unit test rather than the whole e2e ring:

```rust
#[tokio::test]
async fn translate_input_hx_reverse_direction_renders_target_code() {
    let response = app()
        .oneshot(
            axum::http::Request::get(
                "/ui/hts/concept-maps/example-cm/translate?direction=reverse",
            )
            .header("HX-Request", "true")
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = body_text(response).await;
    assert!(
        html.contains("translate-target-code"),
        "reverse direction must render the `targetCode` input in the partial",
    );
    assert!(
        !html.contains("name=\"code\""),
        "reverse direction must NOT include the forward-mode `code` input",
    );
}
```

That test hits the same axum layer the failing e2e path exercises (it does *not* assert against htmx behaviour — it just proves the server side accepts `?direction=reverse` and renders the reverse arm).

---

## 5. Risks + rollback

### 5.1 What could break in other tests

- **None expected.** `hx-params="none"` on the direction radios only affects requests fired *by those two radios*. All other htmx GETs/POSTs in the file are on the form itself or on tab anchors, which are untouched.
- The **submit path** (`hx-post` on the form → `#hts-workbench-result`) does not read `hx-params` from the radios; it reads from the form element, which has none, so form data is sent normally on submit. Tests :113 (forward $translate), :159 (no-match), and the ring's `translate_forward_posts_code_and_system_parameters` + `translate_reverse_posts_target_code_parameter` continue to work.
- **Nojs contract**: a hard `GET /ui/hts/concept-maps/ex-cm-1/translate?direction=reverse` (someone typing/bookmarking the URL) is unaffected — the server-side `translate_input` handler always parses `direction` from the URL. There is no client-side JavaScript path involved in nojs, so `hx-params` is irrelevant off the htmx track.

### 5.2 Rust ring tests affected

- `translate_tab_htmx_returns_input_partial_only` — unchanged; that test hits `/translate` (no `?direction=`), so it never sees the duplicate-key case.
- All other ring tests in `crates/hts-ui/tests/concept_maps.rs` — unaffected. They exercise POST bodies, not radio-triggered GETs.
- The suggested additional ring test in §4.2 is a *new* test, not a breaking edit.

### 5.3 Rollback

Rollback is trivial: delete the two `hx-params="none"` attributes. The bug returns and Playwright :139 re-times out with the same "Target code / element(s) not found" signature.

### 5.4 Longer-term hygiene

Two follow-ups that are not required to close :139 but are worth queuing:

1. **Update the trigger-note comment (L57-L61)** in `hts-cm-translate-input.html` — replace the (now known to be incorrect) "`change` fires on both radios" narrative with the real reason (`hx-params` prevents the trigger-radio's own value from doubling the URL key).
2. **grupo-C edit 3 (workbench-level swap on the Translate tab anchor in `cm-detail.html`)** is still on the table for **:91**, and completely orthogonal to :139. If that lands later, keep `hx-params="none"` on the direction radios — the target/select change doesn't affect what htmx puts in the URL. In fact if the operator later moves the direction toggle to `hx-target="#hts-cm-workbench" + hx-select="#hts-cm-workbench"`, the trigger radio is *still* inside the form and the duplicate-key problem re-appears without `hx-params="none"`.

---

## Short answer for the operator

- **Root cause:** htmx serialises the checked-radio's `name=value` onto the `hx-get` URL. The URL literal already carries `?direction=reverse`, so the wire becomes `?direction=reverse&direction=reverse`. axum 0.8's `Query<TranslateInputForm>` (via `serde_urlencoded 0.7.1` + derived `Deserialize`) rejects the duplicate scalar field with HTTP 400. htmx's default 4xx handler skips the swap, so `<label>Target code</label>` never enters the DOM.
- **Chosen fix:** **Option F1** — add `hx-params="none"` to both direction-toggle radios (`<input type="radio" name="direction" value="forward|reverse">`) in `crates/hts-ui/templates/partials/hts-cm-translate-input.html`. Two-attribute change, zero blast radius.
- **Files to touch:** `crates/hts-ui/templates/partials/hts-cm-translate-input.html` (two lines added). Optional: add a Rust ring test in `crates/hts-ui/tests/concept_maps.rs` covering `GET /translate?direction=reverse` with `HX-Request: true`.
- **Expected test outcome:** :139 passes (reverse fieldset swaps in, "Target code" becomes visible, empty submit hits the pre-flight validation gate, `.hts-outcome--error` renders). No other test regresses; :91 remains failing for the separate grupo-C tab-swap reason.
