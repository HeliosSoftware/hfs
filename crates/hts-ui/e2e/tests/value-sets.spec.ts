import { expect, test } from "@playwright/test";

// Phase 2 Slice C: ValueSet browser + detail with embedded `$expand`
// workbench. Mirrors code-systems.spec.ts: walks the browser filter form,
// the tabs pattern on the detail page, the Expand workbench input +
// result panel (both flat and tree modes), the too-costly banner, and
// the shell-outcome path for unknown ids.
//
// Boot fixture: the e2e/boot.mjs script starts `hts` with a seed
// ValueSet that expands to a small, deterministic set. The tests assume
// these seed identifiers â€” see e2e/README.md for the required fixtures:
//
//   - id="ex-vs-1"                                   (flat expansion)
//   - url="http://example.org/vs/limbs"              (canonical URL)
//   - version="1.0.0"
//   - status="active"
//   - expansion contains at least 30 concepts, enough for [Load more]
//     to fire under the default `count=50` (adjust the test if the
//     future seed uses a different default).
//   - id="ex-vs-tree" containing a small hierarchical expansion so the
//     tree-mode tab renders `role="tree"` and the `showing full tree`
//     leaf-count label.
//   - id="ex-vs-too-costly" whose $expand deliberately trips HTS's
//     `HTS_MAX_EXPANSION_SIZE` and returns 422 with a `too-costly`
//     OperationOutcome â€” the banner + Raise-threshold form are asserted.

test.describe("HTS ValueSet browser (Â§7.4)", () => {
  test("renders the browser heading and status pill at /ui/hts/value-sets", async ({
    page,
  }) => {
    const response = await page.goto("/ui/hts/value-sets");
    expect(response?.status(), "browser route must respond 200").toBe(200);
    await expect(
      page.getByRole("heading", { name: "ValueSets", exact: true, level: 1 }),
    ).toBeVisible();
    // The seed VS is active, so the browser table's first row shows an
    // "active" pill translated by the Fluent catalog.
    await expect(
      page.getByRole("cell").filter({ hasText: "active" }).first(),
    ).toBeVisible();
  });

  test("filter input debounces and swaps rows via htmx", async ({ page }) => {
    await page.goto("/ui/hts/value-sets");
    // Type into the URL filter; htmx `hx-trigger="input changed delay:300ms"`
    // waits ~300ms before firing the swap. A VS whose canonical URL does
    // not contain "no-such-valueset" must not survive the filter.
    await page
      .getByLabel("Canonical URL", { exact: false })
      .fill("no-such-valueset");
    await expect(
      page.getByText("No ValueSets match these filters."),
    ).toBeVisible({ timeout: 3_000 });
    // Reset returns us to the full listing (empty-anchor href navigation).
    await page.getByRole("link", { name: "Reset", exact: true }).click();
    await expect(
      page.getByRole("cell", { name: "http://example.org/vs/limbs" }),
    ).toBeVisible();
  });
});

test.describe("HTS ValueSet detail + $expand workbench (Â§7.4)", () => {
  test("landing on /ui/hts/value-sets/{id} shows Metadata tab active", async ({
    page,
  }) => {
    const response = await page.goto("/ui/hts/value-sets/ex-vs-1");
    expect(response?.status()).toBe(200);
    await expect(
      page.getByRole("tab", { name: "Metadata", exact: true }),
    ).toHaveAttribute("aria-selected", "true");
    // Identity section renders the seed VS's canonical URL.
    await expect(
      page.getByText("http://example.org/vs/limbs"),
    ).toBeVisible();
    // Â§7.4.1 F9: NO Validate tab in Slice C.
    await expect(
      page.getByRole("tab", { name: "Validate", exact: true }),
    ).toHaveCount(0);
  });

  test("clicking the Expand tab swaps in the workbench input via htmx", async ({
    page,
  }) => {
    await page.goto("/ui/hts/value-sets/ex-vs-1");
    await page.getByRole("tab", { name: "Expand", exact: true }).click();
    // The GET /expand handler renders the input partial. The Run button
    // is Fluent-resolved from hts-workbench-run ("Run" in en). The
    // Advanced <details> panel exposes the threshold input.
    await expect(
      page.getByRole("button", { name: "Run", exact: true }),
    ).toBeVisible({ timeout: 3_000 });
    await expect(
      page.getByRole("group", { name: /Mode/i }),
    ).toBeVisible();
  });

  test("running $expand in flat mode renders a table with concept rows", async ({
    page,
  }) => {
    await page.goto("/ui/hts/value-sets/ex-vs-1/expand");
    await page.getByRole("button", { name: "Run", exact: true }).click();
    // The workbench result panel is aria-live="polite"; after the POST
    // the flat table renders with a code column.
    const result = page.locator("#hts-workbench-result");
    await expect(result.locator("table.hts-vs-workbench__flat")).toBeVisible({
      timeout: 3_000,
    });
    // The seed expansion has enough concepts that the flat pager fires.
    await expect(
      result.getByRole("button", { name: "Load more", exact: true }),
    ).toBeVisible();
  });

  test("switching to tree mode renders role=tree and hides the pager", async ({
    page,
  }) => {
    await page.goto("/ui/hts/value-sets/ex-vs-tree/expand");
    // Â§7.4.1 F7: tree â‡’ hierarchical=true, flat â‡’ excludeNested=true.
    await page.getByLabel("Tree", { exact: true }).check();
    await page.getByRole("button", { name: "Run", exact: true }).click();
    const result = page.locator("#hts-workbench-result");
    await expect(result.locator('[role="tree"]')).toBeVisible({
      timeout: 3_000,
    });
    // Â§7.4.1 F10: tree mode hides the pager and shows the leaf-count label.
    await expect(
      result.getByRole("button", { name: "Load more", exact: true }),
    ).toHaveCount(0);
    await expect(result).toContainText(/showing full tree/i);
  });

  test("a too-costly expansion renders the banner with the Raise form", async ({
    page,
  }) => {
    await page.goto("/ui/hts/value-sets/ex-vs-too-costly/expand");
    // HTS only enforces `HTS_MAX_EXPANSION_SIZE` when the request omits
    // `count` (see crates/hts/src/backends/sqlite/value_set.rs — the
    // too-costly gate is guarded by `if req.count.is_none()`). The
    // workbench Advanced panel defaults `count=50`, so unless we clear
    // it the request would resolve to a bounded page and never trip the
    // banner. Emptying the input mirrors the "expand everything" intent
    // the too-costly banner is designed to gate.
    await page.locator('input[name="count"]').fill("");
    await page.getByRole("button", { name: "Run", exact: true }).click();
    const result = page.locator("#hts-workbench-result");
    await expect(
      result.locator(".hts-vs-workbench__too-costly"),
    ).toBeVisible({ timeout: 3_000 });
    // The banner exposes a compact Raise-threshold form whose hidden
    // `threshold` field is the same key the Advanced panel writes to.
    await expect(
      result.locator(".hts-vs-workbench__too-costly-form"),
    ).toBeVisible();
    await expect(
      result.locator('input[name="threshold"]'),
    ).toBeVisible();
  });

  test("a soft-deleted or unknown VS id renders an outcome partial inside the shell", async ({
    page,
  }) => {
    const response = await page.goto("/ui/hts/value-sets/does-not-exist");
    // Â§7.4.1 invariant #5: 200 with an OperationOutcome partial rather
    // than a page 404 (mirrors the CS soft-delete contract).
    expect(response?.status()).toBe(200);
    await expect(page.locator(".hts-outcome--error")).toBeVisible();
  });
});
