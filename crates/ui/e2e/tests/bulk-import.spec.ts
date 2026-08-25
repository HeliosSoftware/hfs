import { test, expect } from "../pages/fixtures";
import type { Locator } from "@playwright/test";

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
