import { expect, test } from "@playwright/test";

// Phase 2 Slice D: ConceptMap browser + detail with an embedded
// `$translate` workbench. Mirrors value-sets.spec.ts: walks the browser
// filter form, the tabs pattern on the detail page, the Translate
// workbench input + result panel (both forward and reverse directions),
// the shell-outcome path for unknown ids, and the §7.5 "no-matches"
// neutral state.
//
// Boot fixture: the `boot.mjs` harness spawns `hts` against a throwaway
// SQLite DB. Slice D adds a ConceptMap seed to the roster below;
// `boot.mjs` does NOT create these today, so specs will 404 until the
// Slice G seed loader lands (see e2e/README.md).
//
// Required seed identifiers:
//
//   - id="ex-cm-1"                                (forward-direction map)
//   - url="http://example.org/cm/example"         (canonical URL)
//   - version="1.0.0"
//   - status="active"
//   - sourceUri="http://example.org/vs/source"
//   - targetUri="http://example.org/vs/target"
//   - at least one mapping group whose source concept `A` in
//     `http://example.org/cs/source` maps to target concept `T1` in
//     `http://example.org/cs/target` with equivalence `equivalent`.
//
//   - id="ex-cm-no-match"                         (§7.5 F11 realized:
//     any well-formed translate request returns HTTP 200 with
//     `result=false` — used to prove the no-matches neutral state).

test.describe("HTS ConceptMap browser (§7.5)", () => {
  test("renders the browser heading and status pill at /ui/hts/concept-maps", async ({
    page,
  }) => {
    // `_count=100` (== MAX_COUNT) so the seed row lands on the first page
    // regardless of how much bootstrap terminology HTS added to the ledger
    // (Slice B invariant #1 clamps at 100 without a hard 400). Scoping to
    // the seed row via the linkified `Name` cell is what makes this test
    // pagination-independent: whichever page the bootstrap noise pushes
    // `ex-cm-1` onto, `_count=100` keeps it visible, and the row link is
    // the only stable selector that survives the CM browser filter's
    // known "does not narrow" backend bug (see Source system test below).
    const response = await page.goto("/ui/hts/concept-maps?_count=100");
    expect(response?.status(), "browser route must respond 200").toBe(200);
    await expect(
      page.getByRole("heading", { name: "ConceptMaps", exact: true, level: 1 }),
    ).toBeVisible();
    // The seed CM is active, so its row shows an "active" pill translated
    // by the Fluent catalog. Scope to the seed row via its detail-page
    // link so bootstrap-driven pagination noise cannot mask the check.
    const seedRow = page
      .getByRole("row")
      .filter({
        has: page.locator("a[href='/ui/hts/concept-maps/ex-cm-1']"),
      });
    await expect(seedRow).toHaveCount(1);
    await expect(
      seedRow.getByRole("cell").filter({ hasText: "active" }),
    ).toBeVisible();
  });

  test("filter input debounces and swaps rows via htmx", async ({ page }) => {
    await page.goto("/ui/hts/concept-maps");
    // Type into the URL filter; htmx `hx-trigger="input changed delay:300ms"`
    // waits ~300ms before firing the swap. A CM whose canonical URL does
    // not contain "no-such-conceptmap" must not survive the filter.
    await page
      .getByLabel("Canonical URL", { exact: false })
      .fill("no-such-conceptmap");
    await expect(
      page.getByText("No ConceptMaps match these filters."),
    ).toBeVisible({ timeout: 3_000 });
    // Reset returns us to the full listing (empty-anchor href navigation).
    // The critical guarantee is that the empty state clears — we do NOT
    // assert on any specific row's URL because (a) bootstrap terminology
    // may push the seed CM off the default first page and (b) the CM
    // browser's server-side title / URL filters currently do not narrow
    // the row set (pinned by the Source system filter test below). Assert
    // the empty-state banner is gone and at least one data row is in the
    // tbody so the "swap-back" path is exercised end to end.
    await page.getByRole("link", { name: "Reset", exact: true }).click();
    await expect(
      page.getByText("No ConceptMaps match these filters."),
    ).toHaveCount(0);
    await expect(page.locator("#hts-cm-rows tbody > tr").first()).toBeVisible();
  });

  test("Source system filter routes to the `source` FHIR param without narrowing the table", async ({
    page,
  }) => {
    // Pins §3.5 demo step 2: the rail advertises a Source system filter
    // that maps to the FHIR `source-uri` canonical search parameter. HTS
    // currently drops the param (backend bug tracked in helios-hts), but
    // the rail must still (a) send it on the wire so a future backend
    // fix light-switches the filter on, (b) echo the typed value back so
    // the operator can see what they asked for, and (c) leave the row
    // set untouched (no accidental client-side narrowing).
    await page.goto("/ui/hts/concept-maps");
    // Scope to the rail input by id: the Mapping cell in every browser
    // row also carries `<span aria-label="Source system">S:</span>`, so
    // an accessible-name locator would match 1 + N elements. `#filter-
    // source` is the rail's stable anchor (cm-browser.html).
    const railSource = page.locator("#filter-source");
    const rowsUrl = /\/ui\/hts\/concept-maps\/rows\b/;
    const requestPromise = page.waitForRequest(rowsUrl, { timeout: 3_000 });
    await railSource.fill("http://example.org/cs/source");
    const request = await requestPromise;
    // (a) `source=` is on the wire, URL-encoded exactly as the backend
    // would expect if it wired the param through.
    expect(request.url()).toContain(
      "source=http%3A%2F%2Fexample.org%2Fcs%2Fsource",
    );
    // (b) After htmx swaps the rows partial, the rail input keeps its
    // typed value (the rail form is outside the swap target).
    await expect(railSource).toHaveValue("http://example.org/cs/source");
    // (c) The table does NOT narrow to empty. HTS ignores the param
    // today, so whichever rows the first page contained pre-filter are
    // still there — the empty-state marker must not appear. This is
    // pagination-independent (works whether the seed CM lands on the
    // first page or not, since the bootstrap ledger adds N > 25 CMs).
    // If the backend ever starts honoring `source-uri`, this expectation
    // will flip (only maps whose sourceUri matches would survive) and
    // this test needs updating alongside the demo doc.
    await expect(
      page.getByText("No ConceptMaps match these filters."),
    ).toHaveCount(0);
  });
});

test.describe("HTS ConceptMap detail + $translate workbench (§7.5)", () => {
  test("landing on /ui/hts/concept-maps/{id} redirects to /translate with Translate tab active", async ({
    page,
  }) => {
    // §8.3 operation-first landing: the naked `/{id}` URL 308-redirects
    // to `/{id}/translate`; Playwright follows the redirect transparently.
    // The Metadata tab was retired — the facts block stays visible above
    // the tab strip regardless of which operation is active.
    const response = await page.goto("/ui/hts/concept-maps/ex-cm-1");
    expect(response?.status()).toBe(200);
    expect(page.url()).toContain("/ui/hts/concept-maps/ex-cm-1/translate");
    await expect(
      page.getByRole("tab", { name: "Translate", exact: true }),
    ).toHaveAttribute("aria-selected", "true");
    // Facts block above the tab strip renders the seed CM's canonical
    // URL (Identity section) and target URI (Mapping section). The
    // canonical URL is intentionally echoed again inside the translate
    // input's "ConceptMap (fixed)" pinned-map header so the operator sees
    // which map the workbench is scoped to; scope this assertion to the
    // Identity section via its aria-labelledby heading so the two legit
    // occurrences do not collide under strict mode. The target URI only
    // renders inside the Mapping section, so the bare locator is unique.
    await expect(
      page.getByLabel("Identity").getByText("http://example.org/cm/example"),
    ).toBeVisible();
    await expect(
      page.getByText("http://example.org/vs/target"),
    ).toBeVisible();
    // §8.3 + §7.5: only Translate tab in the strip — no Metadata /
    // Lookup / Validate / Expand / Subsumes leak in from Slice B / C.
    for (const wrong of ["Metadata", "Lookup", "Validate", "Subsumes", "Expand"]) {
      await expect(
        page.getByRole("tab", { name: wrong, exact: true }),
      ).toHaveCount(0);
    }
  });

  test("clicking the Translate tab keeps its aria-current highlight", async ({
    page,
  }) => {
    // §8.3: Translate is the only tab on CM detail, and lands active on
    // the naked `/{id}` URL after the redirect chain. Clicking it is a
    // no-op self-click; the important guarantee is that the run button
    // is available and the tab keeps its highlight.
    await page.goto("/ui/hts/concept-maps/ex-cm-1");
    await page.getByRole("tab", { name: "Translate", exact: true }).click();
    // The GET /translate handler renders the input partial with the
    // direction toggle defaulting to Forward.
    await expect(
      page.getByRole("button", { name: "Translate", exact: true }),
    ).toBeVisible({ timeout: 3_000 });
    await expect(
      page.getByRole("group", { name: /Direction/i }),
    ).toBeVisible();
    await expect(page.getByLabel("Forward", { exact: true })).toBeChecked();
    // §7.5: `version` / `dependency` / `targetsystem` (lowercase) must
    // never surface in the form.
    for (const forbidden of ['name="version"', 'name="dependency"', 'name="targetsystem"']) {
      const count = await page.locator(`[${forbidden.replace('=', "='").replace('"', "").replace('"', "")}']`).count().catch(() => 0);
      expect(count, `${forbidden} must not appear in the Translate form`).toBe(0);
    }
  });

  test("running forward $translate renders a match grid with an Equivalence or Relationship column", async ({
    page,
  }) => {
    await page.goto("/ui/hts/concept-maps/ex-cm-1/translate");
    // Forward direction requires code + system to make it past the
    // pre-flight validation gate.
    await page.getByLabel("Code", { exact: true }).fill("A");
    await page
      .getByLabel("System", { exact: true })
      .first()
      .fill("http://example.org/cs/source");
    await page
      .getByRole("button", { name: "Translate", exact: true })
      .click();
    const result = page.locator("#hts-workbench-result");
    await expect(
      result.locator("table.hts-cm-workbench__matches"),
    ).toBeVisible({ timeout: 3_000 });
    // §7.5 a11y contract: the mapping column's heading reflects whichever
    // field name HTS returned — either "Equivalence" (R4/R4B) or
    // "Relationship" (R5/R6). We don't know which the seed HTS emits at
    // this call site, so accept either.
    const headings = result.getByRole("columnheader");
    await expect(headings.filter({ hasText: /Equivalence|Relationship/ })).toHaveCount(1);
  });

  test("reverse without targetCode surfaces the inline validation outcome without calling HTS", async ({
    page,
  }) => {
    await page.goto("/ui/hts/concept-maps/ex-cm-1/translate");
    await page.getByLabel("Reverse", { exact: true }).check();
    // The reverse-mode partial swaps in via htmx; wait for `Target code`
    // to become visible before submitting empty.
    await expect(
      page.getByLabel("Target code", { exact: true }),
    ).toBeVisible({ timeout: 3_000 });
    await page
      .getByRole("button", { name: "Translate", exact: true })
      .click();
    // §7.5 pre-flight gate: an OperationOutcome renders in the result
    // region and no HTS round-trip happens.
    await expect(page.locator(".hts-outcome--error")).toBeVisible({
      timeout: 3_000,
    });
  });

  test("direction toggle emits exactly one `direction=` param on the wire (CM:139 pin)", async ({
    page,
  }) => {
    // Pins §3.5's Reverse red flag and the diagnosis captured in
    // hts-cm-translate-input.html lines 60-72: without `hx-params="none"`
    // on the direction radios, htmx serialises the radio's own form
    // value onto the GET URL, colliding with the literal ?direction=…
    // already in `hx-get` and yielding `direction=reverse&direction=reverse`.
    // axum then rejects with HTTP 400, htmx swaps nothing, and the
    // reverse fieldset never lands. Assert the wire directly.
    await page.goto("/ui/hts/concept-maps/ex-cm-1/translate");
    const translateGet = (dir: "forward" | "reverse") =>
      page.waitForRequest(
        (req) =>
          req.method() === "GET" &&
          /\/ui\/hts\/concept-maps\/ex-cm-1\/translate\b/.test(req.url()) &&
          req.url().includes(`direction=${dir}`),
        { timeout: 3_000 },
      );

    // Reverse: exactly one direction=reverse, and the partial lands.
    const reversePromise = translateGet("reverse");
    await page.getByLabel("Reverse", { exact: true }).check();
    const reverseReq = await reversePromise;
    const reverseMatches =
      reverseReq.url().match(/(?:^|[?&])direction=/g) || [];
    expect(
      reverseMatches.length,
      `expected 1 direction= in ${reverseReq.url()}`,
    ).toBe(1);
    expect(reverseReq.url()).toContain("direction=reverse");
    expect(reverseReq.url()).not.toContain(
      "direction=reverse&direction=reverse",
    );
    await expect(
      page.getByLabel("Target code", { exact: true }),
    ).toBeVisible({ timeout: 3_000 });

    // Forward round-trip: symmetric guarantee.
    const forwardPromise = translateGet("forward");
    await page.getByLabel("Forward", { exact: true }).check();
    const forwardReq = await forwardPromise;
    const forwardMatches =
      forwardReq.url().match(/(?:^|[?&])direction=/g) || [];
    expect(
      forwardMatches.length,
      `expected 1 direction= in ${forwardReq.url()}`,
    ).toBe(1);
    expect(forwardReq.url()).toContain("direction=forward");
    expect(forwardReq.url()).not.toContain(
      "direction=forward&direction=forward",
    );
    await expect(
      page.getByLabel("Code", { exact: true }),
    ).toBeVisible({ timeout: 3_000 });
  });

  test("reverse translate with targetCode + targetSystem renders a match", async ({
    page,
  }) => {
    // Pins the §3.5 demo instruction (reverse happy-path). ex-cm-1
    // maps source A → target T1 with equivalence=equivalent, so a
    // reverse translate on T1 + the target CS must return one match
    // row. Filling both the target-side inputs is what discriminates
    // this from the pre-flight test above.
    await page.goto("/ui/hts/concept-maps/ex-cm-1/translate");
    await page.getByLabel("Reverse", { exact: true }).check();
    await expect(
      page.getByLabel("Target code", { exact: true }),
    ).toBeVisible({ timeout: 3_000 });
    await page
      .getByLabel("Target code", { exact: true })
      .fill("T1");
    await page
      .getByLabel("Target system", { exact: true })
      .fill("http://example.org/cs/target");
    await page
      .getByRole("button", { name: "Translate", exact: true })
      .click();
    const result = page.locator("#hts-workbench-result");
    await expect(
      result.locator(".hts-cm-workbench__result--translate"),
    ).toBeVisible({ timeout: 3_000 });
    await expect(result.locator(".hts-outcome--error")).toHaveCount(0);
    // Backend surfaces the equivalence as a value on the match row.
    await expect(result).toContainText(/equivalent/i);
  });

  test("a translate with no matches renders the neutral state, not the error partial", async ({
    page,
  }) => {
    await page.goto("/ui/hts/concept-maps/ex-cm-no-match/translate");
    await page.getByLabel("Code", { exact: true }).fill("Z");
    await page
      .getByLabel("System", { exact: true })
      .first()
      .fill("http://example.org/cs/source");
    await page
      .getByRole("button", { name: "Translate", exact: true })
      .click();
    // §7.5 F11 realized: HTTP 200 + `result=false` renders the neutral
    // no-matches label, NOT the shared error partial.
    const result = page.locator("#hts-workbench-result");
    await expect(result.locator(".hts-cm-workbench__no-matches")).toBeVisible({
      timeout: 3_000,
    });
    await expect(result.locator(".hts-outcome--error")).toHaveCount(0);
  });

  test("a soft-deleted or unknown CM id renders an outcome partial inside the shell", async ({
    page,
  }) => {
    const response = await page.goto("/ui/hts/concept-maps/does-not-exist");
    // §7.5 (Slice B invariant #5): 200 with an OperationOutcome partial
    // rather than a page 404. Mirrors the CS / VS soft-delete contract.
    expect(response?.status()).toBe(200);
    await expect(page.locator(".hts-outcome--error")).toBeVisible();
  });
});
