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
    // in the Slice B detail-page embed.
    await expect(
      page.getByLabel(/Supplement/i).first(),
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
    // omits the panel entirely.
    await page.goto("/ui/hts/operations?op=lookup&resource=CodeSystem");
    await expect(
      page.locator("input[name='threshold']"),
    ).toHaveCount(0);
    await page.goto("/ui/hts/operations?op=expand&resource=ValueSet");
    await expect(
      page.locator("input[name='threshold']"),
    ).toBeVisible();
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
    await expect(
      workbench.locator("input[name='threshold']"),
    ).toBeVisible({ timeout: 3_000 });
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
    // system+code rows. Slice E1 ships the input surface; the run
    // handler is stubbed until Slice E2.
    await page.goto("/ui/hts/operations?op=closure&resource=");
    await expect(page.locator("input[name='name']")).toBeVisible();
    await expect(
      page.locator("input[name='name']"),
    ).toHaveAttribute("required", "");
    // At least one coding row (system + code inputs) is present on
    // first load.
    await expect(page.locator("input[name='system']").first()).toBeVisible();
    await expect(page.locator("input[name='code']").first()).toBeVisible();
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
    await page.goto(
      "/ui/hts/operations?op=batch-validate&resource=ValueSet",
    );
    // A target-ValueSet input (canonical URL or instance id) is
    // present at the top of the form.
    await expect(
      page.locator("input[name='url'], input[name='valueSet']").first(),
    ).toBeVisible();
    // At least one seed row is present (code input).
    await expect(page.locator("input[name='code']").first()).toBeVisible();
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
