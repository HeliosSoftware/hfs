import { test, expect } from "../pages/fixtures";
import type { Locator } from "@playwright/test";
import { seedBulkImportDetail } from "../pages/routes";
import {
  CANONICAL_BUTTON_GEOMETRY,
  readButtonGeometries,
} from "../pages/button-geometry";

function contrastRatio(foreground: string, background: string): number {
  const luminance = (cssColor: string): number => {
    const channels = cssColor.match(/[\d.]+/g)?.slice(0, 3).map(Number);
    if (!channels || channels.length !== 3) throw new Error(`cannot parse CSS color: ${cssColor}`);
    const [r, g, b] = channels.map((channel) => {
      const value = channel / 255;
      return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
    });
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
  };
  const fore = luminance(foreground);
  const back = luminance(background);
  return (Math.max(fore, back) + 0.05) / (Math.min(fore, back) + 0.05);
}

const VIEWPORTS = [
  { name: "wide", width: 1900, height: 900, columns: 2 },
  { name: "short", width: 1536, height: 360, columns: 2 },
  { name: "compact", width: 1100, height: 620, columns: 1 },
] as const;

for (const viewport of VIEWPORTS) {
  test(`detail summary stays full-width and cards stay separated — ${viewport.name}`, async ({
    page,
    request,
    bulkImport,
  }) => {
    await page.setViewportSize({ width: viewport.width, height: viewport.height });
    await bulkImport.seedAndGoto(request, `layout-${viewport.name}`);

    const summaryLayout = await bulkImport.summary.evaluate((element) => {
      const style = getComputedStyle(element);
      const grid = element.querySelector<HTMLElement>(".kv-grid")!;
      return {
        position: style.position,
        maxHeight: style.maxHeight,
        overflowY: style.overflowY,
        scrolls: element.scrollHeight > element.clientHeight,
        columns: getComputedStyle(grid).gridTemplateColumns.split(/\s+/).filter(Boolean).length,
      };
    });
    expect(summaryLayout).toEqual({
      position: "static",
      maxHeight: "none",
      overflowY: "visible",
      scrolls: false,
      columns: viewport.columns,
    });

    const boxes = await Promise.all(
      [bulkImport.summary, bulkImport.manifestsCard, bulkImport.logCard].map((card) =>
        card.boundingBox(),
      ),
    );
    expect(boxes.every(Boolean)).toBe(true);
    const gaps = [
      boxes[1]!.y - (boxes[0]!.y + boxes[0]!.height),
      boxes[2]!.y - (boxes[1]!.y + boxes[1]!.height),
    ];
    for (const gap of gaps) expect(gap).toBeGreaterThanOrEqual(15);
  });
}

for (const viewport of [
  { name: "wide", width: 1440, height: 1024 },
  { name: "narrow", width: 390, height: 844 },
] as const) {
  test(`summary actions follow the responsive Figma composition — ${viewport.name}`, async ({
    page,
    request,
    bulkImport,
  }) => {
    await page.setViewportSize(viewport);
    await bulkImport.seedAndGoto(request, `summary-actions-${viewport.name}`);

    const metadata = bulkImport.summaryGrid;
    const actions = bulkImport.summary.locator(".bulk-import-summary__actions");
    const [summaryBounds, metadataBounds, actionBounds] = await Promise.all([
      bulkImport.summary.boundingBox(),
      metadata.boundingBox(),
      actions.boundingBox(),
    ]);
    expect(summaryBounds).not.toBeNull();
    expect(metadataBounds).not.toBeNull();
    expect(actionBounds).not.toBeNull();

    if (viewport.name === "wide") {
      expect(actionBounds!.x).toBeGreaterThan(metadataBounds!.x + metadataBounds!.width);
      expect(Math.abs(actionBounds!.y - metadataBounds!.y)).toBeLessThanOrEqual(1);
    } else {
      expect(actionBounds!.y).toBeGreaterThanOrEqual(metadataBounds!.y + metadataBounds!.height);
      expect(actionBounds!.x).toBeGreaterThanOrEqual(summaryBounds!.x);
      expect(actionBounds!.x + actionBounds!.width).toBeLessThanOrEqual(
        summaryBounds!.x + summaryBounds!.width,
      );
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= document.documentElement.clientWidth,
        ),
      ).toBe(true);
    }
  });
}

for (const theme of ["light", "dark"] as const) {
  test(`back link, destructive action, and empty states use shared treatments — ${theme}`, async ({
    request,
    bulkImport,
    chrome,
  }) => {
    await chrome.seedTheme(theme);
    await bulkImport.seedAndGoto(request, `shared-treatments-${theme}`);

    const normalLink = await bulkImport.backLink.evaluate((element) => {
      const style = getComputedStyle(element);
      return { color: style.color, decoration: style.textDecorationLine };
    });
    await bulkImport.backLink.hover();
    const hoverLink = await bulkImport.backLink.evaluate((element) => {
      const style = getComputedStyle(element);
      return { color: style.color, decoration: style.textDecorationLine };
    });
    expect(hoverLink.color).toBe(normalLink.color);
    expect(hoverLink.decoration).toContain("underline");

    await bulkImport.backLink.focus();
    const focusLink = await bulkImport.backLink.evaluate((element) => {
      const style = getComputedStyle(element);
      return {
        color: style.color,
        outlineColor: style.outlineColor,
        outlineStyle: style.outlineStyle,
        outlineWidth: style.outlineWidth,
      };
    });
    expect(focusLink.outlineStyle).toBe("solid");
    expect(focusLink.outlineWidth).toBe("2px");
    expect(focusLink.outlineColor).toBe(focusLink.color);

    await expect(bulkImport.deleteButton).toHaveClass(/\bbtn--danger\b/);
    const normalButton = bulkImport.summary.locator(".btn:not(.btn--danger)").first();
    const normal = await normalButton.evaluate((element) => {
      const style = getComputedStyle(element);
      return { color: style.color, borderColor: style.borderColor };
    });
    const danger = await bulkImport.deleteButton.evaluate((element) => {
      const style = getComputedStyle(element);
      const probe = document.createElement("span");
      probe.style.color = "var(--danger)";
      probe.style.backgroundColor = "var(--danger-soft)";
      probe.style.borderColor = "var(--bg-content)";
      document.body.append(probe);
      const probeStyle = getComputedStyle(probe);
      const tokens = {
        danger: probeStyle.color,
        dangerSoft: probeStyle.backgroundColor,
        contentSurface: probeStyle.borderTopColor,
      };
      probe.remove();
      return {
        color: style.color,
        borderColor: style.borderColor,
        backgroundColor: style.backgroundColor,
        backgroundImage: style.backgroundImage,
        tokens,
      };
    });
    expect(danger.color).toBe(danger.tokens.danger);
    expect(danger.borderColor).toBe(danger.tokens.danger);
    expect(danger.color).not.toBe(normal.color);
    expect(danger.borderColor).not.toBe(normal.borderColor);
    expect(contrastRatio(danger.tokens.danger, danger.tokens.contentSurface)).toBeGreaterThanOrEqual(
      4.5,
    );

    await bulkImport.deleteButton.hover();
    const dangerHover = await bulkImport.deleteButton.evaluate((element) => {
      const style = getComputedStyle(element);
      return {
        color: style.color,
        backgroundColor: style.backgroundColor,
        backgroundImage: style.backgroundImage,
      };
    });
    expect(dangerHover.color).toBe(danger.tokens.danger);
    expect(dangerHover.backgroundColor).toBe(danger.tokens.dangerSoft);
    expect(dangerHover.backgroundColor).not.toBe(danger.backgroundColor);
    expect(dangerHover.backgroundImage).toBe("none");
    expect(contrastRatio(dangerHover.color, dangerHover.backgroundColor)).toBeGreaterThanOrEqual(
      4.5,
    );

    const emptySignature = async (locator: Locator) =>
      locator.evaluate((element) => {
        const style = getComputedStyle(element);
        return {
          color: style.color,
          textAlign: style.textAlign,
          paddingTop: style.paddingTop,
          paddingRight: style.paddingRight,
          paddingBottom: style.paddingBottom,
          paddingLeft: style.paddingLeft,
          backgroundColor: style.backgroundColor,
        };
      });
    const manifestEmpty = await emptySignature(bulkImport.manifestEmptyState);
    const logEmpty = await emptySignature(bulkImport.logEmptyState);
    expect(manifestEmpty).toEqual(logEmpty);

    await bulkImport.manifestEmptyState.hover();
    expect((await emptySignature(bulkImport.manifestEmptyState)).backgroundColor).toBe(
      manifestEmpty.backgroundColor,
    );
  });
}

// Bulk Import dialogs: dismissal clears the form (#682), the Add Manifest
// field is labeled Format (#684), and its headers textarea matches the other
// fields instead of overflowing the panel (#685).
test("dismissing the New Submission dialog clears the typed form", async ({ page }) => {
  await page.goto("/ui/bulk-import");
  const toggle = page.locator("summary.btn", { hasText: "New Submission" });
  await toggle.click();
  const name = page.locator("input[name='name']");
  await name.fill("draft-i-abandoned");
  await page.keyboard.press("Escape");
  await toggle.click();
  await expect(name).toHaveValue("");
});

test("the Add Manifest dialog labels Format and sizes its textarea", async ({
  page,
  request,
}) => {
  const detail = await seedBulkImportDetail(request);
  await page.goto(detail);
  await page.locator("summary.btn", { hasText: "Add Manifest" }).click();
  // #684: the field reads Format, not Output format.
  await expect(page.locator(".field__label", { hasText: /^Format$/ })).toBeVisible();
  // #685: the textarea inherits the page font, sizes within the panel, and
  // only resizes vertically.
  const styles = await page
    .locator("textarea[name='file_request_headers']")
    .evaluate((el) => {
      const c = getComputedStyle(el);
      return { fontFamily: c.fontFamily, resize: c.resize, boxSizing: c.boxSizing };
    });
  expect(styles.resize).toBe("vertical");
  expect(styles.boxSizing).toBe("border-box");
  expect(styles.fontFamily.toLowerCase()).not.toContain("monospace");
});

test("manifest-row actions use the canonical scale across emphasis variants", async ({
  page,
  request,
  bulkImport,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  const detail = await bulkImport.seedAndGoto(request, `button-scale-${Date.now()}`);
  const response = await request.post(`${detail}/manifests`, {
    form: {
      manifest_url: "https://example.test/manifest.json",
      fhir_base_url: "https://example.test/fhir",
      output_format: "application/fhir+ndjson",
    },
    maxRedirects: 0,
  });
  expect(response.status()).toBeGreaterThanOrEqual(300);
  expect(response.status()).toBeLessThan(400);
  await page.goto(detail, { waitUntil: "networkidle" });

  const metrics = await readButtonGeometries(
    bulkImport.manifestsCard.locator(".bulk-import-manifest-row .btn"),
  );
  expect(metrics).toHaveLength(1);
  for (const geometry of metrics) expect(geometry).toEqual(CANONICAL_BUTTON_GEOMETRY);

  const menuTrigger = bulkImport.manifestsCard.locator(".bulk-import-manifest-menu__trigger");
  await expect(menuTrigger).toHaveCSS("width", "30px");
  await expect(menuTrigger).toHaveCSS("height", "30px");
  await expect(menuTrigger).toHaveCSS("border-radius", "9px");
  await expect(menuTrigger).toHaveCSS("border-top-width", "0px");
});

for (const viewport of [
  { width: 1440, height: 1024 },
  { width: 390, height: 844 },
] as const) {
  test(`long manifest URLs ellipsize without clipping the row menu — ${viewport.width}px`, async ({
    page,
    request,
    bulkImport,
  }) => {
    await page.setViewportSize(viewport);
    const detail = await bulkImport.seedAndGoto(request, `long-url-${viewport.width}`);
    const manifestUrl = `https://example.test/${"deeply-nested-segment/".repeat(16)}manifest.json?${"filter=Patient%2F".repeat(8)}`;
    const response = await request.post(`${detail}/manifests`, {
      form: { manifest_url: manifestUrl },
      maxRedirects: 0,
    });
    expect(response.status()).toBeGreaterThanOrEqual(300);
    expect(response.status()).toBeLessThan(400);
    await page.goto(detail, { waitUntil: "networkidle" });

    const url = page.locator(".bulk-import-manifest-row__url");
    await expect(url).toHaveText(manifestUrl);
    expect(
      await url.evaluate((element) => element.scrollWidth > element.clientWidth),
    ).toBe(true);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= document.documentElement.clientWidth,
      ),
    ).toBe(true);

    const trigger = page.locator(".bulk-import-manifest-menu__trigger");
    await trigger.focus();
    await page.keyboard.press("Enter");
    const panel = page.locator(".bulk-import-manifest-menu__panel");
    await expect(panel).toBeVisible();
    const bounds = await panel.boundingBox();
    expect(bounds!.x).toBeGreaterThanOrEqual(0);
    expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(viewport.width);
    await page.keyboard.press("Escape");
    await expect(panel).not.toBeVisible();
    await expect(trigger).toBeFocused();
  });
}

// #721: the New Submission summary doubles as the modal backdrop; the
// button's fixed height used to beat the inset stretch and shrink it to a
// bar across the top of the viewport, so clicking anywhere lower neither
// closed the dialog nor dimmed the page. The dialog also repeated the
// server-fixed recipient URL as a read-only row.
test("the open New Submission dialog backdrop covers the viewport", async ({ page }) => {
  await page.goto("/ui/bulk-import");
  const toggle = page.locator("summary.btn", { hasText: "New Submission" });
  await toggle.click();
  const backdrop = await toggle.boundingBox();
  const viewport = page.viewportSize()!;
  expect(backdrop!.height).toBeGreaterThan(viewport.height * 0.9);
  await expect(
    page.locator(".addbox__panel .field__label", { hasText: "Recipient base URL" }),
  ).toHaveCount(0);
  // Clicking the backdrop low on the screen closes the dialog.
  await page.mouse.click(viewport.width / 2, viewport.height - 10);
  await expect(page.locator(".addbox__panel")).not.toBeVisible();
});

// The Test authentication button wires an htmx post to a fragment target
// inside the dialog. The handler's outcome states are covered by the Rust
// ring; this asserts the round trip lands in #test-auth-result.
test("Test authentication reports its outcome inside the dialog", async ({ page }) => {
  await page.goto("/ui/bulk-import");
  await page.locator("summary.btn", { hasText: "New Submission" }).click();
  await page.locator("input[name='client_id']").fill("e2e-client");
  await page
    .locator("input[name='token_url']")
    .fill(new URL("/health", page.url()).toString());
  await page.locator("button[formaction='/ui/bulk-import/test-auth']").click();
  await expect(page.locator("#test-auth-result .field__hint")).toBeVisible();
});
