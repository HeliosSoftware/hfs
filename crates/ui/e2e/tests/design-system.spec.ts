import { test, expect } from "../pages/fixtures";
import type { APIRequestContext } from "@playwright/test";
import { ROUTES, seedBulkImportDetail } from "../pages/routes";
import { createResource, waitSearchable } from "../pages/api";
import { expectDetailFieldSpacing } from "../pages/detail-spacing";
import postcss from "postcss";

// The design-system guard (#543). The stylesheet defines one component
// vocabulary (crates/ui/README.md, "Component vocabulary"); these tests make
// divergence executable instead of a review judgement:
//   - a class that matches no rule is a typo or a second vocabulary starting
//     (class="table" shipped that way and styled nothing);
//   - a page <h1> off the canonical class is a second heading scale;
//   - ordinary actions use one exact size scale, regardless of emphasis;
//   - a selector defined twice renders as the cascade-merge of two authors'
//     blocks, which nobody wrote.

// Classes added at runtime by vendored libraries, not defined in app.css.
const RUNTIME_CLASSES = /^htmx-/;

// The assets are read over HTTP from the server under test, not from the
// source tree: the CI runner drives a packaged binary with no checkout
// alongside it, and in HFS_E2E_BASE_URL mode the running server is the only
// truth worth checking anyway.
async function fetchAsset(request: APIRequestContext, path: string): Promise<string> {
  const res = await request.get(path);
  if (!res.ok()) throw new Error(`${path} -> ${res.status()}`);
  return res.text();
}

async function stylesheet(request: APIRequestContext): Promise<string> {
  return fetchAsset(request, "/ui/assets/app.css");
}

// A class is load-bearing if a rule styles it — or if shipped script selects
// it (a JS hook like .json-line--foldable). Only dot-prefixed names inside JS
// string literals count, so createElement("table") never legitimizes a dead
// class="table". Scripts load per page, so the sweep collects every
// `<script src>` it encounters and legitimizes across the union.
async function jsHookClasses(request: APIRequestContext, sources: Set<string>): Promise<Set<string>> {
  const found = new Set<string>();
  for (const src of sources) {
    if (src.endsWith("htmx.min.js")) continue;
    const js = await fetchAsset(request, src);
    for (const str of js.matchAll(/"(?:[^"\\\n]|\\.)*"|'(?:[^'\\\n]|\\.)*'|`(?:[^`\\]|\\.)*`/g)) {
      for (const m of str[0].matchAll(/\.(-?[A-Za-z_][A-Za-z0-9_-]*)/g)) found.add(m[1]);
    }
  }
  return found;
}

test("every class used on every page matches a rule in app.css", async ({ page, request }) => {
  const defined = new Set<string>();
  const css = (await stylesheet(request)).replace(/\/\*[\s\S]*?\*\//g, "");
  for (const m of css.matchAll(/\.(-?[A-Za-z_][A-Za-z0-9_-]*)/g)) defined.add(m[1]);

  // One pass over every route: the classes in use, and the scripts each page
  // actually ships.
  const usedByRoute = new Map<string, string[]>();
  const scripts = new Set<string>();
  for (const route of [...ROUTES, await seedBulkImportDetail(request)]) {
    await page.goto(route, { waitUntil: "networkidle" });
    const { used, sources } = await page.evaluate(() => ({
      used: Array.from(
        new Set(Array.from(document.querySelectorAll("*")).flatMap((el) => Array.from(el.classList))),
      ),
      sources: Array.from(document.querySelectorAll("script[src]"))
        .map((n) => n.getAttribute("src") ?? "")
        .filter((s) => s.startsWith("/ui/assets/")),
    }));
    usedByRoute.set(route, used);
    sources.forEach((s) => scripts.add(s));
  }
  for (const cls of await jsHookClasses(request, scripts)) defined.add(cls);

  const offenders: string[] = [];
  for (const [route, used] of usedByRoute) {
    for (const cls of used) {
      if (RUNTIME_CLASSES.test(cls)) continue;
      if (!defined.has(cls)) offenders.push(`${route}: .${cls}`);
    }
  }
  expect(
    offenders,
    `classes with no matching rule in app.css (typo, or a second vocabulary starting):\n${offenders.join("\n")}`,
  ).toEqual([]);
});

test("every page <h1> uses the canonical .page-head__title", async ({ page, request }) => {
  const offenders: string[] = [];
  for (const route of [...ROUTES, await seedBulkImportDetail(request)]) {
    await page.goto(route, { waitUntil: "domcontentloaded" });
    const headings = await page.$$eval("h1", (nodes) =>
      nodes.map((n) => ({ classes: n.className, text: (n.textContent ?? "").trim().slice(0, 40) })),
    );
    expect(headings.length, `${route} should have exactly one <h1>`).toBe(1);
    for (const h of headings) {
      if (!h.classes.split(/\s+/).includes("page-head__title")) {
        offenders.push(`${route}: <h1 class="${h.classes}"> (“${h.text}”)`);
      }
    }
  }
  expect(offenders, `page titles off the canonical class:\n${offenders.join("\n")}`).toEqual([]);
});

test("every rendered table uses the canonical .data-table", async ({ page, request }) => {
  const offenders: string[] = [];
  for (const route of [...ROUTES, await seedBulkImportDetail(request)]) {
    await page.goto(route, { waitUntil: "networkidle" });
    const tables = await page.$$eval("table:not(.data-table)", (nodes) =>
      nodes.map((node) => {
        const id = node.id ? `#${node.id}` : "";
        const classes = Array.from(node.classList)
          .map((name) => `.${name}`)
          .join("");
        return `${node.tagName.toLowerCase()}${id}${classes}`;
      }),
    );
    for (const table of tables) offenders.push(`${route}: ${table}`);
  }
  expect(offenders, `tables off the canonical class:\n${offenders.join("\n")}`).toEqual([]);
});

test("data tables share one alignment contract", async ({ page }) => {
  await page.goto("/ui");
  await page.evaluate(() => {
    const probe = document.createElement("table");
    probe.id = "data-table-contract";
    probe.className = "data-table";
    probe.innerHTML = `
      <thead>
        <tr>
          <th data-probe="ordinary-header">Name</th>
          <th class="col-num" data-probe="numeric-header">Count</th>
          <th class="col-actions" data-probe="actions-header">Actions</th>
        </tr>
      </thead>
      <tbody>
        <tr>
          <td data-probe="ordinary-cell">Example</td>
          <td class="col-num" data-probe="numeric-cell">123</td>
          <td class="col-actions" data-probe="actions-cell"></td>
        </tr>
        <tr class="data-table__empty">
          <td class="col-actions" colspan="3" data-probe="empty-cell">No results</td>
        </tr>
      </tbody>
    `;
    document.body.append(probe);
  });

  const probe = page.locator("#data-table-contract");
  for (const name of ["ordinary-header", "ordinary-cell", "numeric-header", "numeric-cell"]) {
    await expect(probe.locator(`[data-probe='${name}']`)).toHaveCSS("text-align", "left");
  }
  for (const name of ["numeric-header", "numeric-cell"]) {
    await expect(probe.locator(`[data-probe='${name}']`)).toHaveCSS("font-variant-numeric", "tabular-nums");
  }
  for (const name of ["actions-header", "actions-cell"]) {
    const action = probe.locator(`[data-probe='${name}']`);
    await expect(action).toHaveCSS("text-align", "right");
  }
  await expect(probe.locator("[data-probe='empty-cell']")).toHaveCSS("text-align", "center");
});

test("ordinary actions resolve to the canonical button scale on every page", async ({ page, request }) => {
  const offenders: string[] = [];
  const primaryColors = new Map<string, string[]>();
  for (const route of [...ROUTES, await seedBulkImportDetail(request)]) {
    await page.goto(route, { waitUntil: "networkidle" });
    const actions = await page.$$eval(".btn, .icon-button", (nodes) =>
      nodes
        .filter((n) => n instanceof HTMLElement && n.offsetParent !== null)
        .map((n) => {
          const s = getComputedStyle(n);
          const box = n.getBoundingClientRect();
          return {
            label: `${n.tagName.toLowerCase()}.${Array.from(n.classList).join(".")} “${(n.textContent ?? "").trim().slice(0, 30)}”`,
            isButton: n.classList.contains("btn"),
            isIconButton: n.classList.contains("icon-button"),
            isIconShape: n.classList.contains("btn--icon"),
            isPrimary: n.classList.contains("btn--primary"),
            height: s.height,
            width: s.width,
            radius: s.borderRadius,
            fontSize: s.fontSize,
            paddingLeft: s.paddingLeft,
            paddingRight: s.paddingRight,
            background: s.backgroundColor,
            color: s.color,
            targetWidth: box.width,
            targetHeight: box.height,
          };
        }),
    );
    const routePrimaryColors: string[] = [];
    for (const action of actions) {
      if (action.targetWidth < 24 || action.targetHeight < 24) {
        offenders.push(`${route}: ${action.label} target=${action.targetWidth}x${action.targetHeight}`);
      }
      if (action.isIconButton) {
        if (action.width !== "30px" || action.height !== "30px" || action.radius !== "9px") {
          offenders.push(`${route}: ${action.label} icon geometry=${action.width}x${action.height} radius=${action.radius}`);
        }
        continue;
      }
      if (!action.isButton) continue;
      if (action.height !== "30px" || action.radius !== "9px" || action.fontSize !== "12px") {
        offenders.push(`${route}: ${action.label} height=${action.height} radius=${action.radius} font=${action.fontSize}`);
      }
      const expectedPadding = action.isIconShape ? "0px" : "12px";
      if (action.paddingLeft !== expectedPadding || action.paddingRight !== expectedPadding) {
        offenders.push(`${route}: ${action.label} padding=${action.paddingLeft}/${action.paddingRight}`);
      }
      if (action.isIconShape && action.width !== "30px") {
        offenders.push(`${route}: ${action.label} icon width=${action.width}`);
      }
      if (action.isPrimary) routePrimaryColors.push(`${action.background}/${action.color}`);
    }
    if (routePrimaryColors.length) primaryColors.set(route, Array.from(new Set(routePrimaryColors)));
  }
  expect(offenders, `actions off the canonical scale or below 24px:\n${offenders.join("\n")}`).toEqual([]);

  const distinctPrimaryColors = new Set(Array.from(primaryColors.values()).flat());
  const listing = Array.from(primaryColors, ([r, colors]) => `${r}: ${colors.join(" | ")}`).join("\n");
  expect(distinctPrimaryColors.size, `primary emphasis diverges across pages:\n${listing}`).toBeLessThanOrEqual(1);
});

test("emphasis variants cannot declare geometry and the retired accent variant stays absent", async ({ request }) => {
  const css = await stylesheet(request);
  const root = postcss.parse(css);
  const geometry = /^(?:width|min-width|max-width|height|min-height|max-height|padding(?:-.+)?|font(?:-.+)?|line-height|border-radius|gap)$/;
  const offenders: string[] = [];
  const contextual: string[] = [];
  root.walkRules((rule) => {
    const emphasis = rule.selectors.filter((selector) => /\.btn--(?:primary|danger|current)(?![-\w])/.test(selector));
    rule.walkDecls((decl) => {
      if (!geometry.test(decl.prop)) return;
      if (emphasis.length) offenders.push(`${emphasis.join(", ")}: ${decl.prop}: ${decl.value}`);
      for (const selector of rule.selectors) {
        const target = selector.trim();
        if (!/\.btn(?:--[\w-]+)?(?:\[[^\]]+\]|:[\w()-]+)*$/.test(target)) continue;
        if (target === ".btn" || target === ".btn--icon" || target === ".addbox--modal[open] > summary.btn") continue;
        contextual.push(`${target}: ${decl.prop}: ${decl.value}`);
      }
    });
  });
  expect(offenders, `emphasis variants declaring geometry:\n${offenders.join("\n")}`).toEqual([]);
  expect(contextual, `contextual button geometry outside the icon/backdrop exceptions:\n${contextual.join("\n")}`).toEqual([]);
  expect(css).not.toContain(".btn--accent");
});

test("shared query-builder actions stay 30px while their inputs keep the field scale", async ({ page }) => {
  for (const route of ["/ui/resources", "/ui/search", "/ui/queries"]) {
    await page.goto(route, { waitUntil: "networkidle" });
    const form = page.locator("#saved-query-form");
    await expect(form, `${route} should render the shared query builder`).toBeVisible();
    for (const control of ["#query-copy", ".query-recent__toggle", "[data-intent='run']"]) {
      await expect(form.locator(control)).toHaveCSS("height", "30px");
    }
    await expect(form.locator("#query-copy")).toHaveCSS("width", "30px");
    await expect(form.locator("#query-copy")).toHaveCSS("padding-left", "0px");
    await expect(form.locator(".query-builder__url")).toHaveCSS("height", "38px");

    if (route === "/ui/queries") {
      await expect(form.locator("[data-intent='save']")).toHaveCSS("height", "30px");
      await expect(form.locator("input[name='name']")).toHaveCSS("height", "36px");
    }
  }

  for (const theme of ["light", "dark"] as const) {
    for (const width of [1280, 390]) {
      await page.setViewportSize({ width, height: 800 });
      await page.goto("/ui");
      await page.evaluate((selected) => localStorage.setItem("hfs-theme", selected), theme);
      await page.goto("/ui/queries", { waitUntil: "networkidle" });
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      for (const control of ["#query-copy", ".query-recent__toggle", "[data-intent='run']", "[data-intent='save']"]) {
        await expect(page.locator(`#saved-query-form ${control}`)).toHaveCSS("height", "30px");
      }
    }
  }
});

test("shared back links match the Figma geometry on every consumer", async ({
  page,
  request,
}) => {
  const css = postcss.parse(await stylesheet(request));
  let sourceRule:
    | { selectors: string[]; declarations: Record<string, string> }
    | undefined;
  css.walkRules((rule) => {
    if (!rule.selectors.includes(".back-link") || !rule.selectors.includes(".back-link:visited")) {
      return;
    }
    const declarations: Record<string, string> = {};
    rule.walkDecls((declaration) => {
      declarations[declaration.prop] = declaration.value;
    });
    sourceRule = { selectors: rule.selectors, declarations };
  });
  expect(sourceRule?.selectors).toEqual(
    expect.arrayContaining([".back-link", ".back-link:visited"]),
  );
  expect(sourceRule?.declarations).toMatchObject({
    display: "inline-flex",
    "align-items": "center",
    gap: "7px",
    "margin-bottom": "24px",
    "font-size": "13px",
    color: "var(--accent-text)",
    "text-decoration": "none",
  });
  let headerRule: Record<string, string> | undefined;
  css.walkRules((rule) => {
    if (!rule.selectors.includes(".page-head--back-link")) return;
    const declarations: Record<string, string> = {};
    rule.walkDecls((declaration) => {
      declarations[declaration.prop] = declaration.value;
    });
    headerRule = declarations;
  });
  expect(headerRule).toMatchObject({
    display: "grid",
    "grid-template-columns": "minmax(0, 1fr) auto",
    "column-gap": "16px",
    "align-items": "start",
  });
  expect(headerRule?.["grid-template-areas"]).toContain('"back-link ."');
  expect(headerRule?.["grid-template-areas"]).toContain('"copy action"');

  // One-shot bulk import (#781) dropped the detail page's action slot; its
  // copy column simply spans. The shared link geometry still binds both.
  const consumers = [
    { name: "bulk import detail", route: await seedBulkImportDetail(request), hasAction: false },
    { name: "new bulk export", route: "/ui/bulk-export/new", hasAction: false },
  ];
  for (const theme of ["light", "dark"] as const) {
    await page.goto("/ui");
    await page.evaluate((selected) => localStorage.setItem("hfs-theme", selected), theme);
    for (const viewport of [
      { name: "wide", width: 1440, height: 900 },
      { name: "narrow", width: 390, height: 844 },
    ] as const) {
      await page.setViewportSize(viewport);
      for (const consumer of consumers) {
        await page.goto(consumer.route, { waitUntil: "networkidle" });
        await expect(page.locator("html"), `${consumer.name} should use ${theme} theme`).toHaveAttribute(
          "data-theme",
          theme,
        );

        const link = page.locator("a.back-link");
        const icon = link.locator("svg");
        const label = link.locator(":scope > span").last();
        const title = page.locator(".page-head__title");
        const action = page.locator(".page-head--back-link > .page-head__action");
        const actionControl = action.locator(
          ":scope > a.btn, :scope > details > summary.btn",
        );
        if (consumer.hasAction) {
          await expect(action).toHaveCount(1);
          await expect(actionControl).toHaveCount(1);
          await expect(actionControl).toBeVisible();
        } else {
          await expect(action).toHaveCount(0);
        }
        // Grid items are blockified, so the authored inline-flex computes to
        // flex here. The source-rule assertion above guards the shared value.
        await expect(link).toHaveCSS("display", "flex");
        await expect(link).toHaveCSS("font-size", "13px");
        await expect(link).toHaveCSS("gap", "7px");
        await expect(link).toHaveCSS("margin-bottom", "24px");
        await expect(icon).toHaveAttribute("width", "5");
        await expect(icon).toHaveAttribute("height", "8");

        const [linkBox, iconBox, labelBox, titleBox] = await Promise.all([
          link.boundingBox(),
          icon.boundingBox(),
          label.boundingBox(),
          title.boundingBox(),
        ]);
        const actionBox = consumer.hasAction ? await action.boundingBox() : null;
        expect(linkBox).not.toBeNull();
        expect(iconBox).not.toBeNull();
        expect(labelBox).not.toBeNull();
        expect(titleBox).not.toBeNull();
        expect(Math.abs(labelBox!.x - (iconBox!.x + iconBox!.width) - 7)).toBeLessThanOrEqual(1);
        expect(Math.abs(titleBox!.y - (linkBox!.y + linkBox!.height) - 24)).toBeLessThanOrEqual(1);
        if (consumer.hasAction) {
          expect(actionBox).not.toBeNull();
          expect(actionBox!.y).toBeGreaterThan(linkBox!.y + linkBox!.height);
          expect(Math.abs(actionBox!.y - titleBox!.y)).toBeLessThanOrEqual(1);
        }

        const normal = await link.evaluate((element) => {
          const style = getComputedStyle(element);
          return { color: style.color, decoration: style.textDecorationLine };
        });
        expect(normal.decoration).toBe("none");
        await link.hover();
        await expect(link).toHaveCSS("color", normal.color);
        await expect(link).toHaveCSS("text-decoration-line", /underline/);
        await link.focus();
        await expect(link).toHaveCSS("outline-style", "solid");
        await expect(link).toHaveCSS("outline-width", "2px");
        await expect(link).toHaveCSS("outline-color", normal.color);

        if (viewport.name === "narrow") {
          expect(
            await page.evaluate(
              () => document.documentElement.scrollWidth <= document.documentElement.clientWidth,
            ),
          ).toBe(true);
        }
      }
    }
  }
});

test("icon-only action shapes compute to the shared 30px square", async ({ page }) => {
  await page.goto("/ui");
  const metrics = await page.evaluate(() => {
    const classes = ["btn btn--icon", "icon-button"];
    return classes.map((className) => {
      const probe = document.createElement("button");
      probe.className = className;
      probe.type = "button";
      probe.textContent = "x";
      document.body.append(probe);
      const style = getComputedStyle(probe);
      const result = {
        className,
        width: style.width,
        height: style.height,
        radius: style.borderRadius,
        paddingLeft: style.paddingLeft,
        paddingRight: style.paddingRight,
      };
      probe.remove();
      return result;
    });
  });
  expect(metrics).toEqual([
    { className: "btn btn--icon", width: "30px", height: "30px", radius: "9px", paddingLeft: "0px", paddingRight: "0px" },
    { className: "icon-button", width: "30px", height: "30px", radius: "9px", paddingLeft: "0px", paddingRight: "0px" },
  ]);
});

test("no selector is defined twice in app.css", async ({ request }) => {
  const css = await stylesheet(request);
  const root = postcss.parse(css);
  const seen = new Map<string, number[]>();
  root.walkRules((rule) => {
    // Context = enclosing at-rules EXCEPT @layer: the layer a rule lives in
    // must not license a second definition, but the same selector inside a
    // different @media block is a legitimate refinement.
    const ctx: string[] = [];
    let p: any = rule.parent;
    while (p && p.type === "atrule") {
      if (p.name !== "layer") ctx.push(`@${p.name} ${p.params}`);
      p = p.parent;
    }
    const key =
      ctx.reverse().join(" | ") +
      " :: " +
      rule.selectors.map((s) => s.replace(/\s+/g, " ").trim()).sort().join(", ");
    if (!seen.has(key)) seen.set(key, []);
    seen.get(key)!.push(rule.source!.start!.line);
  });
  const dupes = Array.from(seen)
    .filter(([, lines]) => lines.length > 1)
    .map(([key, lines]) => `${key} — lines ${lines.join(", ")}`);
  expect(dupes, `selectors defined more than once:\n${dupes.join("\n")}`).toEqual([]);
});

test("every exercised detail field gets external spacing from its direct parent", async ({
  page,
  request,
}) => {
  const viewDefinitionId = await createResource(request, "ViewDefinition", {
    name: `spacing_guard_${Date.now()}`,
    status: "active",
    resource: "Patient",
    select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
  });
  await waitSearchable(request, "ViewDefinition", viewDefinitionId);

  await page.goto("/ui/capability-statement", { waitUntil: "networkidle" });
  await expectDetailFieldSpacing(page, "Capability Statement summary");

  await page.goto("/ui/sql/export", { waitUntil: "networkidle" });
  await expectDetailFieldSpacing(page, "SQL Export form");

  await page.goto("/ui/sql/files", { waitUntil: "networkidle" });
  await expectDetailFieldSpacing(page, "SQL Files form");

  await page.goto("/ui/search-parameters", { waitUntil: "networkidle" });
  await page.locator("a.row-link").first().click();
  await expect(page.locator(".detail .detail__field").first()).toBeVisible();
  await expectDetailFieldSpacing(page, "Search Parameters detail");

  await page.goto("/ui/compartments", { waitUntil: "networkidle" });
  await expectDetailFieldSpacing(page, "Compartments definition");
  await page.locator(".tabs .tab").last().click();
  const tester = page.locator("form.tester");
  await expect(tester).toBeVisible();
  await expect(tester.locator(":scope > .detail__field").first()).toBeVisible();
  await expectDetailFieldSpacing(page, "Compartments tester");

  await page.goto(await seedBulkImportDetail(request), { waitUntil: "networkidle" });
  await expectDetailFieldSpacing(page, "Bulk Import summary");
});

for (const viewport of [
  { width: 1440, height: 900, columns: 2 },
  { width: 1240, height: 800, columns: 1 },
  { width: 390, height: 844, columns: 1 },
] as const) {
  test(`Capability Summary follows the shared metadata grid at ${viewport.width}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width: viewport.width, height: viewport.height });
    await page.goto("/ui/capability-statement", { waitUntil: "networkidle" });

    const summary = page.locator("section.card").first();
    const body = summary.locator(":scope > .card__body");
    const grid = body.locator(":scope > .kv-grid");
    const fields = grid.locator(":scope > .detail__field");
    await expect(fields).toHaveCount(7);

    // A deliberately hostile implementation URL proves the shared field/grid
    // contract wraps unbroken values instead of widening the document.
    await fields
      .nth(1)
      .locator(":scope > div")
      .evaluate((element) => {
        element.textContent = `https://example.test/${"deeply-nested-segment/".repeat(24)}metadata`;
      });

    const metrics = await summary.evaluate((element) => {
      const body = element.querySelector<HTMLElement>(":scope > .card__body")!;
      const grid = body.querySelector<HTMLElement>(":scope > .kv-grid")!;
      const fields = Array.from(grid.querySelectorAll<HTMLElement>(":scope > .detail__field"));
      const bodyBox = body.getBoundingClientRect();
      const gridBox = grid.getBoundingClientRect();
      const boxes = fields.map((field) => field.getBoundingClientRect().toJSON());
      const gridStyle = getComputedStyle(grid);
      return {
        columns: gridStyle.gridTemplateColumns.split(/\s+/).filter(Boolean).length,
        rowGap: gridStyle.rowGap,
        columnGap: gridStyle.columnGap,
        marginBottom: gridStyle.marginBottom,
        fieldGap: getComputedStyle(fields[0]).rowGap,
        bottomInset: bodyBox.bottom - gridBox.bottom,
        gridWidth: gridBox.width,
        boxes,
        documentOverflows:
          document.documentElement.scrollWidth > document.documentElement.clientWidth,
      };
    });

    expect(metrics.columns).toBe(viewport.columns);
    expect(metrics.rowGap).toBe("14px");
    expect(metrics.columnGap).toBe("18px");
    expect(metrics.marginBottom).toBe("0px");
    expect(metrics.fieldGap).toBe("5px");
    expect(Math.abs(metrics.bottomInset - 14)).toBeLessThanOrEqual(1);
    expect(metrics.documentOverflows).toBe(false);
    expect(Math.abs(metrics.boxes[0].width - metrics.gridWidth)).toBeLessThanOrEqual(1);
    expect(Math.abs(metrics.boxes[1].width - metrics.gridWidth)).toBeLessThanOrEqual(1);

    if (viewport.columns === 2) {
      expect(Math.abs(metrics.boxes[2].y - metrics.boxes[3].y)).toBeLessThanOrEqual(1);
      expect(metrics.boxes[3].x).toBeGreaterThan(metrics.boxes[2].x);
    } else {
      for (const box of metrics.boxes) {
        expect(Math.abs(box.width - metrics.gridWidth)).toBeLessThanOrEqual(1);
      }
    }
  });
}
