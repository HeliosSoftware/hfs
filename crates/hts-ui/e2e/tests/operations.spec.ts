import { expect, test } from "@playwright/test";

// Phase 2 Slice E: the standalone Operations workbench at
// `/ui/hts/operations`. Mirrors Slices B/C/D specs but adds coverage for
// the workbench shell shape (seven-op selector, closure banner gate,
// threshold advanced panel gate, resource-family tab strip) and the
// widened per-op input surfaces that the detail-page embeds
// deliberately deferred.
//
// Slice E1 stubs $closure and batch-validate — the specs below only
// assert on the shell / input rendering for those two. The real
// batch-fanout and closure invocation specs land alongside Slice E2.
//
// Seed data required (see e2e/README.md — added by Slice G):
//
//   - the seed fixtures already used by the CS / VS / CM specs are
//     enough for E1: the Operations workbench in "free" scope mode
//     accepts a bare `system` URL and `code`, so no new seeds are
//     required to exercise the shell + input rendering.

test.describe("HTS Operations workbench shell (§7.6)", () => {
  test("landing on /ui/hts/operations shows the seven-op selector and defaults to $lookup", async ({
    page,
  }) => {
    const response = await page.goto("/ui/hts/operations");
    expect(response?.status(), "ops route must respond 200").toBe(200);
    await expect(
      page.getByRole("heading", { name: /Operations/i, level: 1 }),
    ).toBeVisible();

    // The op-selector strip renders one link per operation kind. Slice
    // E1 defaults to `?op=lookup&resource=CodeSystem`, so the Lookup
    // link is aria-current="page" and the input partial is the CS
    // Lookup form (widened with `useSupplement`).
    for (const op of [
      "$lookup",
      "$validate-code",
      "$subsumes",
      "$expand",
      "$translate",
      "$closure",
      "batch-validate",
    ]) {
      await expect(
        page.getByRole("link", { name: op, exact: true }),
      ).toBeVisible();
    }
    await expect(
      page.getByRole("link", { name: "$lookup", exact: true }),
    ).toHaveAttribute("aria-current", "page");
    // Widened CS Lookup surface: the `useSupplement` field is present
    // in the standalone workbench, but was deliberately NOT surfaced
    // in the Slice B detail-page embed. The field is a text input
    // grouped under a <fieldset><legend> — no explicit <label for> —
    // so the assertion pins the input by name, not by accessible label.
    await expect(
      page.locator("input[name='useSupplement']"),
    ).toBeVisible();
  });

  test("the closure banner renders only on ?op=closure", async ({ page }) => {
    await page.goto("/ui/hts/operations?op=lookup&resource=CodeSystem");
    await expect(page.locator("[role='status']").first()).toHaveCount(0);
    // §7.6 F7: the stateless-warning banner is `role="status"` and is
    // present only when the operator has selected `$closure`.
    await page.goto("/ui/hts/operations?op=closure&resource=");
    await expect(page.locator("[role='status']").first()).toBeVisible();
    await expect(
      page.getByText(/Closure state lives on the server/i),
    ).toBeVisible();
  });

  test("the Threshold advanced panel renders only on ?op=expand", async ({
    page,
  }) => {
    // §7.6 F12: the `<details>` Advanced panel that hosts the
    // `threshold` numeric input is scoped to `$expand`. Every other op
    // omits the panel entirely. The panel starts collapsed on $expand,
    // so the input is attached-but-hidden — assert on existence rather
    // than paint-visibility.
    await page.goto("/ui/hts/operations?op=lookup&resource=CodeSystem");
    await expect(
      page.locator("input[name='threshold']"),
    ).toHaveCount(0);
    await page.goto("/ui/hts/operations?op=expand&resource=ValueSet");
    await expect(
      page.locator("input[name='threshold']"),
    ).toHaveCount(1);
  });

  test("resource-family tab strip appears only for validate-code and batch-validate", async ({
    page,
  }) => {
    // §7.6 F5: only the two ops with two ScopeResource families expose
    // the tab strip. Every other op renders no tablist at all.
    await page.goto("/ui/hts/operations?op=lookup&resource=CodeSystem");
    await expect(page.locator("[role='tablist']").first()).toHaveCount(0);

    await page.goto("/ui/hts/operations?op=validate-code&resource=CodeSystem");
    await expect(page.locator("[role='tablist']").first()).toBeVisible();
    await expect(
      page.getByRole("tab", { name: /CodeSystem/i, exact: false }),
    ).toHaveAttribute("aria-selected", "true");

    await page.goto("/ui/hts/operations?op=batch-validate&resource=ValueSet");
    await expect(page.locator("[role='tablist']").first()).toBeVisible();
    await expect(
      page.getByRole("tab", { name: /ValueSet/i, exact: false }),
    ).toHaveAttribute("aria-selected", "true");
  });

  test("switching the op selector swaps the input partial via htmx without a full reload", async ({
    page,
  }) => {
    // §7.6 F14: op-selector links are `<a href="?op=X&resource=Y">`
    // with an `hx-get` fallback that swaps `#hts-workbench-input`.
    // Under the JS ring the swap keeps the URL contract but avoids
    // a full document reload.
    await page.goto("/ui/hts/operations?op=lookup&resource=CodeSystem");
    // Click through to $expand and verify the workbench input body is
    // now the VS $expand form (widened with `designation[]` chip).
    await page.getByRole("link", { name: "$expand", exact: true }).click();
    const workbench = page.locator("#hts-workbench-input");
    await expect(workbench).toBeVisible();
    // Threshold lives inside a collapsed <details> Advanced panel; pin
    // attach-in-DOM rather than paint-visibility to match §7.6 F12.
    await expect(
      workbench.locator("input[name='threshold']"),
    ).toHaveCount(1, { timeout: 3_000 });
    await expect(
      workbench.locator("input[name='designation']").first(),
    ).toBeVisible();
  });
});

test.describe("HTS Operations workbench inputs (§7.6.F4)", () => {
  test("CS Lookup widened input exposes the useSupplement field", async ({
    page,
  }) => {
    // Slice B's detail-page Lookup form deliberately hid
    // `useSupplement`; Slice E's standalone workbench surfaces the
    // full parameter matrix per hts-details.md §$lookup.
    await page.goto("/ui/hts/operations?op=lookup&resource=CodeSystem");
    await expect(
      page.locator("input[name='useSupplement']").first(),
    ).toBeVisible();
  });

  test("CS Validate widened input exposes the CodeableConcept mode selector", async ({
    page,
  }) => {
    // Slice B's detail-page Validate form was code + Coding only;
    // Slice E adds `CodeableConcept` mode plus every hts-details.md
    // §$validate-code parameter.
    await page.goto("/ui/hts/operations?op=validate-code&resource=CodeSystem");
    await expect(
      page.getByLabel("CodeableConcept", { exact: false }).first(),
    ).toBeVisible();
  });

  test("VS Expand widened input exposes the designation[] chip", async ({
    page,
  }) => {
    // Slice C's detail-page Expand form did not surface
    // `designation[]`; Slice E adds a repeatable chip filter.
    await page.goto("/ui/hts/operations?op=expand&resource=ValueSet");
    await expect(
      page.locator("input[name='designation']").first(),
    ).toBeVisible();
  });

  test("Closure input renders a required name plus repeatable coding rows", async ({
    page,
  }) => {
    // §7.6 F6: closure input is `name` (required) + repeatable
    // system+code rows. Row inputs are namespaced `concept.system` /
    // `concept.code` so the server-side parser can group them per row
    // (see the E2 closure invocation spec below, which posts the same
    // names). Slice E1 shipped the input surface; Slice E2 wired the
    // real $closure handler.
    await page.goto("/ui/hts/operations?op=closure&resource=");
    await expect(page.locator("input[name='name']")).toBeVisible();
    await expect(
      page.locator("input[name='name']"),
    ).toHaveAttribute("required", "");
    // At least one coding row (system + code inputs) is present on
    // first load.
    await expect(
      page.locator("input[name='concept.system']").first(),
    ).toBeVisible();
    await expect(
      page.locator("input[name='concept.code']").first(),
    ).toBeVisible();
  });

  test("VS Validate free-scope input renders the three-way ValueSet source selector", async ({
    page,
  }) => {
    // §7.6 F3: in `free` scope the VS Validate input must let the
    // operator pick between canonical URL / instance id / inline JSON.
    // The three inputs share `role="radio"` inside a single fieldset.
    await page.goto("/ui/hts/operations?op=validate-code&resource=ValueSet");
    const sources = page.getByRole("radio");
    // At least three source-selector radios are rendered (canonical /
    // instance / inline); the actual labels resolve from
    // `hts-vs-validate-source-*` Fluent keys.
    expect(await sources.count()).toBeGreaterThanOrEqual(3);
  });

  test("batch-validate input surfaces the target ValueSet + a repeatable row form", async ({
    page,
  }) => {
    // §7.6 F1=D: batch-validate is UI-fabricated fan-out over
    // $validate-code. Slice E1 renders the input surface; the seed
    // handler currently returns a not-implemented outcome (per
    // tests/operations.rs). This spec asserts only on the input shell.
    // The target-ValueSet field is `name="target"` and each seed row
    // uses `name="row.code" / row.system / row.display` so the server
    // parser can group them (§7.6 F1 collect_batch_rows).
    await page.goto(
      "/ui/hts/operations?op=batch-validate&resource=ValueSet",
    );
    // A target-ValueSet input (canonical URL or instance id) is
    // present at the top of the form.
    await expect(page.locator("input[name='target']")).toBeVisible();
    // At least one seed row is present (code input).
    await expect(
      page.locator("input[name='row.code']").first(),
    ).toBeVisible();
  });
});

test.describe("HTS Operations workbench a11y + nav (§7.6, §7.10)", () => {
  test("the nav bar exposes the Operations entry after ConceptMaps", async ({
    page,
  }) => {
    await page.goto("/ui/hts");
    // The nav order (dashboard, code systems, value sets, concept
    // maps, operations) is asserted end-to-end so a future addition
    // slots in cleanly.
    const navLinks = page
      .locator("nav")
      .getByRole("link")
      .filter({ hasText: /Dashboard|Code|Value|Concept|Operations/ });
    const names = await navLinks.allTextContents();
    const operationsIdx = names.findIndex((n) => /Operations/i.test(n));
    const conceptMapsIdx = names.findIndex((n) => /Concept/i.test(n));
    expect(operationsIdx).toBeGreaterThan(-1);
    expect(conceptMapsIdx).toBeGreaterThan(-1);
    expect(
      operationsIdx,
      "Operations must come after ConceptMaps in the nav",
    ).toBeGreaterThan(conceptMapsIdx);
  });

  test("HTMX-driven input swaps target #hts-workbench-input, not the shell", async ({
    page,
  }) => {
    // §7.6 F15: the shared workbench ids `#hts-workbench-input` and
    // `#hts-workbench-result` are the swap targets both from op-
    // selector clicks and from the resource-family tab strip. The
    // shell heading + closure banner must not re-render on op swap.
    await page.goto("/ui/hts/operations?op=lookup&resource=CodeSystem");
    const heading = page.getByRole("heading", { level: 1 });
    const originalHeading = await heading.textContent();
    await page.getByRole("link", { name: "$subsumes", exact: true }).click();
    await expect(page.locator("#hts-workbench-input")).toBeVisible();
    await expect(heading).toHaveText(originalHeading ?? "");
  });
});

// ─── Slice E2 additions — real invocations behind the E1 shell ──────────
//
// The describe blocks above cover Slice E1: the workbench shell shape,
// the seven-op selector, closure banner + threshold panel gating,
// resource-family tabs, and the widened per-op input surfaces. Slice
// E2 wired the real handlers for `$closure`, VS `$validate-code`, and
// the batch-validate fan-out, so the specs below assert the submit →
// result behavior end-to-end against the shared hts binary.
//
// Fixture context (`boot.mjs`):
//   - Spins up the `hts` binary against an empty SQLite (no bootstrap
//     dir, no ValueSets seeded). Every upstream call the UI makes
//     proxies back into the same-process HTS terminology server, so
//     unknown ValueSets / closure tables surface as `OperationOutcome`
//     or degraded partials. These specs assert on the *shape* of the
//     response (outcome partial rendered inline, workbench-result
//     region populated, htmx polling attributes correct) rather than
//     on a specific happy-path body.
//   - Uses single-worker sequential mode (`fullyParallel: false`,
//     `workers: 1`); Slice E2's batch job store is process-global so
//     interleaving would otherwise cause id collisions.
//   - `page.request.post(...)` is used for pre-flight shape checks
//     because it bypasses browser HTML5 validation on `required`
//     fields; interactive DOM-state checks still go through
//     `page.goto` + `fill` + `click` so we can key off role/label
//     selectors instead of raw HTML strings.

test.describe("HTS Operations $closure invocation (§7.6 E2)", () => {
  test("submitting closure with an empty `name` renders the OperationOutcome partial inline", async ({
    page,
  }) => {
    // §7.6 F6 pre-flight: `name` (closure table identifier) is
    // required; an empty submit must NOT reach HTS and must render
    // the shared outcome partial into #hts-workbench-result. Mirrors
    // the invalid-input arm exercised by `tests/operations_e2.rs`.
    const response = await page.request.post(
      "/ui/hts/operations/closure",
      {
        headers: {
          "HX-Request": "true",
          "Content-Type": "application/x-www-form-urlencoded",
        },
        data: "name=",
      },
    );
    expect(response.status()).toBe(200);
    const html = await response.text();
    // The shared error partial re-emits the wrapping workbench id.
    expect(html).toContain("hts-workbench-result");
    // hts-outcome.html renders `<aside class="hts-outcome…" role=
    // "alert">` for severity=error, which the invalid-input pre-flight
    // always is.
    expect(
      html,
      "empty-name pre-flight must render the shared hts-outcome error partial",
    ).toMatch(/hts-outcome__code|role="alert"/);
    // Positive: the E1 not-implemented stub is gone — closure is real
    // in E2. Any residual "not implemented" copy would flag a regression.
    expect(html).not.toMatch(/not[-_ ]implemented/i);
  });

  test("submitting closure with `name` + one coding row renders result content in the workbench", async ({
    page,
  }) => {
    // §7.6 F7 happy path: `name` + at least one (system, code) row
    // reaches HTS as POST /ConceptMap/$closure (the E2 verb rule test
    // in `tests/operations_e2.rs` proves the upstream verb). Against
    // the empty SQLite fixture the response is either an
    // OperationOutcome, the degraded banner, or the neutral
    // "empty graph" copy from hts-cm-closure-result.html. Any of
    // those shapes is the wired-up dispatch; the invariant is that
    // #hts-workbench-result is populated with a known result surface.
    const body =
      "name=e2e-closure&concept.system=" +
      encodeURIComponent("http://example.org/cs") +
      "&concept.code=abc";
    const response = await page.request.post(
      "/ui/hts/operations/closure",
      {
        headers: {
          "HX-Request": "true",
          "Content-Type": "application/x-www-form-urlencoded",
        },
        data: body,
      },
    );
    expect(response.status()).toBe(200);
    const html = await response.text();
    expect(html).toContain("hts-workbench-result");
    expect(
      html,
      "closure submit must render one of: edge list, empty-graph, outcome, degraded",
    ).toMatch(
      /Closure edges|No closure edges yet|hts-outcome__code|hts-degraded__title/,
    );
    expect(html).not.toMatch(/not[-_ ]implemented/i);
  });

  test("closure banner (F7) stays visible after a result render", async ({
    page,
  }) => {
    // §7.6 F7 invariant: the stateless-warning banner (`role="status"`
    // with the `hts-op-banner` class) is part of the page shell, not
    // the swap target. A closure submit must swap only
    // #hts-workbench-result and leave the banner in place.
    await page.goto("/ui/hts/operations?op=closure&resource=");
    const banner = page.locator(".hts-op-banner[role='status']");
    await expect(banner).toBeVisible();
    await expect(
      page.getByText(/Closure state lives on the server/i),
    ).toBeVisible();

    await page.getByLabel(/Closure name/i).fill("e2e-banner-stays");
    const responsePromise = page.waitForResponse(
      (r) =>
        r.url().includes("/ui/hts/operations/closure") &&
        r.request().method() === "POST",
    );
    await page.getByRole("button", { name: /Run/i, exact: false }).click();
    await responsePromise;

    // Give htmx a beat to apply the outerHTML swap into
    // #hts-workbench-result before asserting the banner is still there.
    await expect
      .poll(
        async () => {
          const text = await page
            .locator("#hts-workbench-result")
            .innerText()
            .catch(() => "");
          return text.trim().length > 0 ? "populated" : "empty";
        },
        { timeout: 8_000 },
      )
      .toBe("populated");
    await expect(
      banner,
      "closure banner must persist across a result swap",
    ).toBeVisible();
  });
});

test.describe("HTS Operations VS $validate-code widened form (§7.6 F3 E2)", () => {
  test("free-scope input renders three source radios plus all three input surfaces", async ({
    page,
  }) => {
    // §7.6 F3: the ValueSet-source selector is a `<fieldset>` with
    // three radios (`sourceMode` ∈ canonical | instance | inline).
    // Slice E2's current template renders all three paired input
    // surfaces at the same time — selecting a radio only sets
    // `sourceMode` on submit. A JS-driven show/hide of the surfaces
    // is out of scope for E2 (flagged in the Phase 3a output doc).
    await page.goto("/ui/hts/operations?op=validate-code&resource=ValueSet");
    const sourceRadios = page.locator(
      "input[type='radio'][name='sourceMode']",
    );
    expect(await sourceRadios.count()).toBeGreaterThanOrEqual(3);
    // All three paired input surfaces are rendered.
    await expect(page.locator("input[name='sourceCanonical']")).toBeVisible();
    await expect(page.locator("input[name='sourceInstance']")).toBeVisible();
    await expect(page.locator("textarea[name='sourceInline']")).toBeVisible();
  });

  test("each source radio can be selected and clears the other two", async ({
    page,
  }) => {
    // §7.6 F3: source selection is single-choice (native radio group).
    // Slice E2 defaults to `canonical`. Switching to `instance` /
    // `inline` clears the previous check without extra JS.
    await page.goto("/ui/hts/operations?op=validate-code&resource=ValueSet");
    const canonical = page.locator(
      "input[type='radio'][name='sourceMode'][value='canonical']",
    );
    const instance = page.locator(
      "input[type='radio'][name='sourceMode'][value='instance']",
    );
    const inline = page.locator(
      "input[type='radio'][name='sourceMode'][value='inline']",
    );
    await expect(canonical).toBeChecked();
    await instance.check();
    await expect(instance).toBeChecked();
    await expect(canonical).not.toBeChecked();
    await inline.check();
    await expect(inline).toBeChecked();
    await expect(instance).not.toBeChecked();
  });

  test("submitting canonical + code renders a result surface inline", async ({
    page,
  }) => {
    // §7.6 F4 happy path: `sourceMode=canonical` + `code`+`system`
    // fans out to POST /ValueSet/$validate-code (verb-rule test lives
    // in `tests/operations_e2.rs`). Against the empty SQLite fixture
    // the response surfaces as one of: `result=false` neutral badge,
    // an OperationOutcome, or the degraded banner — all valid E2
    // outputs. The invariant is that #hts-workbench-result is
    // populated with a known result-surface class, NOT the E1
    // not-implemented placeholder.
    await page.goto("/ui/hts/operations?op=validate-code&resource=ValueSet");
    await page
      .locator("input[name='sourceCanonical']")
      .fill("http://example.org/ValueSet/e2e");
    await page.locator("input[name='code']").fill("abc");
    await page.locator("input[name='system']").fill("http://example.org/cs");

    const responsePromise = page.waitForResponse(
      (r) =>
        r.url().includes("/ui/hts/operations/validate-code") &&
        r.request().method() === "POST",
    );
    await page.getByRole("button", { name: /Run/i, exact: false }).click();
    await responsePromise;

    const surface = page
      .locator("#hts-workbench-result")
      .locator(".hts-op-workbench__badge, .hts-outcome, .hts-degraded")
      .first();
    await expect(surface).toBeVisible({ timeout: 8_000 });
  });

  test("submitting inline mode with an empty body triggers the pre-flight OperationOutcome", async ({
    page,
  }) => {
    // §7.6 F3 pre-flight: `sourceMode=inline` with an empty
    // `sourceInline` textarea is rejected server-side without
    // burning an HTS round-trip, and the shared outcome partial
    // renders in #hts-workbench-result.
    //
    // NOTE (design-vs-implementation): the parent brief asked for a
    // spec on "invalid JSON" triggering pre-flight. Slice E2's
    // actual pre-flight only rejects an empty inline body — non-
    // empty invalid JSON is passed through to `serde_json::from_str`
    // inside `UpstreamClient::vs_validate_code`, which quietly maps
    // failures to `Value::Null` and lets HTS return its own
    // OperationOutcome. Documented as a design-vs-implementation
    // gap in `edson/docs/hts-ui-phase3a-operations-output.md`.
    const response = await page.request.post(
      "/ui/hts/operations/validate-code",
      {
        headers: {
          "HX-Request": "true",
          "Content-Type": "application/x-www-form-urlencoded",
        },
        data:
          "resource=ValueSet&sourceMode=inline&sourceInline=&mode=code&code=abc",
      },
    );
    expect(response.status()).toBe(200);
    const html = await response.text();
    expect(html).toContain("hts-workbench-result");
    expect(
      html,
      "empty inline body must render the shared hts-outcome partial",
    ).toMatch(/hts-outcome__code|role="alert"/);
  });
});

test.describe("HTS Operations $batch-validate fan-out (§7.6 F1=D E2)", () => {
  test("seed submit returns the skeleton table with per-row + progress htmx polling attributes", async ({
    page,
  }) => {
    // §7.6.1 F1 = D: the seed response is the skeleton table
    // (hts-vs-batch-table.html). Each row carries its per-row
    // `hx-get="/ui/hts/operations/batch-validate/row/{i}?batch_id=…"`
    // with `hx-trigger="load"`, and the sibling progress region polls
    // `…/batch-validate/progress?batch_id=…` on `hx-trigger="load,
    // every 1s"` until the terminal arm fires.
    //
    // NOTE (design-vs-implementation): the parent brief described
    // the progress path as `/ui/hts/operations/batch-progress/<id>`;
    // the actual E2 route is a query-string variant of
    // `/batch-validate/progress`. Flagged in the Phase 3a output doc.
    await page.goto(
      "/ui/hts/operations?op=batch-validate&resource=ValueSet",
    );
    await page
      .locator("input[name='target']")
      .fill("http://example.org/ValueSet/e2e-batch");
    const codeInputs = page.locator("input[name='row.code']");
    const systemInputs = page.locator("input[name='row.system']");
    // Fill two of the three template rows; the third stays blank so
    // the server-side `collect_batch_rows` filter drops it
    // (§7.6 F1 bullet on empty-row elision).
    await codeInputs.nth(0).fill("a");
    await systemInputs.nth(0).fill("http://example.org/cs");
    await codeInputs.nth(1).fill("b");
    await systemInputs.nth(1).fill("http://example.org/cs");

    const responsePromise = page.waitForResponse(
      (r) =>
        r.url().includes("/ui/hts/operations/batch-validate") &&
        r.request().method() === "POST",
    );
    await page.getByRole("button", { name: /Run/i, exact: false }).click();
    const seedResponse = await responsePromise;

    // §7.6.1 F1 = D contract: assert on the seed response BODY, not on
    // the live DOM. The `hts-vs-batch-table.html` skeleton row emits
    // `hx-trigger="load"`, which htmx fires immediately on insertion;
    // the per-row endpoint returns in ~1 poll iteration because the
    // fan-out `tokio::spawn` tasks complete while the seed HTML is
    // still being rendered, so `hx-swap="outerHTML"` destroys each
    // skeleton row (replacing it with `hts-vs-batch-row.html`, same
    // `id`, no `hx-*`) inside ~10–60 ms. Playwright's locator polls
    // at ~100 ms and cannot outrun that window, so DOM-locator
    // assertions on `#hts-batch-row-N` deterministically catch the
    // replacement. The wire body IS the skeleton contract — see
    // edson/docs/hts-ui-ops531-diagnosis.md.
    expect(seedResponse.status()).toBe(200);
    const seedHtml = await seedResponse.text();

    // Skeleton row 0: per-row polling target, aria-busy, load trigger.
    expect(
      seedHtml,
      'skeleton row 0 must carry aria-busy="true"',
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
      'skeleton row 0 must fire hx-trigger="load"',
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

    // Progress region: `hx-get` targets `…/batch-validate/progress?batch_id=…`
    // and `hx-trigger` includes the recurring `every Ns` interval.
    // Interval is intentionally NOT hardcoded — Phase 3b may tune it.
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

    // TODO(phase-3b): once local polling timings are confirmed, extend
    // this spec to assert that (a) each `#hts-batch-row-N` sheds its
    // `aria-busy` attribute after the per-row endpoint drains, and
    // (b) `#hts-batch-progress` reaches the terminal arm through the
    // browser-side htmx polling loop (not just the raw `page.request`
    // poll covered by the sibling spec below).
  });

  test("progress endpoint reaches the terminal (no `hx-trigger`) arm after fan-out drains", async ({
    page,
  }) => {
    // §7.6.1 F1 = D terminal-state contract: once every row completes,
    // hts-vs-batch-progress.html omits the `hx-trigger` polling
    // attribute so htmx stops polling. This is the lighter
    // "aggregated result" assertion the parent brief suggested — a
    // full result-replacement check driven through the browser's
    // htmx loop is deferred to Phase 3b (see the TODO in the sibling
    // spec above). Mirrors the Rust hook
    // `batch_progress_terminal_state_stops_polling` in
    // `tests/operations_e2.rs`.
    const seed = await page.request.post(
      "/ui/hts/operations/batch-validate",
      {
        headers: {
          "HX-Request": "true",
          "Content-Type": "application/x-www-form-urlencoded",
        },
        data: [
          "resource=ValueSet",
          `target=${encodeURIComponent("http://example.org/vs")}`,
          "row.code=a",
          `row.system=${encodeURIComponent("http://example.org/cs")}`,
        ].join("&"),
      },
    );
    expect(seed.status()).toBe(200);
    const seedHtml = await seed.text();
    const match = seedHtml.match(
      /batch-validate\/row\/0\?batch_id=([^"& \n]+)/,
    );
    expect(match, "seed response must embed a batch id").not.toBeNull();
    const batchId = match![1];

    // Poll the progress endpoint through the raw request API so we
    // don't depend on the DOM-side htmx swap loop. `expect.poll`
    // retries with a generous deadline; Phase 3b may need to raise
    // this on Windows CPU-loaded hosts.
    await expect
      .poll(
        async () => {
          const poll = await page.request.get(
            `/ui/hts/operations/batch-validate/progress?batch_id=${batchId}`,
          );
          const body = await poll.text();
          return body.includes("hx-trigger") ? "polling" : "done";
        },
        {
          timeout: 15_000,
          intervals: [200, 400, 800, 1_200],
          message:
            "batch progress must reach the terminal state within 15s (Phase 3b: raise if Windows CPU load is high)",
        },
      )
      .toBe("done");
  });
});
