import { test, expect } from "../pages/fixtures";
import type { APIRequestContext, Page } from "@playwright/test";
import { ROUTES, seedBulkImportDetail } from "../pages/routes";
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
