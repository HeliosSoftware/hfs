import { test, expect } from "../pages/fixtures";

type CapabilityJsonBody = HTMLElement & {
  capabilityJsonXhr?: XMLHttpRequest | null;
};

test("Capability Statement links follow the active FHIR version", async ({
  capabilityStatement,
  chrome,
}) => {
  await capabilityStatement.goto("?filter=Patient");

  const version = await chrome.currentVersion();
  const configuredVersion = process.env.HFS_DEFAULT_FHIR_VERSION;
  if (configuredVersion) {
    expect(version).toBe(configuredVersion);
  }
  const expected = {
    R4: {
      summary: "4.0.1",
      patientHref: "https://hl7.org/fhir/R4/patient.html",
    },
    R4B: {
      summary: "4.3.0",
      patientHref: "https://hl7.org/fhir/R4B/patient.html",
    },
    R5: {
      summary: "5.0.0",
      patientHref: "https://hl7.org/fhir/R5/patient.html",
    },
    R6: {
      summary: "6.0.0",
      patientHref: "https://hl7.org/fhir/6.0.0-ballot4/patient.html",
    },
  }[version];
  expect(expected, `No Capability Statement expectation is defined for ${version}`).toBeTruthy();

  await expect(capabilityStatement.resourceFilterVersion).toHaveValue(version);
  await expect(capabilityStatement.fhirVersionSummary).toHaveText(expected!.summary);
  await expect(capabilityStatement.resourceLink("Patient")).toHaveAttribute(
    "href",
    expected!.patientHref,
  );
});

test("Capability Statement JSON expands in bounded reloadable fragments", async ({
  page,
  capabilityStatement,
  chrome,
}) => {
  test.setTimeout(120_000);
  const fragmentRequests = new Map<string, number>();
  let releaseRoot!: () => void;
  const rootGate = new Promise<void>((resolve) => {
    releaseRoot = resolve;
  });
  let releaseRest!: () => void;
  const restGate = new Promise<void>((resolve) => {
    releaseRest = resolve;
  });
  let releaseRestItem!: () => void;
  const restItemGate = new Promise<void>((resolve) => {
    releaseRestItem = resolve;
  });
  const fragmentPath = (url: string): string | null => {
    const parsed = new URL(url);
    if (parsed.pathname !== "/ui/capability-statement/json-fragment") return null;
    return parsed.searchParams.get("path");
  };
  page.on("request", (request) => {
    const path = fragmentPath(request.url());
    if (path !== null) fragmentRequests.set(path, (fragmentRequests.get(path) ?? 0) + 1);
  });
  await page.route("**/ui/capability-statement/json-fragment?*", async (route) => {
    const path = fragmentPath(route.request().url());
    if (path === "") await rootGate;
    if (path === "/rest" && fragmentRequests.get(path) === 1) await restGate;
    if (path === "/rest/0" && fragmentRequests.get(path) === 1) await restItemGate;
    await route.continue();
  });

  await capabilityStatement.goto("?filter=Patient");
  const version = await chrome.currentVersion();
  await expect(capabilityStatement.jsonView).toHaveCount(0);

  const rootResponse = page.waitForResponse((response) => fragmentPath(response.url()) === "");
  await capabilityStatement.rawSummary.click();
  await expect(capabilityStatement.rawLoading).toBeVisible();
  releaseRoot();
  const response = await rootResponse;
  expect(response.ok()).toBeTruthy();
  const requested = new URL(response.url());
  expect(requested.searchParams.get("version")).toBe(version);
  expect(requested.searchParams.get("path")).toBe("");
  expect(requested.searchParams.get("limit")).toBe("100");
  expect(fragmentRequests.get("")).toBe(1);

  await expect(capabilityStatement.jsonOutline).toBeVisible();
  await expect(capabilityStatement.jsonOutline).toHaveAttribute("data-item-count", /\d+/);
  await expect(capabilityStatement.collapseAllJson).toBeVisible();
  await expect(capabilityStatement.expandAllJson).toBeVisible();
  await expect(capabilityStatement.rawDisclosure).toHaveAttribute("open", "");
  await expect(capabilityStatement.rawBody).toHaveAttribute("tabindex", "0");

  const format = capabilityStatement.jsonNode(capabilityStatement.jsonOutline, "format");
  const formatBody = capabilityStatement.nodeBody(format);
  await capabilityStatement.expandAllJson.click();
  await expect(format).toHaveAttribute("open", "");
  await expect(formatBody.locator("[data-capability-json-page], .json-view").first()).toBeVisible();
  await capabilityStatement.collapseAllJson.click();
  await expect(format).not.toHaveAttribute("open", "");
  await expect(capabilityStatement.rawDisclosure).toHaveAttribute("open", "");
  await format.locator(":scope > summary").click();
  await expect(formatBody.locator("[data-capability-json-page], .json-view").first()).toBeVisible();
  await expect(capabilityStatement.rawBody).toHaveCSS("overflow-y", "auto");
  await expect(formatBody.locator(".json-view")).toHaveCSS("max-height", "none");
  await expect(formatBody.locator(".json-view")).toHaveCSS("overflow-y", "visible");

  const rest = capabilityStatement.jsonNode(capabilityStatement.jsonOutline, "rest");
  const restBody = capabilityStatement.nodeBody(rest);
  // An in-flight request is aborted and its busy state cleared when the node
  // closes. Opening this sibling leaves the format branch available.
  await rest.locator(":scope > summary").click();
  await expect(format).toHaveAttribute("open", "");
  await expect(restBody).toHaveAttribute("aria-busy", "true");
  await rest.locator(":scope > summary").click();
  await expect(restBody).not.toHaveAttribute("aria-busy", "true");
  releaseRest();
  await rest.locator(":scope > summary").click();
  await expect(restBody.locator("[data-capability-json-page], .json-view").first()).toBeVisible();
  expect(fragmentRequests.get("/rest")).toBe(2);

  // A collapsing ancestor aborts every descendant request before removing its
  // DOM. Releasing the intercepted response afterward cannot perform a stale
  // swap into the collapsed branch.
  const restItem = restBody
    .locator(":scope > .capability-json-outline > .capability-json-rows > details")
    .first();
  const restItemBody = capabilityStatement.nodeBody(restItem);
  await restItem.locator(":scope > summary").click();
  await expect(restItemBody).toHaveAttribute("aria-busy", "true");
  const blockedBody = await restItemBody.elementHandle();
  expect(blockedBody).not.toBeNull();
  await expect
    .poll(() =>
      blockedBody!.evaluate((body) =>
        Boolean((body as CapabilityJsonBody).capabilityJsonXhr),
      ),
    )
    .toBe(true);
  await rest.locator(":scope > summary").click();
  expect(
    await blockedBody!.evaluate(
      (body) => (body as CapabilityJsonBody).capabilityJsonXhr === null,
    ),
  ).toBe(true);
  expect(await blockedBody!.evaluate((body) => body.isConnected)).toBe(false);
  releaseRestItem();
  await expect(restBody.locator("[data-capability-json-page], .json-view")).toHaveCount(0);
  await rest.locator(":scope > summary").click();
  await expect(restBody.locator("[data-capability-json-page], .json-view").first()).toBeVisible();
  expect(fragmentRequests.get("/rest")).toBe(3);

  const reloadedRestItem = restBody
    .locator(":scope > .capability-json-outline > .capability-json-rows > details")
    .first();
  const restItemBodyReloaded = capabilityStatement.nodeBody(reloadedRestItem);
  await reloadedRestItem.locator(":scope > summary").click();
  await expect(restItemBodyReloaded.locator(":scope > .capability-json-outline")).toBeVisible();

  // The resource array is larger than one page on every supported version.
  // Paging replaces the current 100-item page instead of accumulating rows.
  const resource = capabilityStatement.jsonNode(
    restItemBodyReloaded.locator(":scope > .capability-json-outline"),
    "resource",
  );
  const resourceBody = capabilityStatement.nodeBody(resource);
  await resource.locator(":scope > summary").click();
  await expect(resourceBody.locator("[data-capability-json-page]")).toHaveAttribute(
    "data-item-count",
    "100",
  );
  await expect
    .poll(() => capabilityStatement.rawBody.evaluate((body) => body.scrollHeight > body.clientHeight))
    .toBe(true);
  await capabilityStatement.rawBody.focus();
  await capabilityStatement.rawBody.press("PageDown");
  await expect.poll(() => capabilityStatement.rawBody.evaluate((body) => body.scrollTop)).toBeGreaterThan(0);
  const next = capabilityStatement.pageControl(resourceBody, "next");
  await expect(next).toBeEnabled();
  await next.click();
  await expect(resourceBody.locator(".capability-json-pagination")).toContainText(/101–/);
  const focusedPageControl = resourceBody.locator(
    "[data-capability-json-page-direction]:focus",
  );
  await expect(focusedPageControl).toHaveCount(1);
  const expectedFocusDirection = (await next.isEnabled()) ? "next" : "previous";
  await expect(focusedPageControl).toHaveAttribute(
    "data-capability-json-page-direction",
    expectedFocusDirection,
  );
  const secondPageCount = Number(
    await resourceBody.locator("[data-capability-json-page]").getAttribute("data-item-count"),
  );
  expect(secondPageCount).toBeGreaterThan(0);
  expect(secondPageCount).toBeLessThanOrEqual(100);
  expect(fragmentRequests.get("/rest/0/resource")).toBe(2);

  await capabilityStatement.collapseAllJson.click();
  await expect(format).not.toHaveAttribute("open", "");
  await expect(rest).not.toHaveAttribute("open", "");
  await expect(capabilityStatement.rawDisclosure).toHaveAttribute("open", "");

  // Closing the top-level disclosure unloads the entire incremental tree.
  await capabilityStatement.rawSummary.click();
  await expect(capabilityStatement.rawDisclosure).not.toHaveAttribute("open", "");
  await expect(capabilityStatement.rawBody.locator("[data-capability-json-page], .json-view")).toHaveCount(0);
  await capabilityStatement.rawSummary.click();
  await expect(capabilityStatement.rawDisclosure).toHaveAttribute("open", "");
  await expect(capabilityStatement.jsonOutline).toBeVisible();
  expect(fragmentRequests.get("")).toBe(2);
});
test("typing filters resource capabilities live", async ({ capabilityStatement }) => {
  await capabilityStatement.goto();
  await expect(capabilityStatement.resourceRow("Patient")).toBeVisible();
  await expect(capabilityStatement.resourceRow("Observation")).toBeVisible();

  await capabilityStatement.filter.fill("Patient");

  await expect(capabilityStatement.resourceRow("Patient")).toBeVisible();
  await expect(capabilityStatement.resourceRow("Observation")).toHaveCount(0);
});

test("the resource filter stacks inside the card at phone width", async ({
  page,
  capabilityStatement,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await capabilityStatement.goto();

  const header = page.locator(".cap-resource-card > .card-head");
  const filter = page.locator(".cap-resource-filter");
  await expect(header).toHaveCSS("flex-direction", "column");
  await expect(filter).toBeVisible();

  const headerBox = await header.boundingBox();
  const filterBox = await filter.boundingBox();
  expect(headerBox).not.toBeNull();
  expect(filterBox).not.toBeNull();
  expect(filterBox!.x).toBeGreaterThanOrEqual(headerBox!.x);
  expect(filterBox!.x + filterBox!.width).toBeLessThanOrEqual(
    headerBox!.x + headerBox!.width + 1,
  );
});
