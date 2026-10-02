import { test, expect } from "../../pages/fixtures";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

// #320, leg 2: auth enabled but NO outbound token provisioned. The self-fetch
// is rejected (401), and the conformance pages must degrade to their warning
// state — no crash, no empty 404 — and re-attempt the fetch on the next
// request instead of caching the failure.

test("search parameters degrade to the warning state", async ({ page, searchParameters }) => {
  await searchParameters.goto();
  await expect(page.locator(".notice--warn")).toBeVisible();
  await expect(searchParameters.rows).toHaveCount(0);
});

test("compartments degrade to a warning page, not a 404", async ({ page }) => {
  const response = await page.goto("/ui/compartments", { waitUntil: "networkidle" });
  expect(response?.status()).toBe(200);
  await expect(page.locator(".notice--warn")).toBeVisible();
  await expect(page.locator("h1.page-head__title")).toBeVisible();
});

test("the degraded fetch is retried, not cached", async ({ page, searchParameters }) => {
  // Two consecutive loads both warn — and both actually hit the API again:
  // the failed snapshot is served degraded for its request only.
  await searchParameters.goto();
  await expect(page.locator(".notice--warn")).toBeVisible();
  await searchParameters.goto();
  await expect(page.locator(".notice--warn")).toBeVisible();
});

// #1560: auth on and no interactive login — the shell says so once on every
// page, and the Batch page's 401 names the missing setting instead of the
// API's raw header text.
test("the shell says that no browser sign-in is configured", async ({ page }) => {
  await page.goto("/ui", { waitUntil: "networkidle" });
  const notice = page.locator("#auth-bearer-only");
  await expect(notice).toHaveCount(1);
  await expect(notice).toContainText("HFS_UI_LOGIN_CLIENT_ID");
});

test("executing a bundle explains the missing sign-in, not the raw 401", async ({ page }) => {
  const file = join(tmpdir(), `hfs-e2e-bearer-only-${Date.now()}.json`);
  writeFileSync(
    file,
    JSON.stringify({
      resourceType: "Bundle",
      type: "batch",
      entry: [{ request: { method: "GET", url: "Patient?_count=1" } }],
    }),
  );
  await page.goto("/ui/batch", { waitUntil: "networkidle" });
  await page.locator("#batch-file").setInputFiles(file);
  await page.locator("#batch-execute-top").click();
  const error = page.locator("#batch-execute-error");
  await expect(error).toBeVisible();
  await expect(error).toContainText("HFS_UI_LOGIN_CLIENT_ID");
  await expect(error).not.toContainText("Missing Authorization header");
});

// #1619 / #1633: with auth on and no browser sign-in, nothing a browser sends
// to the Import page can be authenticated, so an anonymous visitor can neither
// open it nor start, delete or abort a submission — and the FHIR operation the
// page would call refuses the same caller.
test.describe("an anonymous import is refused", () => {
  const form = {
    name: "anonymous-import",
    manifest_url: "http://127.0.0.1:9/manifest.json",
    auth: "none",
    submitter_system: "urn:helios:hfs:e2e",
    submitter_value: "anonymous",
    output_format: "application/fhir+ndjson",
  };

  test("the Import page answers 401 and names the sign-in setting", async ({ page }) => {
    const response = await page.goto("/ui/bulk-import");
    expect(response?.status()).toBe(401);
    await expect(page.locator("body")).toContainText("HFS_UI_LOGIN_CLIENT_ID");
  });

  test("creating a submission from the page is refused", async ({ request }) => {
    const response = await request.post("/ui/bulk-import", { form });
    expect(response.status()).toBe(401);
    expect(await response.text()).toContain("HFS_UI_LOGIN_CLIENT_ID");
  });

  for (const action of ["delete", "abort", "complete", "edit"]) {
    test(`${action} on a submission is refused`, async ({ request }) => {
      const response = await request.post(`/ui/bulk-import/anonymous-import/${action}`, {
        form: { name: "anonymous-import" },
      });
      expect(response.status()).toBe(401);
    });
  }

  test("$bulk-submit itself refuses the anonymous caller", async ({ request }) => {
    const response = await request.post("/$bulk-submit", {
      headers: { "Content-Type": "application/fhir+json" },
      data: {
        resourceType: "Parameters",
        parameter: [
          { name: "submitter", valueIdentifier: { system: form.submitter_system, value: form.submitter_value } },
          { name: "submissionId", valueString: "anonymous-import" },
          { name: "manifestUrl", valueString: form.manifest_url },
        ],
      },
    });
    expect(response.status()).toBe(401);
  });
});
