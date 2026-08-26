import { test, expect } from "../pages/fixtures";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import AxeBuilder from "@axe-core/playwright";
import { axeSummary } from "../pages/axe";

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

  // The execute error slot lives inside the footer, next to its button (#676).
  await expect(page.locator(".batch-footer #batch-execute-error")).toHaveCount(1);

  // Execute: outcomes per entry plus the aggregate summary.
  await page.locator("#batch-execute").click();
  await expect(page.locator("#batch-response")).toBeVisible();
  await expect(page.locator("#batch-outcomes .batch-row")).toHaveCount(2);
  await expect(page.locator("#batch-outcomes .batch-badge").first()).toContainText("201");
  await expect(page.locator("#batch-summary")).toContainText("2");
  await expect(page.locator("#batch-summary")).toContainText(/created/i);
  await expect(page.locator("#batch-overall")).toHaveClass(/--ok/);

  // Done is the one way out (#675): back to a clean upload stage. Done hid
  // the stage that held focus, so focus lands on the drop zone (#679).
  await page.locator("#batch-done").click();
  await expect(page.locator("#batch-upload")).toBeVisible();
  await expect(page.locator("#batch-preflight")).toBeHidden();
  await expect(page.locator("#batch-drop")).toBeFocused();
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
  await expect(page.locator("#batch-busy")).toBeHidden();
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
  // The parse-side busy region clears on the failure path too (#679).
  await expect(page.locator("#batch-busy")).toBeHidden();
});

// ---- #679: the shared busy states -----------------------------------------
// The transient states are made deterministically observable: FileReader
// delivery and the execute POST are both parked behind manual releases.

const FOOTER_CONTROLS = ["#batch-execute", "#batch-execute-top", "#batch-cancel", "#batch-cancel-top"];

test("picking a file shows the busy region before any file bytes arrive", async ({ page }) => {
  await page.goto("/ui/batch", { waitUntil: "networkidle" });
  // Park the read behind a manual release: the region must be up before the
  // file is even delivered, which is the "within one frame of the pick"
  // criterion made testable. readAsText resolves through the prototype at
  // call time, so patching it here works.
  await page.evaluate(() => {
    const orig = FileReader.prototype.readAsText;
    (window as { __releaseRead?: () => void }).__releaseRead = undefined;
    FileReader.prototype.readAsText = function (this: FileReader, ...args: [Blob]) {
      (window as { __releaseRead?: () => void }).__releaseRead = () => orig.apply(this, args);
    };
  });
  // Focus the drop zone first: the real flow clicks it, and the focus
  // fixup for its hidden stage is asynchronous — the containment check in
  // batch.js must still re-home focus.
  await page.locator("#batch-drop").focus();
  await page.locator("#batch-file").setInputFiles(bundleFile("batch"));
  const busy = page.locator("#batch-busy");
  await expect(busy).toBeVisible();
  // The real copy, not the Fluent key: a missing key renders as the key
  // itself and /reading/i would happily match "batch-reading".
  await expect(busy).toContainText("Reading bundle");
  await expect(busy).toHaveAttribute("role", "status");

  await page.evaluate(() => (window as { __releaseRead?: () => void }).__releaseRead?.());
  await expect(page.locator("#batch-preflight")).toBeVisible();
  await expect(busy).toBeHidden();
  // The picked file's drop zone was hidden with its stage; focus lands on
  // the revealed stage (tabindex=-1), not its destructive primary action.
  await expect(page.locator("#batch-preflight")).toBeFocused();
});

test("execute busies the whole footer, ignores re-entrant clicks, and lands focus on Done", async ({ page }) => {
  // setInputFiles + preflight render + an axe scan + a real POST: the
  // tightest budget in this file on the CI backend matrix.
  test.slow();

  let executeRequests = 0;
  let release!: () => void;
  const parked = new Promise<void>((resolve) => { release = resolve; });
  await page.route((url) => url.pathname === "/", async (route) => {
    if (route.request().method() !== "POST") return route.continue().catch(() => {});
    executeRequests++;
    await parked;
    await route.continue().catch(() => {});
  });

  await page.goto("/ui/batch", { waitUntil: "networkidle" });
  await page.locator("#batch-file").setInputFiles(bundleFile("batch"));
  await expect(page.locator("#batch-preflight")).toBeVisible();
  await page.locator("#batch-execute").click();

  // Both Execute copies spin, and the Cancels go inert with them: a
  // mid-flight Cancel raced the settling response and crashed on the nulled
  // bundle before #679.
  await expect(page.locator("#batch-execute")).toHaveAttribute("aria-busy", "true");
  await expect(page.locator("#batch-execute-top")).toHaveAttribute("aria-busy", "true");
  for (const control of FOOTER_CONTROLS) await expect(page.locator(control)).toBeDisabled();
  await expect(page.locator("#batch-busy")).toBeVisible();
  await expect(page.locator("#batch-busy")).toContainText("Executing");

  // The default-motion busy button: the label yields to the animated ring,
  // and the filled primary gets the explicit white ring (accent-on-accent
  // vanishes in dark theme).
  const busyStyle = await page.locator("#batch-execute").evaluate((button) => ({
    color: getComputedStyle(button).color,
    content: getComputedStyle(button, "::after").content,
    animation: getComputedStyle(button, "::after").animationName,
    ringTop: getComputedStyle(button, "::after").borderTopColor,
  }));
  expect(busyStyle.color).toBe("rgba(0, 0, 0, 0)");
  expect(busyStyle.content).toBe('""');
  expect(busyStyle.animation).toBe("spin");
  expect(busyStyle.ringTop).toBe("rgb(255, 255, 255)");

  // Re-entrant activation cannot double-POST: a real click on a disabled
  // button dispatches nothing, and a synthetic event that does reach the
  // handler is ignored by the busy guard.
  await page.locator("#batch-execute").evaluate((button: HTMLButtonElement) => button.click());
  await page
    .locator("#batch-execute-top")
    .evaluate((button) => button.dispatchEvent(new MouseEvent("click", { bubbles: true })));

  // The a11y gate scans routes at rest, so the held-open busy state is
  // audited here or nowhere.
  const { violations } = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
    .analyze();
  expect(violations, axeSummary(violations)).toEqual([]);

  release();
  await expect(page.locator("#batch-response")).toBeVisible();
  await expect(page.locator("#batch-busy")).toBeHidden();
  // Only now is the count meaningful: every request the page could have
  // issued has been intercepted (a sync read here raced the second click).
  expect(executeRequests).toBe(1);
  // The disabled trigger was hidden with its stage, so focus would die on
  // <body>; it lands on the one action the response offers instead.
  await expect(page.locator("#batch-done")).toBeFocused();
});

test("a whole-bundle failure clears the busy state and re-enables the footer", async ({ page }) => {
  let release!: () => void;
  const parked = new Promise<void>((resolve) => { release = resolve; });
  await page.route((url) => url.pathname === "/", async (route) => {
    if (route.request().method() !== "POST") return route.continue().catch(() => {});
    await parked;
    await route
      .fulfill({
        status: 400,
        contentType: "application/fhir+json",
        body: JSON.stringify({
          resourceType: "OperationOutcome",
          issue: [{ severity: "error", code: "processing", diagnostics: "boom" }],
        }),
      })
      .catch(() => {});
  });

  await page.goto("/ui/batch", { waitUntil: "networkidle" });
  await page.locator("#batch-file").setInputFiles(bundleFile("batch"));
  await page.locator("#batch-execute").click();

  // The busy state is genuinely entered before the failure lands…
  await expect(page.locator("#batch-execute")).toHaveAttribute("aria-busy", "true");
  await expect(page.locator("#batch-busy")).toBeVisible();
  release();

  // …and a rolled-back bundle clears every part of it: the early-return
  // branch stays on the preflight with the inline error (#676).
  await expect(page.locator("#batch-execute-error")).toBeVisible();
  await expect(page.locator("#batch-preflight")).toBeVisible();
  for (const control of FOOTER_CONTROLS) await expect(page.locator(control)).toBeEnabled();
  await expect(page.locator("#batch-execute")).not.toHaveAttribute("aria-busy", "true");
  await expect(page.locator("#batch-busy")).toBeHidden();
  // Focus returns to the trigger, which sits next to the inline error (#676).
  await expect(page.locator("#batch-execute")).toBeFocused();
});

test("reduced-motion users get a static ring, not an animated one", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  let release!: () => void;
  const parked = new Promise<void>((resolve) => { release = resolve; });
  await page.route((url) => url.pathname === "/", async (route) => {
    if (route.request().method() !== "POST") return route.continue().catch(() => {});
    await parked;
    await route.continue().catch(() => {});
  });

  await page.goto("/ui/batch", { waitUntil: "networkidle" });
  await page.locator("#batch-file").setInputFiles(bundleFile("batch"));
  await page.locator("#batch-execute").click();
  await expect(page.locator("#batch-execute")).toHaveAttribute("aria-busy", "true");

  // The static form: the ring glyph is present but does not animate. A
  // label-only dimmed button would read as "disabled", not "working".
  const after = await page.locator("#batch-execute").evaluate((button) => ({
    content: getComputedStyle(button, "::after").content,
    animation: getComputedStyle(button, "::after").animationName,
  }));
  expect(after.content).toBe('""');
  expect(after.animation).toBe("none");

  release();
  await expect(page.locator("#batch-response")).toBeVisible();
});

test("an unreadable file clears the busy region and reports the failure", async ({ page }) => {
  await page.goto("/ui/batch", { waitUntil: "networkidle" });
  // A folder drop or a file that vanished between pick and read fires
  // `error`, never `load` — the busy region must not be left spinning.
  await page.evaluate(() => {
    FileReader.prototype.readAsText = function (this: FileReader) {
      setTimeout(() => this.dispatchEvent(new ProgressEvent("error")), 0);
    };
  });
  await page.locator("#batch-file").setInputFiles(bundleFile("batch"));
  await expect(page.locator("#batch-upload-error")).toBeVisible();
  await expect(page.locator("#batch-busy")).toBeHidden();
  await expect(page.locator("#batch-preflight")).toBeHidden();
});

test("a synchronously-failing operation cannot leave a stale region label", async ({ page }) => {
  await page.goto("/ui/batch", { waitUntil: "networkidle" });
  // Drives the helper directly: clear() runs as a microtask and must also
  // cancel the label write queued for the next macrotask, or the hidden
  // region keeps the stale label and announces it on its next reveal.
  const state = await page.evaluate(() => {
    const region = document.getElementById("batch-busy") as HTMLElement;
    const button = document.getElementById("batch-execute") as HTMLButtonElement;
    const busyApi = (window as { hfsBusy?: { during: Function } }).hfsBusy!;
    busyApi.during(
      [button],
      () => {
        throw new Error("sync");
      },
      { region, label: "Stale label" },
    );
    return new Promise<{ hidden: boolean; label: string | null; busyAttr: string | null }>(
      (resolve) =>
        setTimeout(
          () =>
            resolve({
              hidden: region.hidden,
              label: region.querySelector("[data-busy-label]")!.textContent,
              busyAttr: button.getAttribute("aria-busy"),
            }),
          30,
        ),
    );
  });
  expect(state.hidden).toBe(true);
  expect(state.label).toBe("");
  expect(state.busyAttr).toBeNull();
});
