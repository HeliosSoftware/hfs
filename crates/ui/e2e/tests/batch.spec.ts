import { test, expect } from "../pages/fixtures";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import AxeBuilder from "@axe-core/playwright";

// The Batch/Transaction workspace (#476): upload → preflight → execute →
// response, entirely against the ordinary FHIR API.

function bundleFile(type: "batch" | "transaction"): string {
  const stamp = Date.now();
  const bundle = {
    resourceType: "Bundle",
    type,
    entry: [
      {
        fullUrl: "urn:uuid:00000000-0000-4000-8000-000000000001",
        resource: { resourceType: "Patient", name: [{ family: `BatchUi${stamp}` }] },
        request: { method: "POST", url: "Patient" },
      },
      {
        fullUrl: "urn:uuid:00000000-0000-4000-8000-000000000002",
        resource: { resourceType: "Patient", name: [{ family: `BatchUiB${stamp}` }] },
        request: { method: "POST", url: "Patient" },
      },
    ],
  };
  const file = join(tmpdir(), `hfs-e2e-bundle-${type}-${stamp}.json`);
  writeFileSync(file, JSON.stringify(bundle));
  return file;
}

function writeBundle(bundle: unknown, label: string): string {
  const file = join(tmpdir(), `hfs-e2e-${label}-${Date.now()}-${Math.random()}.json`);
  writeFileSync(file, JSON.stringify(bundle));
  return file;
}

function writeRawFile(contents: string, label: string): string {
  const file = join(tmpdir(), `hfs-e2e-${label}-${Date.now()}-${Math.random()}.json`);
  writeFileSync(file, contents);
  return file;
}

test("a transaction bundle uploads, previews, executes, and reports", async ({ page }) => {
  // S3 has no multi-object atomicity and refuses transaction Bundles by
  // design (#489); the UI surfaces the rejection in #batch-execute-error.
  // The matrix sets this flag for that leg. Batch Bundles stay covered below.
  test.skip(process.env.HFS_E2E_NO_TRANSACTIONS === "1", "transactions refused on this backend");
  await page.goto("/ui/batch", { waitUntil: "networkidle" });

  await page.locator("#batch-file").setInputFiles(bundleFile("transaction"));

  // Preflight: request line, all-or-nothing copy, one row per entry.
  await expect(page.locator("#batch-preflight")).toBeVisible();
  await expect(page.locator("#batch-request-line")).toContainText("transaction");
  await expect(page.locator("#batch-request-line")).toContainText("2");
  await expect(page.locator("#batch-semantics")).toContainText(/all or nothing/i);
  await expect(page.locator("#batch-rows .batch-row")).toHaveCount(2);
  await expect(page.locator("#batch-rows .batch-chip").first()).toHaveText("POST");

  // The accordion shows the entry body.
  await page.locator("#batch-rows .batch-row__head").first().click();
  await expect(page.locator("#batch-rows .batch-row__body").first()).toBeVisible();
  await expect(page.locator("#batch-rows .batch-row__body .json-view").first()).toBeVisible();
  await expect(page.locator("#batch-rows .batch-row__body").first()).toContainText("BatchUi");

  // The Bundle JSON tab shows the raw payload.
  await page.locator("#batch-tab-json").click();
  await expect(page.locator("#batch-json")).toBeVisible();
  await expect(page.locator("#batch-json .json-view")).toBeVisible();
  await page.locator("#batch-tab-actions").click();

  // Execute: outcomes per entry plus the aggregate summary.
  await page.locator("#batch-execute").click();
  await expect(page.locator("#batch-response")).toBeVisible();
  await expect(page.locator("#batch-outcomes .batch-row")).toHaveCount(2);
  await expect(page.locator("#batch-outcomes .batch-badge").first()).toContainText("201");
  await expect(page.locator("#batch-summary")).toContainText("2");
  await expect(page.locator("#batch-summary")).toContainText(/created/i);
  await expect(page.locator("#batch-overall")).toHaveClass(/--ok/);

  // Back to the preflight keeps the parsed bundle.
  await page.locator("#batch-back").click();
  await expect(page.locator("#batch-preflight")).toBeVisible();
  await expect(page.locator("#batch-rows .batch-row")).toHaveCount(2);
});

test("JSON previews are lazy, cached, compact, accessible, and isolated per view", async ({ page }) => {
  const previews: { contentType: string | undefined; body: string | null }[] = [];
  page.on("request", (request) => {
    if (request.url().endsWith("/ui/json-view/render")) {
      previews.push({ contentType: request.headers()["content-type"], body: request.postData() });
    }
  });

  await page.goto("/ui/batch", { waitUntil: "networkidle" });
  await page.locator("#batch-file").setInputFiles(bundleFile("batch"));
  expect(previews).toHaveLength(0);

  const heads = page.locator("#batch-rows .batch-row__head");
  await heads.nth(0).click();
  await expect(page.locator("#batch-rows .json-view")).toHaveCount(1);
  expect(previews).toHaveLength(1);
  expect(previews[0].contentType).toBe("application/json");
  expect(previews[0].body).not.toContain("\n");

  await heads.nth(0).click();
  await heads.nth(0).click();
  expect(previews).toHaveLength(1);

  await heads.nth(1).click();
  await expect(page.locator("#batch-rows .json-view")).toHaveCount(2);
  expect(previews).toHaveLength(2);
  await expect(page.locator("#batch-rows #json-view")).toHaveCount(0);
  await expect(page.locator("#batch-rows [data-jpath]")).toHaveCount(0);

  const views = page.locator("#batch-rows .json-view");
  const firstRoot = views.nth(0).locator('.json-line--foldable[data-parents=""] [data-fold]');
  const secondRoot = views.nth(1).locator('.json-line--foldable[data-parents=""] [data-fold]');
  await expect(firstRoot).toHaveAttribute("data-fold", "f1");
  await expect(secondRoot).toHaveAttribute("data-fold", "f1");
  await firstRoot.click();
  await expect(firstRoot).toHaveAttribute("aria-expanded", "false");
  expect(await views.nth(0).locator(".json-line[hidden]").count()).toBeGreaterThan(0);
  await expect(views.nth(1).locator(".json-line[hidden]")).toHaveCount(0);

  await secondRoot.focus();
  await page.keyboard.press("Enter");
  await expect(secondRoot).toHaveAttribute("aria-expanded", "false");

  await page.locator("#batch-tab-json").click();
  await expect(page.locator("#batch-json .json-view")).toBeVisible();
  expect(previews).toHaveLength(3);
  await page.locator("#batch-tab-actions").click();
  await page.locator("#batch-tab-json").click();
  expect(previews).toHaveLength(3);
});

test("no-body entries skip rendering and failures use a safe whitespace-preserving fallback", async ({ page }) => {
  let previewRequests = 0;
  await page.route("**/ui/json-view/render", async (route) => {
    previewRequests++;
    await route.fulfill({ status: 503, body: "unavailable" });
  });
  const file = writeBundle(
    {
      resourceType: "Bundle",
      type: "batch",
      entry: [
        { request: { method: "DELETE", url: "Patient/missing" } },
        {
          resource: { resourceType: "Patient", name: [{ family: "<script>unsafe()</script>" }] },
          request: { method: "POST", url: "Patient" },
        },
      ],
    },
    "no-body",
  );

  await page.goto("/ui/batch", { waitUntil: "networkidle" });
  await page.locator("#batch-file").setInputFiles(file);
  const heads = page.locator("#batch-rows .batch-row__head");
  await heads.nth(0).click();
  expect(previewRequests).toBe(0);
  await expect(page.locator("#batch-rows .json-view__fallback").nth(0)).toContainText("no body");

  await heads.nth(1).click();
  await expect(page.locator("#batch-rows .batch-row__body").nth(1).locator("pre.json-view__fallback")).toBeVisible();
  expect(previewRequests).toBe(1);
  const fallback = await page.locator("#batch-rows .batch-row__body").nth(1).textContent();
  expect(fallback).toContain('\n  "resourceType"');
  await expect(page.locator("#batch-rows .batch-row__body").nth(1).locator("script")).toHaveCount(0);

  // A fallback is a terminal cached result for this file, just like a
  // highlighted response: reopening never hammers a failing endpoint.
  await heads.nth(1).click();
  await heads.nth(1).click();
  expect(previewRequests).toBe(1);

  await page.locator("#batch-tab-json").click();
  await expect(page.locator("#batch-json pre.json-view__fallback")).toBeVisible();
  expect(previewRequests).toBe(2);
  await page.locator("#batch-tab-actions").click();
  await page.locator("#batch-tab-json").click();
  expect(previewRequests).toBe(2);
});

test("an invalid replacement clears the old preview and cannot execute stale or null data", async ({ page }) => {
  let executeRequests = 0;
  page.on("request", (request) => {
    if (request.method() === "POST" && new URL(request.url()).pathname === "/") executeRequests++;
  });

  await page.goto("/ui/batch", { waitUntil: "networkidle" });
  await page.locator("#batch-file").setInputFiles(bundleFile("batch"));
  await page.locator("#batch-rows .batch-row__head").first().click();
  await expect(page.locator("#batch-rows .json-view")).toHaveCount(1);

  await page.locator("#batch-file").setInputFiles(writeRawFile("{ invalid", "invalid-replacement"));
  await expect(page.locator("#batch-upload")).toBeVisible();
  await expect(page.locator("#batch-upload-error")).toBeVisible();
  await expect(page.locator("#batch-preflight")).toBeHidden();
  await expect(page.locator("#batch-rows")).toBeEmpty();
  await expect(page.locator("#batch-json")).toBeEmpty();

  // Exercise the defensive guard even though the hidden control cannot be
  // reached by a user while the upload stage is visible.
  await page.locator("#batch-execute").evaluate((button: HTMLButtonElement) => button.click());
  expect(executeRequests).toBe(0);
  await expect(page.locator("#batch-upload-error")).toBeVisible();
});

test("a large highlighted preview folds every descendant and restores it exactly", async ({ page }) => {
  const identifiers = Array.from({ length: 600 }, (_, i) => ({
    system: `https://example.test/system/${i}`,
    value: String(i),
  }));
  const file = writeBundle(
    {
      resourceType: "Bundle",
      type: "batch",
      entry: [{
        resource: { resourceType: "Patient", identifier: identifiers },
        request: { method: "POST", url: "Patient" },
      }],
    },
    "large-preview",
  );

  await page.goto("/ui/batch", { waitUntil: "networkidle" });
  await page.locator("#batch-file").setInputFiles(file);
  await page.locator("#batch-rows .batch-row__head").click();
  const view = page.locator("#batch-rows .json-view");
  await expect(view).toBeVisible();
  const lines = view.locator(".json-line");
  expect(await lines.count()).toBeGreaterThan(1_000);

  const rootFold = view.locator('.json-line--foldable[data-parents=""] [data-fold]');
  await rootFold.click();
  await expect(rootFold).toHaveAttribute("aria-expanded", "false");
  expect(await view.locator(".json-line[hidden]").count()).toBe((await lines.count()) - 1);

  await rootFold.click();
  await expect(rootFold).toHaveAttribute("aria-expanded", "true");
  await expect(view.locator(".json-line[hidden]")).toHaveCount(0);
});

test("a render from an old file cannot overwrite the new file", async ({ page }) => {
  let releaseOld: (() => void) | undefined;
  let calls = 0;
  await page.route("**/ui/json-view/render", async (route) => {
    const call = ++calls;
    if (call === 1) await new Promise<void>((resolve) => { releaseOld = resolve; });
    const marker = call === 1 ? "STALE_RENDER" : "CURRENT_RENDER";
    await route.fulfill({
      status: 200,
      contentType: "text/html",
      body: `<div class="json-view"><div class="json-line" data-fold-id="" data-parents=""><span class="json-line__code">${marker}</span></div></div>`,
    }).catch(() => {});
  });

  await page.goto("/ui/batch", { waitUntil: "networkidle" });
  await page.locator("#batch-file").setInputFiles(bundleFile("batch"));
  const firstHead = page.locator("#batch-rows .batch-row__head").first();
  await firstHead.click();
  await expect.poll(() => calls).toBe(1);
  await firstHead.click();
  await firstHead.click();
  expect(calls).toBe(1);

  await page.locator("#batch-file").setInputFiles(bundleFile("batch"));
  await page.locator("#batch-rows .batch-row__head").first().click();
  await expect(page.locator("#batch-rows")).toContainText("CURRENT_RENDER");
  releaseOld?.();
  await expect(page.locator("#batch-rows")).not.toContainText("STALE_RENDER");
});

for (const theme of ["light", "dark"] as const) {
  test(`dynamic highlighted Batch JSON has no WCAG 2.2 AA violations — ${theme}`, async ({ page }) => {
    await page.addInitScript((selected) => localStorage.setItem("hfs-theme", selected), theme);
    await page.goto("/ui/batch", { waitUntil: "networkidle" });
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await page.locator("#batch-file").setInputFiles(bundleFile("batch"));
    await page.locator("#batch-rows .batch-row__head").first().click();
    await expect(page.locator("#batch-rows .json-view")).toBeVisible();
    const { violations } = await new AxeBuilder({ page })
      .include("#batch-preflight")
      .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
      .analyze();
    expect(violations).toEqual([]);
  });
}

test("a batch bundle gets the independent-entries copy", async ({ page }) => {
  await page.goto("/ui/batch", { waitUntil: "networkidle" });
  await page.locator("#batch-file").setInputFiles(bundleFile("batch"));
  await expect(page.locator("#batch-semantics")).toContainText(/independently/i);
});

test("a non-bundle file is rejected with a message, not a crash", async ({ page }) => {
  await page.goto("/ui/batch", { waitUntil: "networkidle" });
  const file = join(tmpdir(), `hfs-e2e-notabundle-${Date.now()}.json`);
  writeFileSync(file, JSON.stringify({ resourceType: "Patient" }));
  await page.locator("#batch-file").setInputFiles(file);
  await expect(page.locator("#batch-upload-error")).toBeVisible();
  await expect(page.locator("#batch-preflight")).toBeHidden();
});
