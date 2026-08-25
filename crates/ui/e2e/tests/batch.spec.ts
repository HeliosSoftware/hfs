import { test, expect } from "../pages/fixtures";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

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
  await expect(page.locator("#batch-rows .batch-row__body").first()).toContainText("BatchUi");

  // The Bundle JSON tab shows the raw payload.
  await page.locator("#batch-tab-json").click();
  await expect(page.locator("#batch-json")).toBeVisible();
  await page.locator("#batch-tab-actions").click();

  // Execute: outcomes per entry plus the aggregate summary.
  await page.locator("#batch-execute").click();
  await expect(page.locator("#batch-response")).toBeVisible();
  await expect(page.locator("#batch-outcomes .batch-row")).toHaveCount(2);
  await expect(page.locator("#batch-outcomes .batch-badge").first()).toContainText("201");
  await expect(page.locator("#batch-summary")).toContainText("2");
  await expect(page.locator("#batch-summary")).toContainText(/created/i);
  await expect(page.locator("#batch-overall")).toHaveClass(/--ok/);

  // Done is the one way out (#675): back to a clean upload stage.
  await page.locator("#batch-done").click();
  await expect(page.locator("#batch-upload")).toBeVisible();
  await expect(page.locator("#batch-preflight")).toBeHidden();
});

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
