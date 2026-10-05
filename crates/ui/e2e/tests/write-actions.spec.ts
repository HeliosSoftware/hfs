// #1750: every POST form and htmx write button goes busy on submit and drops
// repeat clicks (busy.js). Each test delays the write with `page.route` so the
// "in flight" window is wide, counts the requests that reach the network, and
// fires double clicks inside ONE `page.evaluate` so the submit guard is
// exercised and not only the `disabled` attribute.
import type { Locator, Page } from "@playwright/test";
import { acceptConfirm, dismissConfirm, expect, test } from "../pages/fixtures";
import { createResource, createSqlQueryLibrary, deleteByNamePrefix, waitSearchable } from "../pages/api";
import { VdEditor } from "../pages/vd-editor";

const DELAY_MS = 800;
const LIBRARY_TYPES = "http://hl7.org/fhir/uv/sql-on-fhir/CodeSystem/LibraryTypesCodes";

type Seen = { bodies: string[] };

/** Delay every POST to `pathname`; count them. `fulfill204` answers with
 * "No Content" (the page stays put, so the form stays marked) instead of
 * letting the write through. */
async function slowPost(page: Page, pathname: string, opts: { fulfill204?: boolean } = {}): Promise<Seen> {
  const seen: Seen = { bodies: [] };
  await page.route(
    (url) => url.pathname === pathname,
    async (route) => {
      const request = route.request();
      if (request.method() !== "POST") return route.continue();
      seen.bodies.push(request.postData() ?? "");
      await new Promise((resolve) => setTimeout(resolve, DELAY_MS));
      if (opts.fulfill204) return route.fulfill({ status: 204 });
      return route.continue();
    },
  );
  return seen;
}

/** Two clicks in one task, then the button's state once the tick that
 * disables it has run. Read inside the page: once the POST navigates, a
 * locator assertion would wait for the new document and never see it. */
async function doubleClick(locator: Locator): Promise<{ busy: string | null; disabled: boolean }> {
  return locator.evaluate(async (el) => {
    (el as HTMLElement).click();
    (el as HTMLElement).click();
    await new Promise((resolve) => setTimeout(resolve, 100));
    const button = el as HTMLButtonElement;
    return { busy: button.getAttribute("aria-busy"), disabled: button.disabled };
  });
}

async function count(request: import("@playwright/test").APIRequestContext, type: string, name: string): Promise<number> {
  const res = await request.get(`/${type}?name=${encodeURIComponent(name)}&_summary=count`);
  return ((await res.json()).total as number) ?? 0;
}

function stamp(prefix: string): string {
  return `${prefix}_${Date.now().toString(36)}`;
}

async function seedVd(request: import("@playwright/test").APIRequestContext, name: string): Promise<string> {
  const id = await createResource(request, "ViewDefinition", {
    name, url: `http://example.org/ViewDefinition/${name}`, status: "active", resource: "Patient",
    select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
  });
  await waitSearchable(request, "ViewDefinition", id);
  return id;
}

async function seedLibrary(
  request: import("@playwright/test").APIRequestContext, name: string, code: "sql-query" | "sql-view",
): Promise<string> {
  if (code === "sql-query") {
    const id = await createSqlQueryLibrary(request, name, `http://example.org/ViewDefinition/${name}_vd`);
    await waitSearchable(request, "Library", id);
    return id;
  }
  const id = await createResource(request, "Library", {
    name, status: "active", url: `http://example.org/Library/${name}`,
    type: { coding: [{ system: LIBRARY_TYPES, code }] },
    content: [{ contentType: "application/sql", data: Buffer.from("SELECT 1 AS n").toString("base64") }],
  });
  await waitSearchable(request, "Library", id);
  return id;
}

const vdSave = (page: Page) => page.locator("#vd-editor-form button[name='action'][value='save']");
const vdDuplicate = (page: Page) => page.locator("button[name='action'][value='duplicate']");
const libSave = (page: Page) => page.locator("#lib-editor-form button[name='action'][value='save']");

test("View Definitions: a double-click on Save sends one POST and the button is busy meanwhile", async ({ page, request }) => {
  const name = stamp("e2e_wa_vd_save");
  try {
    const id = await seedVd(request, name);
    await page.goto(`/ui/sql/view-definitions?vd=${id}`);
    const seen = await slowPost(page, "/ui/sql/view-definitions");
    expect(await doubleClick(vdSave(page))).toEqual({ busy: "true", disabled: true });
    await page.waitForURL(/saved=1/);
    expect(seen.bodies).toHaveLength(1);
  } finally {
    await deleteByNamePrefix(request, "ViewDefinition", name);
  }
});

test("View Definitions: a double-click on Duplicate sends one POST with action=duplicate and makes one copy", async ({ page, request }) => {
  const name = stamp("e2e_wa_vd_dup");
  try {
    const id = await seedVd(request, name);
    await page.goto(`/ui/sql/view-definitions?vd=${id}`);
    const seen = await slowPost(page, "/ui/sql/view-definitions");
    expect(await doubleClick(vdDuplicate(page))).toEqual({ busy: "true", disabled: true });
    await page.waitForURL(/saved=1/);
    expect(seen.bodies).toHaveLength(1);
    expect(seen.bodies[0]).toContain("action=duplicate");
    await expect.poll(() => count(request, "ViewDefinition", name)).toBe(2);
  } finally {
    await deleteByNamePrefix(request, "ViewDefinition", name);
  }
});

test("SQL Queries: Save and Duplicate double-clicks each send one POST; Duplicate makes one copy", async ({ page, request }) => {
  const name = stamp("e2e_wa_q");
  try {
    const id = await seedLibrary(request, name, "sql-query");
    await page.goto(`/ui/sql/queries?lib=${id}`);
    const seen = await slowPost(page, "/ui/sql/queries");

    expect(await doubleClick(libSave(page))).toEqual({ busy: "true", disabled: true });
    await page.waitForURL(/saved=1/);
    expect(seen.bodies).toHaveLength(1);
    expect(seen.bodies[0]).toContain("action=save");

    seen.bodies.length = 0;
    await page.goto(`/ui/sql/queries?lib=${id}`);
    expect(await doubleClick(vdDuplicate(page))).toEqual({ busy: "true", disabled: true });
    await page.waitForURL((url) => url.searchParams.get("saved") === "1" && url.searchParams.get("lib") !== id);
    expect(seen.bodies).toHaveLength(1);
    expect(seen.bodies[0]).toContain("action=duplicate");
    await expect.poll(() => count(request, "Library", name)).toBe(2);
  } finally {
    await deleteByNamePrefix(request, "Library", name);
  }
});

test("SQL Views: a double-click on Save sends one POST", async ({ page, request }) => {
  const name = stamp("e2e_wa_v");
  try {
    const id = await seedLibrary(request, name, "sql-view");
    await page.goto(`/ui/sql/views?lib=${id}`);
    const seen = await slowPost(page, "/ui/sql/views");
    expect(await doubleClick(libSave(page))).toEqual({ busy: "true", disabled: true });
    await page.waitForURL(/saved=1/);
    expect(seen.bodies).toHaveLength(1);
  } finally {
    await deleteByNamePrefix(request, "Library", name);
  }
});

test("a forced click while the write is in flight sends nothing more", async ({ page, request }) => {
  const name = stamp("e2e_wa_force");
  try {
    const id = await seedVd(request, name);
    await page.goto(`/ui/sql/view-definitions?vd=${id}`);
    const seen = await slowPost(page, "/ui/sql/view-definitions", { fulfill204: true });
    await vdSave(page).click();
    await expect(vdSave(page)).toHaveAttribute("aria-busy", "true");
    await vdSave(page).click({ force: true });
    await vdSave(page).evaluate((el) => (el as HTMLElement).click());
    await page.waitForTimeout(DELAY_MS + 300);
    expect(seen.bodies).toHaveLength(1);
  } finally {
    await deleteByNamePrefix(request, "ViewDefinition", name);
  }
});

test("a submit cancelled by another script marks nothing; accepting the confirmation sends one POST", async ({ page, request }) => {
  const name = stamp("e2e_wa_lint");
  try {
    await page.goto("/ui/sql/view-definitions?vd=new");
    const ed = new VdEditor(page);
    await ed.setDoc(`{
  "resourceType": "ViewDefinition",
  "name": "${name}",
  "status": "active",
  "resource": "Patient",
  "select": [{ "column": [{ "name": "id", "path": "getResourceKey()" }], "columns": [] }]
}`);
    await expect(page.locator(".cm-lintRange-error")).toHaveCount(1);
    const seen = await slowPost(page, "/ui/sql/view-definitions");

    await vdSave(page).click();
    await dismissConfirm(page);
    await expect(vdSave(page)).not.toHaveAttribute("aria-busy", "true");
    await expect(vdSave(page)).toBeEnabled();
    expect(seen.bodies).toHaveLength(0);

    await vdSave(page).click();
    await acceptConfirm(page);
    await page.waitForURL(/saved=1/);
    expect(seen.bodies).toHaveLength(1);
  } finally {
    await deleteByNamePrefix(request, "ViewDefinition", name);
  }
});

test("an htmx write button (Add parameter) is busy and a double-click sends one request", async ({ page, request }) => {
  const name = stamp("e2e_wa_param");
  try {
    const id = await seedLibrary(request, name, "sql-query");
    await page.goto(`/ui/sql/queries?lib=${id}`);
    const seen = await slowPost(page, "/ui/sql/queries/document");
    await page.locator("#lib-params summary.editor-add__toggle").click();
    await page.locator("input[name='param_name']").fill("ward");
    const add = page.locator("button[name='op'][value='add-parameter']");
    expect(await doubleClick(add)).toEqual({ busy: "true", disabled: true });
    await page.waitForTimeout(DELAY_MS + 300);
    expect(seen.bodies).toHaveLength(1);
  } finally {
    await deleteByNamePrefix(request, "Library", name);
  }
});

test("pageshow with persisted restores a marked form", async ({ page, request }) => {
  const name = stamp("e2e_wa_pageshow");
  try {
    const id = await seedVd(request, name);
    await page.goto(`/ui/sql/view-definitions?vd=${id}`);
    await slowPost(page, "/ui/sql/view-definitions", { fulfill204: true });
    await vdSave(page).click();
    await expect(vdSave(page)).toHaveAttribute("aria-busy", "true");
    await expect(vdSave(page)).toBeDisabled();
    await expect(vdDuplicate(page)).toBeDisabled();

    await page.evaluate(() => window.dispatchEvent(new PageTransitionEvent("pageshow", { persisted: true })));
    await expect(vdSave(page)).not.toHaveAttribute("aria-busy", "true");
    await expect(vdSave(page)).toBeEnabled();
    await expect(vdDuplicate(page)).toBeEnabled();
  } finally {
    await deleteByNamePrefix(request, "ViewDefinition", name);
  }
});

test("a GET form (the rail search) is not marked busy and is not guarded", async ({ page }) => {
  await page.goto("/ui/sql/view-definitions");
  const seen: string[] = [];
  await page.route(
    (url) => url.pathname === "/ui/sql/view-definitions" && url.searchParams.has("filter"),
    async (route) => {
      seen.push(route.request().url());
      await new Promise((resolve) => setTimeout(resolve, DELAY_MS));
      await route.continue();
    },
  );
  const result = await page.evaluate(() => {
    let submits = 0;
    window.addEventListener("submit", () => submits++);
    const form = document.querySelector(".filter-rail__search") as HTMLFormElement;
    (form.querySelector("input[type='search']") as HTMLInputElement).value = "zzz";
    form.requestSubmit();
    form.requestSubmit();
    return { submits, busy: document.querySelectorAll('[aria-busy="true"]').length };
  });
  expect(result).toEqual({ submits: 2, busy: 0 });
});

test("the capture guard drops a second submit of a form in flight", async ({ page, request }) => {
  const name = stamp("e2e_wa_guard");
  try {
    const id = await seedVd(request, name);
    await page.goto(`/ui/sql/view-definitions?vd=${id}`);
    const seen = await slowPost(page, "/ui/sql/view-definitions");
    const submits = await page.evaluate(() => {
      let count = 0;
      window.addEventListener("submit", () => count++);
      const form = document.querySelector("#vd-editor-form") as HTMLFormElement;
      const save = form.querySelector("button[name='action'][value='save']") as HTMLButtonElement;
      form.requestSubmit(save);
      form.requestSubmit(save);
      return count;
    });
    expect(submits).toBe(1);
    await page.waitForURL(/saved=1/);
    expect(seen.bodies).toHaveLength(1);
  } finally {
    await deleteByNamePrefix(request, "ViewDefinition", name);
  }
});
