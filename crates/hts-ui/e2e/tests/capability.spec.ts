import { expect, test } from "@playwright/test";
import type { Locator } from "@playwright/test";

async function openJsonNode(scope: Locator, key: string): Promise<Locator> {
  const node = scope
    .locator("summary.capability-json-row")
    .filter({ hasText: `"${key}":` })
    .locator("..")
    .first();
  const body = node.locator(":scope > [data-capability-json-body]");
  await node.locator(":scope > summary").click();
  await expect(body.locator("[data-capability-json-page], .json-view").first()).toBeVisible();
  return node;
}

// The Capability & Conformance page at `/ui/hts/capability-statement`.
//
// **Shape of record (2026-08-27).** A mirror of HFS's
// `crates/ui/templates/pages/capability-statement.html`: same sidebar label
// and icon, same route shape, a `.page-head` followed by stacked
// `<section class="card">` blocks. The page was called "Diagnostics" and
// lived at `/ui/hts/diagnostics` until this rename; that path now 308s here.
//
// Mirrors the Rust-side coverage in `crates/hts-ui/tests/capability.rs` but
// exercises what HTML-only Rust tests cannot reach: real layout, aggregate
// expansion behaviour, and heading structure as the
// accessibility tree sees it.
//
// Handler: `crates/hts-ui/src/capability.rs`. Template:
// `templates/pages/capability-statement.html`.
//
// Boot fixture (see e2e/boot.mjs): the Playwright suite boots a real `hts`
// binary against a throwaway SQLite DB with `HTS_UI_ENABLED=1`, so
// `/metadata` and `/metadata?mode=terminology` are served by the same
// in-process HTS and both return 200 during a suite run.

const PATH = "/ui/hts/capability-statement";

// The cards that always render, in order. "System Interactions" is
// deliberately absent: HTS serves `POST /` but declares no
// `rest[].interaction[]`, and the card is omitted rather than rendered
// blank. If HTS ever advertises them, that card appears and the count
// assertion below is the thing that will (correctly) fail.
const CARD_HEADINGS = [
  "Server Summary",
  "Operations",
  "Per-Resource Capabilities",
  "Terminology Capabilities",
];

test.describe("HTS Capability & Conformance shell", () => {
  test("responds at the mirrored path with the page-head + stacked cards", async ({
    page,
  }) => {
    const response = await page.goto(PATH);
    expect(response?.status(), "capability route must respond 200").toBe(200);

    // H1 comes from HFS's own Fluent key `cap-title` — the catalog is
    // shared between the two crates, so the string cannot drift.
    await expect(
      page.getByRole("heading", { name: /Capability Statement/i, level: 1 }),
    ).toBeVisible();
    await expect(page.locator("h1.page-head__title")).toBeVisible();
    await expect(page.locator("p.page-head__lede")).toBeVisible();

    for (const heading of CARD_HEADINGS) {
      await expect(
        page.locator(".card-head h3", { hasText: heading }).first(),
      ).toBeVisible();
    }
  });

  test("the pre-rename path still resolves, as a redirect", async ({ page }) => {
    const response = await page.goto("/ui/hts/diagnostics");
    expect(response?.status(), "the old path must not 404").toBe(200);
    expect(
      new URL(page.url()).pathname,
      "the old path must land on the renamed page",
    ).toBe(PATH);
  });

  test("carries no tab strip and uses the shared raw-tree affordance", async ({ page }) => {
    await page.goto(PATH);
    // The tab era is over: no tablist, no tabs, no tabpanel, no
    // `#diag-panel` swap target.
    await expect(page.getByRole("tablist")).toHaveCount(0);
    await expect(page.getByRole("tab")).toHaveCount(0);
    await expect(page.getByRole("tabpanel")).toHaveCount(0);
    await expect(page.locator("#diag-panel")).toHaveCount(0);
    const raw = page.locator("#capability-json-fold");
    await expect(raw.locator(":scope > .card-head > h3")).toHaveText(/^Raw CapabilityStatement/);
    await expect(raw.locator(":scope > .card-head > [data-capability-json-actions]")).toBeVisible();
    await expect(raw.locator("[data-capability-json-tree] > [data-path='']")).toBeVisible();
  });

  test("nav exposes HFS's own label, after Import", async ({ page }) => {
    await page.goto("/ui/hts");
    const navLinks = page
      .locator("nav")
      .getByRole("link")
      .filter({ hasText: /Home|Code|Value|Concept|Import|Capability/ });
    const names = await navLinks.allTextContents();
    const importIdx = names.findIndex((n) => /Import/i.test(n));
    const capIdx = names.findIndex((n) => /Capability & Conformance/i.test(n));
    expect(importIdx, "Import must be present in the sidebar nav").toBeGreaterThan(-1);
    expect(
      capIdx,
      "the sidebar must use HFS's `Capability & Conformance` label",
    ).toBeGreaterThan(-1);
    expect(
      capIdx,
      "Capability & Conformance must appear after Import in the sidebar nav",
    ).toBeGreaterThan(importIdx);
    expect(
      names.some((n) => /Diagnostics/i.test(n)),
      "no nav entry should still say Diagnostics",
    ).toBe(false);
  });
});

test.describe("HTS Capability & Conformance card content", () => {
  test("the Server Summary card renders detail__field facts", async ({ page }) => {
    await page.goto(PATH);
    const card = page.locator("section.card").first();
    await expect(card.locator(".detail__field").first()).toBeVisible();
    // HFS's fact contract: the label is the single direct <span> child
    // (`.detail__field > span` uppercases it) and the value is a <div> or a
    // <code> — never a second span. Seven rows, HFS's field set exactly.
    await expect(card.locator(".detail__field > span")).toHaveCount(7);
    await expect(card).toContainText(/FHIR version/i);
  });

  test("tables render as table-wrap + data-table", async ({ page }) => {
    await page.goto(PATH);
    // Two table cards: Operations and Per-Resource Capabilities. Both
    // scroll inside their own `.table-wrap` so the page body never scrolls
    // horizontally.
    await expect(page.locator("section.card.table-card")).toHaveCount(2);
    await expect(page.locator(".table-wrap > table.data-table")).toHaveCount(2);
    // Each table has content or an explicit empty row — never a blank tbody.
    for (let i = 0; i < 2; i += 1) {
      await expect(
        page.locator("table.data-table").nth(i).locator("tbody tr"),
      ).not.toHaveCount(0);
    }
    // HTS advertises its terminology operations at the system level.
    await expect(page.locator("table.data-table").first()).toContainText("$lookup");
  });

  test("the Terminology card declares capabilities, not identity", async ({
    page,
  }) => {
    await page.goto(PATH);
    const card = page.locator("section.card").filter({
      has: page.locator(".card-head h3", { hasText: "Terminology Capabilities" }),
    });
    // What the card is for: what $expand supports, and which parameters it
    // accepts (rendered as `.tag` chips, HFS's own primitive).
    await expect(card).toContainText(/Hierarchical expansion/i);
    await expect(card).toContainText(/Expansion paging/i);
    await expect(card.locator("span.tag").first()).toBeVisible();
    // The count links to the browser that actually lists the systems.
    await expect(card.locator('a[href="/ui/hts/code-systems"]')).toBeVisible();
    // What it must NOT repeat: the identity block already shown by the
    // Server Summary card above it. These labels belong to that card only.
    await expect(card).not.toContainText(/^\s*Title\s*$/im);
    await expect(card).not.toContainText(/Base URL/i);
  });

  test("the raw statement starts at the first level and expands through one POST", async ({ page }) => {
    test.setTimeout(120_000);
    await page.goto(PATH);
    const card = page.locator("#capability-json-fold");
    await expect(card.locator(".addbox")).toHaveCount(0);
    const region = card.locator("#capability-json-body");
    const tree = card.locator("[data-capability-json-tree]");
    const initial = await tree.innerHTML();
    await expect(region).toHaveAttribute("role", "region");
    await expect(region).toHaveAttribute("aria-labelledby", "capability-json-heading");
    await expect(region).toHaveAttribute("tabindex", "0");
    await expect(region).toHaveCSS("overflow-y", "auto");
    expect(parseFloat(await region.evaluate((node) => getComputedStyle(node).maxHeight))).toBeGreaterThan(0);
    await expect(tree.locator(":scope > [data-path=''][data-offset='0']")).toBeVisible();
    await expect(tree.locator("details[open]")).toHaveCount(0);

    const requests: string[] = [];
    let release!: () => void;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    page.on("request", (request) => {
      if (request.method() === "POST" && new URL(request.url()).pathname.endsWith("/json-expand")) {
        requests.push(request.postData() ?? "");
      }
    });
    await page.route("**/ui/hts/capability-statement/json-expand", async (route) => {
      await gate;
      await route.continue();
    });
    await card.locator("[data-capability-json-expand-all]").click();
    await expect(region).toHaveAttribute("aria-busy", "true");
    await expect(card.locator("[data-capability-json-status]")).toContainText(/Expanding/i);
    await expect(card.locator("[data-capability-json-collapse-all]")).toBeEnabled();
    expect(requests).toHaveLength(1);
    release();
    await expect(region).not.toHaveAttribute("aria-busy", "true");
    expect(requests).toHaveLength(1);
    const form = new URLSearchParams(requests[0]);
    expect(form.getAll("path")).toContain("");
    await expect(tree.locator(":scope > [data-expansion-state]")).toHaveAttribute(
      "data-expansion-state", /complete|partial/,
    );
    await expect(card.locator("[data-capability-json-status]")).toContainText(/expanded|limit|partially/i);

    await card.locator("[data-capability-json-collapse-all]").click();
    expect(await tree.innerHTML()).toBe(initial);
    await expect(card).toBeVisible();
    await expect(tree.locator("details[open]")).toHaveCount(0);

    const outline = tree.locator(":scope > [data-capability-json-page]");
    const format = await openJsonNode(outline, "format");
    const rest = await openJsonNode(outline, "rest");
    await expect(format).toHaveAttribute("open", "");
    await expect(rest).toHaveAttribute("open", "");
  });

  test("Collapse all cancels a late HTS aggregate response", async ({ page }) => {
    let release!: () => void;
    const gate = new Promise<void>((resolve) => { release = resolve; });
    await page.route("**/ui/hts/capability-statement/json-expand", async (route) => {
      await gate;
      try {
        await route.fulfill({
          status: 200,
          contentType: "text/html",
          body: '<div data-expansion-state="complete"><p id="late-hts-json">late</p></div>',
        });
      } catch {}
    });
    await page.goto(PATH);
    const card = page.locator("#capability-json-fold");
    const region = card.locator("#capability-json-body");
    const tree = card.locator("[data-capability-json-tree]");
    const initial = await tree.innerHTML();
    await card.locator("[data-capability-json-expand-all]").click();
    await expect(region).toHaveAttribute("aria-busy", "true");
    await card.locator("[data-capability-json-collapse-all]").click();
    release();
    await page.waitForTimeout(100);
    await expect(page.locator("#late-hts-json")).toHaveCount(0);
    await expect(region).not.toHaveAttribute("aria-busy", "true");
    expect(await tree.innerHTML()).toBe(initial);
  });

  test("the expanded HTS tree scrolls by wheel and PageDown", async ({ page }) => {
    test.setTimeout(120_000);
    await page.setViewportSize({ width: 1280, height: 500 });
    await page.goto(PATH);
    const card = page.locator("#capability-json-fold");
    const region = card.locator("#capability-json-body");
    await card.locator("[data-capability-json-expand-all]").click();
    await expect(region).not.toHaveAttribute("aria-busy", "true");
    expect(await region.evaluate((node) => node.scrollHeight)).toBeGreaterThan(
      await region.evaluate((node) => node.clientHeight),
    );
    await region.evaluate((node) => { node.scrollTop = 0; });
    await region.hover();
    await page.mouse.wheel(0, 500);
    await expect.poll(() => region.evaluate((node) => node.scrollTop)).toBeGreaterThan(0);
    const afterWheel = await region.evaluate((node) => node.scrollTop);
    await region.focus();
    await page.keyboard.press("PageDown");
    await expect.poll(() => region.evaluate((node) => node.scrollTop)).toBeGreaterThan(afterWheel);
  });

  test("aggregate failure retains the HTS tree and retry succeeds", async ({ page }) => {
    let fail = true;
    await page.route("**/ui/hts/capability-statement/json-expand", async (route) => {
      if (fail) {
        fail = false;
        await route.fulfill({ status: 503, body: "temporarily unavailable" });
      } else {
        await route.continue();
      }
    });
    await page.goto(PATH);
    const card = page.locator("#capability-json-fold");
    const tree = card.locator("[data-capability-json-tree]");
    const initial = await tree.innerHTML();
    await card.locator("[data-capability-json-expand-all]").click();
    await expect(card.locator("[data-capability-json-status]")).toContainText(/could not|try again|error/i);
    expect(await tree.innerHTML()).toBe(initial);
    await card.locator("[data-capability-json-expand-all]").click();
    await expect(tree.locator(":scope > [data-expansion-state]")).toBeVisible();
  });

  for (const theme of ["light", "dark"] as const) {
    test(`raw toolbar wraps without horizontal overflow at 390px — ${theme}`, async ({ page }) => {
      await page.addInitScript((value) => localStorage.setItem("hfs-theme", value), theme);
      await page.setViewportSize({ width: 390, height: 844 });
      await page.goto(PATH);
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      const metrics = await page.locator("#capability-json-fold").evaluate((card) => {
        const header = card.querySelector<HTMLElement>(":scope > .card-head")!;
        const tools = header.querySelector<HTMLElement>("[data-capability-json-actions]")!;
        const box = card.getBoundingClientRect();
        const toolsBox = tools.getBoundingClientRect();
        return {
          wraps: getComputedStyle(header).flexWrap,
          toolsInside: toolsBox.left >= box.left && toolsBox.right <= box.right + 1,
          overflow: document.documentElement.scrollWidth > document.documentElement.clientWidth,
        };
      });
      expect(metrics).toEqual({ wraps: "wrap", toolsInside: true, overflow: false });
    });
  }
});

test.describe("HTS Capability & Conformance per-source isolation", () => {
  test.skip("a 5xx on one source degrades only its own card", () => {
    // Skipped: the Playwright suite boots a real hts binary (see
    // `crates/hts-ui/e2e/boot.mjs`) and there is no way from the browser to
    // force `/metadata` to fail — HTS is its own upstream for that endpoint
    // and is guaranteed to be up while the suite is running. The isolation
    // contract is covered by the Rust integration test
    // `interactions_appear_when_declared_and_one_failure_degrades_only_its_card`
    // in `crates/hts-ui/tests/capability.rs`, which seeds a 500 against an
    // in-process mock and asserts exactly one `notice notice--warn` renders
    // while the other cards keep their live data.
  });
});
