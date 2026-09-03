import { test, expect } from "../pages/fixtures";
import type { Locator } from "@playwright/test";

async function openNode(node: Locator): Promise<Locator> {
  const body = node.locator(":scope > [data-capability-json-body]");
  await node.locator(":scope > summary").click();
  await expect(body.locator("[data-capability-json-page], .json-view").first()).toBeVisible();
  return body;
}

test("Capability Statement links follow the active FHIR version", async ({
  capabilityStatement,
  chrome,
}) => {
  await capabilityStatement.goto("?filter=Patient");
  const version = await chrome.currentVersion();
  const configuredVersion = process.env.HFS_DEFAULT_FHIR_VERSION;
  if (configuredVersion) expect(version).toBe(configuredVersion);
  const expected = {
    R4: { summary: "4.0.1", patientHref: "https://hl7.org/fhir/R4/patient.html" },
    R4B: { summary: "4.3.0", patientHref: "https://hl7.org/fhir/R4B/patient.html" },
    R5: { summary: "5.0.0", patientHref: "https://hl7.org/fhir/R5/patient.html" },
    R6: { summary: "6.0.0", patientHref: "https://hl7.org/fhir/6.0.0-ballot4/patient.html" },
  }[version];
  expect(expected, `No Capability Statement expectation is defined for ${version}`).toBeTruthy();
  await expect(capabilityStatement.resourceFilterVersion).toHaveValue(version);
  await expect(capabilityStatement.fhirVersionSummary).toHaveText(expected!.summary);
  await expect(capabilityStatement.resourceLink("Patient")).toHaveAttribute("href", expected!.patientHref);
});

test("raw JSON starts at the first level with an accessible enhanced toolbar", async ({
  capabilityStatement,
}) => {
  await capabilityStatement.goto();
  await expect(capabilityStatement.rawCard).toBeVisible();
  await expect(capabilityStatement.rawCard).not.toHaveAttribute("open", "");
  await expect(capabilityStatement.rawHeader.locator(":scope > h3")).toHaveText(/^Raw CapabilityStatement/);
  await expect(capabilityStatement.rawHeader.locator(":scope > [data-capability-json-actions]")).toBeVisible();
  await expect(capabilityStatement.rawActions).not.toHaveAttribute("hidden", "");
  await expect(capabilityStatement.rawLoadLink).toBeHidden();
  await expect(capabilityStatement.jsonOutline).toBeVisible();
  await expect(capabilityStatement.jsonOutline).toHaveAttribute("data-path", "");
  await expect(capabilityStatement.jsonOutline).toHaveAttribute("data-offset", "0");
  await expect(capabilityStatement.jsonOutline.locator("details[data-capability-json-node][open]")).toHaveCount(0);
  await expect(capabilityStatement.rawBody).toHaveAttribute("role", "region");
  await expect(capabilityStatement.rawBody).toHaveAttribute("aria-labelledby", "capability-json-heading");
  await expect(capabilityStatement.rawBody).toHaveAttribute("tabindex", "0");
  await expect(capabilityStatement.rawBody).toHaveCSS("overflow-y", "auto");
  expect(parseFloat(await capabilityStatement.rawBody.evaluate((node) => getComputedStyle(node).maxHeight))).toBeGreaterThan(0);
  await expect(capabilityStatement.collapseAll).toBeDisabled();
  await expect(capabilityStatement.expandAll).toBeEnabled();
  await expect(capabilityStatement.page.locator("#capability-json")).toHaveCount(0);
});

test("Expand all is one bounded POST and Collapse all restores the first level", async ({
  page,
  capabilityStatement,
}) => {
  test.setTimeout(120_000);
  let release!: () => void;
  const gate = new Promise<void>((resolve) => { release = resolve; });
  const requests: string[] = [];
  page.on("request", (request) => {
    if (request.method() === "POST" && new URL(request.url()).pathname.endsWith("/json-expand")) {
      requests.push(request.postData() ?? "");
    }
  });
  await page.route("**/ui/capability-statement/json-expand?*", async (route) => {
    await gate;
    await route.continue();
  });
  await capabilityStatement.goto();
  const initial = await capabilityStatement.jsonTree.innerHTML();
  await capabilityStatement.expandAll.click();
  await expect(capabilityStatement.rawBody).toHaveAttribute("aria-busy", "true");
  await expect(capabilityStatement.rawStatus).toContainText(/Expanding/i);
  await expect(capabilityStatement.collapseAll).toBeEnabled();
  expect(requests).toHaveLength(1);
  const form = new URLSearchParams(requests[0]);
  expect(form.getAll("path")).toEqual([""]);
  expect(form.getAll("offset")).toEqual(["0"]);
  expect(form.getAll("limit")).toEqual(["100"]);
  release();
  await expect(capabilityStatement.rawBody).not.toHaveAttribute("aria-busy", "true");
  await expect(capabilityStatement.rawStatus).toContainText(/expanded|limit|partially/i);
  await expect(capabilityStatement.jsonTree.locator(":scope > [data-expansion-state]")).toHaveAttribute(
    "data-expansion-state", /complete|partial/,
  );
  expect(requests).toHaveLength(1);
  await capabilityStatement.collapseAll.click();
  await expect(capabilityStatement.rawCard).toBeVisible();
  await expect(capabilityStatement.rawStatus).toHaveText("");
  await expect(capabilityStatement.collapseAll).toBeDisabled();
  expect(await capabilityStatement.jsonTree.innerHTML()).toBe(initial);
  await expect(capabilityStatement.jsonOutline.locator("details[data-capability-json-node][open]")).toHaveCount(0);
  const format = capabilityStatement.jsonNode(capabilityStatement.jsonOutline, "format");
  const rest = capabilityStatement.jsonNode(capabilityStatement.jsonOutline, "rest");
  await openNode(format);
  await openNode(rest);
  await expect(format).toHaveAttribute("open", "");
  await expect(rest).toHaveAttribute("open", "");
});

test("Expand all preserves visible pagination offsets and never follows the next page", async ({
  page,
  capabilityStatement,
}) => {
  test.setTimeout(120_000);
  await capabilityStatement.goto();
  const restBody = await openNode(capabilityStatement.jsonNode(capabilityStatement.jsonOutline, "rest"));
  const firstRestBody = await openNode(restBody.locator("details[data-capability-json-node]").first());
  const resourceBody = await openNode(
    capabilityStatement.jsonNode(firstRestBody.locator(":scope > [data-capability-json-page]"), "resource"),
  );
  const next = capabilityStatement.pageControl(resourceBody, "next");
  await expect(next).toBeEnabled();
  await next.click();
  const visiblePage = resourceBody.locator(":scope > [data-capability-json-page]");
  await expect(visiblePage).toHaveAttribute("data-offset", "100");
  const fragmentRequests: string[] = [];
  page.on("request", (request) => {
    const url = new URL(request.url());
    if (url.pathname.endsWith("/json-fragment") && url.searchParams.get("path") === "/rest/0/resource") {
      fragmentRequests.push(url.searchParams.get("offset") ?? "");
    }
  });
  const expandRequest = page.waitForRequest(
    (request) => request.method() === "POST" && new URL(request.url()).pathname.endsWith("/json-expand"),
  );
  await capabilityStatement.expandAll.click();
  const body = new URLSearchParams((await expandRequest).postData() ?? "");
  const paths = body.getAll("path");
  const offsets = body.getAll("offset");
  expect(paths.map((path, index) => ({ path, offset: offsets[index] }))).toContainEqual({
    path: "/rest/0/resource", offset: "100",
  });
  await expect(capabilityStatement.rawBody).not.toHaveAttribute("aria-busy", "true");
  await expect(capabilityStatement.jsonTree.locator('[data-path="/rest/0/resource"]')).toHaveAttribute(
    "data-offset", "100",
  );
  expect(fragmentRequests).not.toContain("200");
});

test("Collapse all cancels a late aggregate response without swapping it", async ({
  page,
  capabilityStatement,
}) => {
  let release!: () => void;
  const gate = new Promise<void>((resolve) => { release = resolve; });
  await page.route("**/ui/capability-statement/json-expand?*", async (route) => {
    await gate;
    try {
      await route.fulfill({
        status: 200,
        contentType: "text/html",
        body: '<div data-expansion-state="complete"><p id="late-json-response">late</p></div>',
      });
    } catch {
      // Chromium may dispose an intercepted route immediately after AbortController fires.
    }
  });
  await capabilityStatement.goto();
  const initial = await capabilityStatement.jsonTree.innerHTML();
  await capabilityStatement.expandAll.click();
  await expect(capabilityStatement.rawBody).toHaveAttribute("aria-busy", "true");
  await capabilityStatement.collapseAll.click();
  await expect(capabilityStatement.rawBody).not.toHaveAttribute("aria-busy", "true");
  release();
  await page.waitForTimeout(100);
  await expect(page.locator("#late-json-response")).toHaveCount(0);
  expect(await capabilityStatement.jsonTree.innerHTML()).toBe(initial);
});

test("an aggregate HTTP failure retains the tree and permits retry", async ({
  page,
  capabilityStatement,
}) => {
  let fail = true;
  await page.route("**/ui/capability-statement/json-expand?*", async (route) => {
    if (fail) {
      fail = false;
      await route.fulfill({ status: 503, body: "temporarily unavailable" });
    } else {
      await route.continue();
    }
  });
  await capabilityStatement.goto();
  const initial = await capabilityStatement.jsonTree.innerHTML();
  await capabilityStatement.expandAll.click();
  await expect(capabilityStatement.rawStatus).toContainText(/could not|try again|error/i);
  expect(await capabilityStatement.jsonTree.innerHTML()).toBe(initial);
  await expect(capabilityStatement.expandAll).toBeEnabled();
  await capabilityStatement.expandAll.click();
  await expect(capabilityStatement.rawBody).not.toHaveAttribute("aria-busy", "true");
  await expect(capabilityStatement.jsonTree.locator(":scope > [data-expansion-state]")).toHaveAttribute(
    "data-expansion-state", /complete|partial/,
  );
});

test("the expanded tree owns keyboard and wheel scrolling", async ({ page, capabilityStatement }) => {
  test.setTimeout(120_000);
  await page.setViewportSize({ width: 1280, height: 500 });
  await capabilityStatement.goto();
  await capabilityStatement.expandAll.click();
  await expect(capabilityStatement.rawBody).not.toHaveAttribute("aria-busy", "true");
  const dimensions = await capabilityStatement.rawBody.evaluate((node) => ({
    clientHeight: node.clientHeight,
    scrollHeight: node.scrollHeight,
  }));
  expect(dimensions.scrollHeight).toBeGreaterThan(dimensions.clientHeight);
  await capabilityStatement.rawBody.evaluate((node) => { node.scrollTop = 0; });
  await capabilityStatement.rawBody.hover();
  await page.mouse.wheel(0, 500);
  await expect.poll(() => capabilityStatement.rawBody.evaluate((node) => node.scrollTop)).toBeGreaterThan(0);
  const afterWheel = await capabilityStatement.rawBody.evaluate((node) => node.scrollTop);
  await capabilityStatement.rawBody.focus();
  await page.keyboard.press("PageDown");
  await expect.poll(() => capabilityStatement.rawBody.evaluate((node) => node.scrollTop)).toBeGreaterThan(afterWheel);
});

test("wheel scrolling chains to the page at the expanded tree boundary", async ({
  page,
  capabilityStatement,
}) => {
  test.setTimeout(120_000);
  await page.setViewportSize({ width: 1280, height: 500 });
  await capabilityStatement.goto();
  await capabilityStatement.expandAll.click();
  await expect(capabilityStatement.rawBody).not.toHaveAttribute("aria-busy", "true");
  await capabilityStatement.rawBody.evaluate((node) => {
    node.scrollTop = 0;
  });
  await page.evaluate(() => window.scrollTo(0, document.documentElement.scrollHeight));
  await capabilityStatement.rawBody.hover();
  const pageScrollBeforeWheel = await page.evaluate(() => window.scrollY);
  expect(pageScrollBeforeWheel).toBeGreaterThan(0);

  await page.mouse.wheel(0, -240);

  await expect.poll(() => page.evaluate(() => window.scrollY)).toBeLessThan(pageScrollBeforeWheel);
  await expect(capabilityStatement.rawBody).toHaveJSProperty("scrollTop", 0);

  const regionScrollBottom = await capabilityStatement.rawBody.evaluate((node) => {
    node.scrollTop = node.scrollHeight;
    return node.scrollTop;
  });
  await capabilityStatement.rawBody.hover();
  const pageScrollBeforeDownwardWheel = await page.evaluate(() => window.scrollY);
  await page.mouse.wheel(0, 240);

  await expect
    .poll(() => page.evaluate(() => window.scrollY))
    .toBeGreaterThan(pageScrollBeforeDownwardWheel);
  await expect(capabilityStatement.rawBody).toHaveJSProperty("scrollTop", regionScrollBottom);
});

for (const theme of ["light", "dark"] as const) {
  test(`raw toolbar stays inside the card at phone width — ${theme}`, async ({
    page,
    capabilityStatement,
    chrome,
  }) => {
    await chrome.seedTheme(theme);
    await page.setViewportSize({ width: 390, height: 844 });
    await capabilityStatement.goto();
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    const metrics = await capabilityStatement.rawCard.evaluate((card) => {
      const actions = card.querySelector<HTMLElement>("[data-capability-json-actions]")!;
      const header = card.querySelector<HTMLElement>(":scope > .card-head")!;
      const cardBox = card.getBoundingClientRect();
      const actionsBox = actions.getBoundingClientRect();
      return {
        actionsInside: actionsBox.left >= cardBox.left && actionsBox.right <= cardBox.right + 1,
        wraps: getComputedStyle(header).flexWrap,
        overflows: document.documentElement.scrollWidth > document.documentElement.clientWidth,
      };
    });
    expect(metrics).toEqual({ actionsInside: true, wraps: "wrap", overflows: false });
  });
}

test("typing filters resource capabilities live", async ({ capabilityStatement }) => {
  await capabilityStatement.goto();
  await expect(capabilityStatement.resourceRow("Patient")).toBeVisible();
  await expect(capabilityStatement.resourceRow("Observation")).toBeVisible();
  await capabilityStatement.filter.fill("Patient");
  await expect(capabilityStatement.resourceRow("Patient")).toBeVisible();
  await expect(capabilityStatement.resourceRow("Observation")).toHaveCount(0);
});

test("the resource filter stacks inside the card at phone width", async ({ page, capabilityStatement }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await capabilityStatement.goto();
  const header = page.locator(".cap-resource-card > .card-head");
  const filter = page.locator(".cap-resource-filter");
  await expect(header).toHaveCSS("flex-direction", "column");
  const headerBox = await header.boundingBox();
  const filterBox = await filter.boundingBox();
  expect(headerBox).not.toBeNull();
  expect(filterBox).not.toBeNull();
  expect(filterBox!.x).toBeGreaterThanOrEqual(headerBox!.x);
  expect(filterBox!.x + filterBox!.width).toBeLessThanOrEqual(headerBox!.x + headerBox!.width + 1);
});
