// #1750: every POST form and htmx write button goes busy on submit and drops
// repeat clicks (busy.js). Each test delays the write with `page.route` so the
// "in flight" window is wide, counts the requests that reach the network, and
// fires double clicks inside ONE `page.evaluate` so the submit guard is
// exercised and not only the `disabled` attribute.
import type { Locator, Page } from "@playwright/test";
import { acceptConfirm, confirmDialog, dismissConfirm, expect, test } from "../pages/fixtures";
import { createResource, createSqlQueryLibrary, deleteByNamePrefix, readResource, waitSearchable } from "../pages/api";
import { Editor } from "../pages/editor";
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

// ---------------------------------------------------------------------------
// Resource editor and Resources modal (#1750): Save/Delete show the busy ring
// and ignore repeat clicks while their request is in flight.
// ---------------------------------------------------------------------------
test.describe("resource editor and modal writes", () => {
  type WriteSeen = { methods: string[] };

  /** Delay every non-GET request under `/Patient` (or `type`) and count them.
   * `fulfill` answers with a canned response instead of reaching the server. */
  async function slowWrite(
    page: Page,
    opts: { type?: string; fulfill?: { status: number; body: unknown } } = {},
  ): Promise<WriteSeen> {
    const type = opts.type ?? "Patient";
    const seen: WriteSeen = { methods: [] };
    await page.route(
      (url) => url.pathname === `/${type}` || url.pathname.startsWith(`/${type}/`),
      async (route) => {
        const method = route.request().method();
        if (method === "GET") return route.continue();
        seen.methods.push(method);
        await new Promise((resolve) => setTimeout(resolve, DELAY_MS));
        if (opts.fulfill) {
          return route.fulfill({
            status: opts.fulfill.status,
            contentType: "application/fhir+json",
            body: JSON.stringify(opts.fulfill.body),
          });
        }
        return route.continue();
      },
    );
    return seen;
  }

  /** Patients carry `name` as an array, which `deleteByNamePrefix` cannot read. */
  async function deletePatients(request: import("@playwright/test").APIRequestContext, family: string) {
    const res = await request.get(`/Patient?family=${encodeURIComponent(family)}&_count=50`);
    const bundle = (await res.json()) as { entry?: { resource: { id: string } }[] };
    for (const entry of bundle.entry ?? []) await request.delete(`/Patient/${entry.resource.id}`);
  }

  const outcome = {
    resourceType: "OperationOutcome",
    issue: [{ severity: "error", code: "invalid", diagnostics: "simulated rejection" }],
  };

  async function patientCount(request: import("@playwright/test").APIRequestContext, family: string) {
    const res = await request.get(`/Patient?family=${encodeURIComponent(family)}&_summary=count`);
    return ((await res.json()).total as number) ?? 0;
  }

  async function seedPatient(request: import("@playwright/test").APIRequestContext, family: string) {
    const id = await createResource(request, "Patient", { name: [{ family }] });
    await waitSearchable(request, "Patient", id);
    return id;
  }

  async function openInModal(
    resources: import("../pages/resources").ResourcesPage,
    page: Page,
    id: string,
  ) {
    await resources.goto("Patient");
    await page.locator("input.query-builder__url[name=url]").fill(`Patient?_id=${id}`);
    await page.locator("[data-intent='run']").click();
    await resources.results.waitShown();
    await resources.results.rows.first().locator("td:last-child").click();
    await resources.modal.waitOpen();
    await expect(resources.modal.subject).toContainText(id);
    await expect(resources.modal.saveButton).toBeEnabled();
  }

  async function versionId(request: import("@playwright/test").APIRequestContext, id: string) {
    const body = (await readResource(request, "Patient", id)) as { meta?: { versionId?: string } };
    return Number(body.meta?.versionId);
  }

  test("modal create: a double-click on Save sends one POST and makes one resource", async ({ resources, page, request }) => {
    const family = stamp("E2eWaCreate");
    try {
      await resources.goto("Patient");
      await resources.openCreate("Patient");
      await resources.modal.editor.applyJson({ resourceType: "Patient", name: [{ family }] });
      await expect(resources.modal.editor.form).toHaveAttribute("data-error-count", "0");
      const seen = await slowWrite(page);
      const state = await resources.modal.saveButton.evaluate(async (el) => {
        (el as HTMLElement).click();
        (el as HTMLElement).click();
        await new Promise((resolve) => setTimeout(resolve, 100));
        return { busy: el.getAttribute("aria-busy"), disabled: (el as HTMLButtonElement).disabled };
      });
      expect(state).toEqual({ busy: "true", disabled: true });
      await expect(resources.modal.saveButton).not.toHaveAttribute("aria-busy", "true", { timeout: 10_000 });
      expect(seen.methods).toEqual(["POST"]);
      await expect.poll(() => patientCount(request, family)).toBe(1);
    } finally {
      await deletePatients(request, family);
    }
  });

  test("modal edit: a double-click on Save sends one PUT and adds exactly one version", async ({ resources, page, request }) => {
    const family = stamp("E2eWaEdit");
    try {
      const id = await seedPatient(request, family);
      const before = await versionId(request, id);
      await openInModal(resources, page, id);
      const seen = await slowWrite(page);
      await resources.modal.saveButton.evaluate(async (el) => {
        (el as HTMLElement).click();
        (el as HTMLElement).click();
      });
      await expect(resources.modal.saveButton).not.toHaveAttribute("aria-busy", "true", { timeout: 10_000 });
      expect(seen.methods).toEqual(["PUT"]);
      expect(await versionId(request, id)).toBe(before + 1);
    } finally {
      await deletePatients(request, family);
    }
  });

  test("modal: Save is busy and Delete disabled while in flight, both restored afterwards", async ({ resources, page, request }) => {
    const family = stamp("E2eWaState");
    try {
      const id = await seedPatient(request, family);
      await openInModal(resources, page, id);
      await slowWrite(page);
      await resources.modal.saveButton.click();
      await expect(resources.modal.saveButton).toHaveAttribute("aria-busy", "true");
      await expect(resources.modal.saveButton).toBeDisabled();
      await expect(resources.modal.deleteButton).toBeDisabled();
      await expect(resources.modal.saveButton).not.toHaveAttribute("aria-busy", "true", { timeout: 10_000 });
      await expect(resources.modal.saveButton).toBeEnabled();
      await expect(resources.modal.deleteButton).toBeEnabled();
    } finally {
      await deletePatients(request, family);
    }
  });

  test("modal: a rejected save re-enables Save, shows the error and a new click sends again", async ({ resources, page, request }) => {
    const family = stamp("E2eWaReject");
    try {
      const id = await seedPatient(request, family);
      await openInModal(resources, page, id);
      const seen = await slowWrite(page, { fulfill: { status: 422, body: outcome } });
      await resources.modal.saveButton.click();
      await expect(resources.modal.saveButton).toHaveAttribute("aria-busy", "true");
      await expect(resources.modal.saveButton).not.toHaveAttribute("aria-busy", "true", { timeout: 10_000 });
      await expect(resources.modal.saveButton).toBeEnabled();
      await expect(resources.modal.status).toContainText("simulated rejection");
      await resources.modal.saveButton.click();
      await expect(resources.modal.saveButton).not.toHaveAttribute("aria-busy", "true", { timeout: 10_000 });
      expect(seen.methods).toEqual(["PUT", "PUT"]);
    } finally {
      await deletePatients(request, family);
    }
  });

  test("modal: a blocked validation sends nothing and leaves Save enabled", async ({ resources, page }) => {
    await resources.goto("Observation");
    await resources.openCreate("Observation");
    await expect(resources.modal.editor.form).toHaveAttribute("data-error-count", /[1-9]/);
    const seen = await slowWrite(page, { type: "Observation" });
    await resources.modal.saveButton.click();
    await expect(resources.modal.status).not.toBeEmpty();
    await expect(resources.modal.saveButton).not.toHaveAttribute("aria-busy", "true");
    await expect(resources.modal.saveButton).toBeEnabled();
    await page.waitForTimeout(DELAY_MS + 200);
    expect(seen.methods).toEqual([]);
  });

  test("modal delete: Delete is busy and Save disabled while the DELETE is in flight; one DELETE", async ({ resources, page, request }) => {
    const family = stamp("E2eWaModalDel");
    try {
      const id = await seedPatient(request, family);
      await openInModal(resources, page, id);
      const seen = await slowWrite(page);
      await resources.modal.deleteButton.click();
      await acceptConfirm(page);
      await expect(resources.modal.deleteButton).toHaveAttribute("aria-busy", "true");
      await expect(resources.modal.saveButton).toBeDisabled();
      await resources.modal.deleteButton.evaluate((el) => {
        (el as HTMLElement).click();
        (el as HTMLElement).click();
      });
      await expect(resources.modal.root).toBeHidden({ timeout: 10_000 });
      expect(seen.methods).toEqual(["DELETE"]);
    } finally {
      await deletePatients(request, family);
    }
  });

  test("full-page editor: Save is busy while saving and a double-click sends one request", async ({ page, request }) => {
    const family = stamp("E2eWaPageSave");
    try {
      const id = await seedPatient(request, family);
      await page.goto(`/ui/editor?type=Patient&id=${id}`, { waitUntil: "networkidle" });
      const save = page.locator("#editor-save");
      await expect(save).toBeEnabled();
      const seen = await slowWrite(page);
      await save.evaluate(async (el) => {
        (el as HTMLElement).click();
        (el as HTMLElement).click();
      });
      await expect(save).toHaveAttribute("aria-busy", "true");
      await expect(save).not.toHaveAttribute("aria-busy", "true", { timeout: 10_000 });
      expect(seen.methods).toHaveLength(1);
    } finally {
      await deletePatients(request, family);
    }
  });

  test("full-page editor: Delete is busy and Save disabled in flight; a failed DELETE restores both", async ({ page, request }) => {
    const family = stamp("E2eWaPageDel");
    try {
      const id = await seedPatient(request, family);
      await page.goto(`/ui/editor?type=Patient&id=${id}`, { waitUntil: "networkidle" });
      const save = page.locator("#editor-save");
      const del = page.locator("#editor-delete");
      await expect(del).toBeVisible();
      await expect(save).toBeEnabled();
      const seen = await slowWrite(page, { fulfill: { status: 500, body: outcome } });
      await del.click();
      await acceptConfirm(page);
      await expect(del).toHaveAttribute("aria-busy", "true");
      await expect(save).toBeDisabled();
      await expect(del).toBeDisabled();
      await expect(del).not.toHaveAttribute("aria-busy", "true", { timeout: 10_000 });
      await expect(del).toBeEnabled();
      await expect(save).toBeEnabled();
      expect(seen.methods).toEqual(["DELETE"]);
    } finally {
      await deletePatients(request, family);
    }
  });
});

test.describe("create save-target rule (#1751)", () => {
  const NOTICE = (type: string, id: string) => `Will be saved as ${type}/${id}`;

  /** Record the methods + paths of every non-GET write to `/Patient...`. */
  function recordWrites(page: Page): string[] {
    const seen: string[] = [];
    page.on("request", (req) => {
      const path = new URL(req.url()).pathname;
      if (req.method() !== "GET" && (path === "/Patient" || path.startsWith("/Patient/"))) {
        seen.push(`${req.method()} ${path}`);
      }
    });
    return seen;
  }

  async function deletePatients(request: import("@playwright/test").APIRequestContext, family: string) {
    const res = await request.get(`/Patient?family=${encodeURIComponent(family)}&_count=50`);
    const bundle = (await res.json()) as { entry?: { resource: { id: string } }[] };
    for (const entry of bundle.entry ?? []) await request.delete(`/Patient/${entry.resource.id}`);
  }

  test("modal create with a pasted id sends PUT to that id and the notice follows the document", async ({ resources, page, request }) => {
    const family = stamp("E2eStCreate");
    const id = stamp("st-id").replace(/_/g, "-");
    try {
      await resources.goto("Patient");
      await resources.openCreate("Patient");
      const subject = resources.modal.subject;
      await expect(subject).toHaveText("Patient · new");
      await expect(subject).not.toHaveClass(/subject--target/);

      await resources.modal.editor.fillRaw({ resourceType: "Patient", id, name: [{ family }] });
      await expect(subject).toHaveText(NOTICE("Patient", id));
      await expect(subject).toHaveClass(/subject--target/);
      await expect(subject.locator("code")).toHaveText(`Patient/${id}`);

      // The notice follows the raw source: another id, no id, an invalid id, broken JSON.
      await resources.modal.editor.setJson(JSON.stringify({ resourceType: "Patient", id: "other-id" }));
      await expect(subject).toHaveText(NOTICE("Patient", "other-id"));
      await resources.modal.editor.setJson(JSON.stringify({ resourceType: "Patient" }));
      await expect(subject).toHaveText("Patient · new");
      await expect(subject).not.toHaveClass(/subject--target/);
      await resources.modal.editor.setJson(JSON.stringify({ resourceType: "Patient", id: "a b" }));
      await expect(subject).toHaveText("Patient · new");
      await resources.modal.editor.setJson("{ not json");
      await expect(subject).toHaveText("Patient · new");
      await expect(subject).not.toHaveClass(/subject--target/);

      await resources.modal.editor.setJson(JSON.stringify({ resourceType: "Patient", id, name: [{ family }] }));
      await expect(subject).toHaveText(NOTICE("Patient", id));
      const seen = recordWrites(page);
      await resources.modal.saveButton.click();
      await expect(subject).toHaveText(`Patient/${id}`);
      await expect(subject).not.toHaveClass(/subject--target/);
      expect(seen).toEqual([`PUT /Patient/${id}`]);
      const read = await request.get(`/Patient/${id}`);
      expect(read.status()).toBe(200);
    } finally {
      await deletePatients(request, family);
    }
  });

  test("modal create without an id sends POST and the server assigns one", async ({ resources, page, request }) => {
    const family = stamp("E2eStPost");
    try {
      await resources.goto("Patient");
      await resources.openCreate("Patient");
      await resources.modal.editor.applyJson({ resourceType: "Patient", name: [{ family }] });
      const seen = recordWrites(page);
      await resources.modal.saveButton.click();
      await expect(resources.modal.subject).toHaveText(/^Patient\/[A-Za-z0-9.-]+$/);
      expect(seen).toEqual(["POST /Patient"]);
      await expect.poll(async () => {
        const res = await request.get(`/Patient?family=${encodeURIComponent(family)}&_summary=count`);
        return ((await res.json()).total as number) ?? 0;
      }).toBe(1);
    } finally {
      await deletePatients(request, family);
    }
  });

  test("opening an existing resource never shows the notice (modal and editor)", async ({ resources, page, request }) => {
    const family = stamp("E2eStExisting");
    try {
      const id = await createResource(request, "Patient", { name: [{ family }] });
      await waitSearchable(request, "Patient", id);
      await resources.goto("Patient");
      await page.locator("input.query-builder__url[name=url]").fill(`Patient?_id=${id}`);
      await page.locator("[data-intent='run']").click();
      await resources.results.waitShown();
      await resources.results.rows.first().locator("td:last-child").click();
      await resources.modal.waitOpen();
      await expect(resources.modal.subject).toContainText(id);
      await expect(resources.modal.subject).not.toContainText("Will be saved as");
      await expect(resources.modal.subject).not.toHaveClass(/subject--target/);

      await page.goto(`/ui/editor?type=Patient&id=${id}`, { waitUntil: "networkidle" });
      const subject = page.locator("#editor-subject");
      await expect(subject).toContainText(`Patient/${id}`);
      await expect(subject).not.toContainText("Will be saved as");
      await expect(subject).not.toHaveClass(/subject--target/);
    } finally {
      await deletePatients(request, family);
    }
  });

  test("full-page editor in new mode shows the notice and saves with PUT to the pasted id", async ({ page, request }) => {
    const family = stamp("E2eStPage");
    const id = stamp("st-page").replace(/_/g, "-");
    try {
      await page.goto("/ui/editor?type=Patient", { waitUntil: "networkidle" });
      const editor = new Editor(page, page.locator("#editor"));
      const subject = page.locator("#editor-subject");
      await expect(subject).toHaveText("");
      await expect(subject).not.toHaveClass(/subject--target/);
      await editor.fillRaw({ resourceType: "Patient", id, name: [{ family }] });
      await expect(subject).toHaveText(NOTICE("Patient", id));
      await expect(subject).toHaveClass(/subject--target/);
      await editor.setJson(JSON.stringify({ resourceType: "Patient", id: "a b" }));
      await expect(subject).toHaveText("");
      await editor.setJson(JSON.stringify({ resourceType: "Patient", id, name: [{ family }] }));
      await expect(subject).toHaveText(NOTICE("Patient", id));
      const seen = recordWrites(page);
      await page.locator("#editor-save").click();
      await expect(subject).toContainText(`Patient/${id}`);
      await expect(subject).not.toHaveClass(/subject--target/);
      expect(seen).toEqual([`PUT /Patient/${id}`]);
      expect((await request.get(`/Patient/${id}`)).status()).toBe(200);
    } finally {
      await deletePatients(request, family);
    }
  });
});

test.describe("confirm before creating over an existing id (#1751)", () => {
  const MESSAGE = (id: string) => `Patient/${id} already exists. Saving will add a new version of it.`;

  type Api = import("@playwright/test").APIRequestContext;

  function newId(prefix: string): string {
    return stamp(prefix).replace(/_/g, "-").toLowerCase();
  }

  /** Create `Patient/<id>` with PUT; returns its versionId. */
  async function seed(request: Api, id: string, family: string): Promise<string> {
    const res = await request.put(`/Patient/${id}`, {
      headers: { "Content-Type": "application/fhir+json" },
      data: { resourceType: "Patient", id, name: [{ family }] },
    });
    expect(res.ok()).toBeTruthy();
    return versionOf(request, id);
  }

  async function versionOf(request: Api, id: string): Promise<string> {
    const doc = (await readResource(request, "Patient", id)) as { meta: { versionId: string } };
    return doc.meta.versionId;
  }

  /** Record writes and existence probes against `/Patient/<id>`. */
  function record(page: Page, id: string): { puts: number; probes: number } {
    const seen = { puts: 0, probes: 0 };
    page.on("request", (req) => {
      const url = new URL(req.url());
      if (url.pathname !== `/Patient/${id}`) return;
      if (req.method() === "PUT") seen.puts += 1;
      if (req.method() === "GET" && url.searchParams.get("_elements") === "id") seen.probes += 1;
    });
    return seen;
  }

  async function cleanup(request: Api, id: string) {
    await request.delete(`/Patient/${id}`);
  }

  async function openModalWith(resources: import("../pages/resources").ResourcesPage, id: string, family: string) {
    await resources.goto("Patient");
    await resources.openCreate("Patient");
    await resources.modal.editor.fillRaw({ resourceType: "Patient", id, name: [{ family }] });
  }

  test("modal: an existing id asks first and no PUT goes out yet", async ({ resources, page, request }) => {
    const id = newId("cf-ask");
    try {
      await seed(request, id, "Original");
      await openModalWith(resources, id, "Pasted");
      const seen = record(page, id);
      await resources.modal.saveButton.click();
      const dialog = confirmDialog(page);
      await expect(dialog.locator(".confirm-dialog__message")).toHaveText(MESSAGE(id));
      await expect(dialog.locator("[data-confirm-ok]")).toHaveText("Save new version");
      expect(seen.puts).toBe(0);
      await dismissConfirm(page);
    } finally {
      await cleanup(request, id);
    }
  });

  test("modal: cancelling writes nothing and leaves the modal intact", async ({ resources, page, request }) => {
    const id = newId("cf-cancel");
    try {
      const before = await seed(request, id, "Original");
      await openModalWith(resources, id, "Pasted");
      const seen = record(page, id);
      await resources.modal.saveButton.click();
      await dismissConfirm(page);
      expect(seen.puts).toBe(0);
      expect(await versionOf(request, id)).toBe(before);
      await expect(resources.modal.root).toBeVisible();
      expect(JSON.parse(await resources.modal.editor.source.inputValue()).name[0].family).toBe("Pasted");
      await expect(resources.modal.saveButton).toBeEnabled();
      await expect(resources.modal.saveButton).not.toHaveAttribute("aria-busy", "true");
    } finally {
      await cleanup(request, id);
    }
  });

  test("modal: accepting writes once and adds exactly one version", async ({ resources, page, request }) => {
    const id = newId("cf-accept");
    try {
      const before = await seed(request, id, "Original");
      await openModalWith(resources, id, "Pasted");
      const seen = record(page, id);
      await resources.modal.saveButton.click();
      await acceptConfirm(page, MESSAGE(id));
      await expect.poll(() => seen.puts).toBe(1);
      await expect(resources.modal.subject).toHaveText(`Patient/${id}`);
      expect(Number(await versionOf(request, id))).toBe(Number(before) + 1);
    } finally {
      await cleanup(request, id);
    }
  });

  test("modal: an id that does not exist saves without a dialog", async ({ resources, page, request }) => {
    const id = newId("cf-new");
    try {
      await openModalWith(resources, id, "Fresh");
      const seen = record(page, id);
      await resources.modal.saveButton.click();
      await expect(resources.modal.subject).toHaveText(`Patient/${id}`);
      await expect(confirmDialog(page)).toHaveCount(0);
      expect(seen.puts).toBe(1);
      expect((await request.get(`/Patient/${id}`)).status()).toBe(200);
    } finally {
      await cleanup(request, id);
    }
  });

  for (const mode of ["500", "abort"] as const) {
    test(`modal: a failed check (${mode}) does not ask and the save continues`, async ({ resources, page, request }) => {
      const id = newId(`cf-fail-${mode}`);
      try {
        await openModalWith(resources, id, "Fresh");
        await page.route(
          (url) => url.pathname === `/Patient/${id}` && url.searchParams.has("_elements"),
          (route) => (mode === "500" ? route.fulfill({ status: 500, body: "{}" }) : route.abort()),
        );
        const seen = record(page, id);
        await resources.modal.saveButton.click();
        await expect(resources.modal.subject).toHaveText(`Patient/${id}`);
        await expect(confirmDialog(page)).toHaveCount(0);
        expect(seen.puts).toBe(1);
      } finally {
        await cleanup(request, id);
      }
    });
  }

  test("modal: a double click during the check makes one check and one dialog", async ({ resources, page, request }) => {
    const id = newId("cf-double");
    try {
      await seed(request, id, "Original");
      await openModalWith(resources, id, "Pasted");
      await page.route(
        (url) => url.pathname === `/Patient/${id}` && url.searchParams.has("_elements"),
        async (route) => {
          await new Promise((resolve) => setTimeout(resolve, DELAY_MS));
          await route.continue();
        },
      );
      const seen = record(page, id);
      await doubleClick(resources.modal.saveButton);
      await expect(confirmDialog(page)).toHaveCount(1);
      await page.waitForTimeout(300);
      expect(seen.probes).toBe(1);
      await expect(confirmDialog(page)).toHaveCount(1);
      await dismissConfirm(page);
      expect(seen.puts).toBe(0);
    } finally {
      await cleanup(request, id);
    }
  });

  test("editor page: new mode with an existing id asks; cancel writes nothing, accept writes once", async ({ page, request }) => {
    const id = newId("cf-page");
    try {
      const before = await seed(request, id, "Original");
      await page.goto("/ui/editor?type=Patient", { waitUntil: "networkidle" });
      const editor = new Editor(page, page.locator("#editor"));
      await editor.fillRaw({ resourceType: "Patient", id, name: [{ family: "Pasted" }] });
      const seen = record(page, id);
      await page.locator("#editor-save").click();
      await dismissConfirm(page, MESSAGE(id));
      expect(seen.puts).toBe(0);
      expect(await versionOf(request, id)).toBe(before);
      await expect(page.locator("#editor-save")).toBeEnabled();
      await expect(page.locator("#editor-save")).not.toHaveAttribute("aria-busy", "true");

      await page.locator("#editor-save").click();
      await acceptConfirm(page, MESSAGE(id));
      await expect.poll(() => seen.puts).toBe(1);
      await expect.poll(async () => Number(await versionOf(request, id))).toBe(Number(before) + 1);
    } finally {
      await cleanup(request, id);
    }
  });

  test("editor page: a double click during the check makes one check and one dialog", async ({ page, request }) => {
    const id = newId("cf-page-dbl");
    try {
      await seed(request, id, "Original");
      await page.goto("/ui/editor?type=Patient", { waitUntil: "networkidle" });
      const editor = new Editor(page, page.locator("#editor"));
      await editor.fillRaw({ resourceType: "Patient", id, name: [{ family: "Pasted" }] });
      await page.route(
        (url) => url.pathname === `/Patient/${id}` && url.searchParams.has("_elements"),
        async (route) => {
          await new Promise((resolve) => setTimeout(resolve, DELAY_MS));
          await route.continue();
        },
      );
      const seen = record(page, id);
      await doubleClick(page.locator("#editor-save"));
      await expect(confirmDialog(page)).toHaveCount(1);
      await page.waitForTimeout(300);
      expect(seen.probes).toBe(1);
      await dismissConfirm(page);
      expect(seen.puts).toBe(0);
    } finally {
      await cleanup(request, id);
    }
  });

  test("saving an already open resource never probes or asks (modal and editor)", async ({ resources, page, request }) => {
    const id = newId("cf-open");
    try {
      await seed(request, id, "Original");
      const seen = record(page, id);

      await resources.goto("Patient");
      await page.locator("input.query-builder__url[name=url]").fill(`Patient?_id=${id}`);
      await page.locator("[data-intent='run']").click();
      await resources.results.waitShown();
      await resources.results.rows.first().locator("td:last-child").click();
      await resources.modal.waitOpen();
      await resources.modal.editor.applyJson({ resourceType: "Patient", id, name: [{ family: "Edited" }] });
      await resources.modal.saveButton.click();
      await expect.poll(() => seen.puts).toBe(1);
      await expect(resources.modal.saveButton).toBeEnabled();
      await expect(confirmDialog(page)).toHaveCount(0);

      await page.goto(`/ui/editor?type=Patient&id=${id}`, { waitUntil: "networkidle" });
      const editor = new Editor(page, page.locator("#editor"));
      await editor.applyJson({ resourceType: "Patient", id, name: [{ family: "EditedAgain" }] });
      await page.locator("#editor-save").click();
      await expect.poll(() => seen.puts).toBe(2);
      await expect(page.locator("#editor-save")).toBeEnabled();
      await expect(confirmDialog(page)).toHaveCount(0);
      expect(seen.probes).toBe(0);
    } finally {
      await cleanup(request, id);
    }
  });
});

/**
 * The SQL live preview's "Running query…" line (#1750). `/run` is held by a
 * gate the test releases, so every state is asserted while the request is
 * provably in flight rather than by racing a timer.
 */
test.describe("SQL live preview running indicator (#1750)", () => {
  type Gate = { seen: () => number; release: (count?: number) => void; releaseAll: () => void };

  /** Hold each POST to `pathname` until released (oldest first), then let it through. */
  async function gateRun(page: Page, pathname: string, mode: "continue" | "abort" = "continue"): Promise<Gate> {
    const waiting: Array<() => void> = [];
    let seen = 0;
    await page.route(
      (url) => url.pathname === pathname,
      async (route) => {
        if (route.request().method() !== "POST") return route.continue();
        seen += 1;
        await new Promise<void>((resolve) => waiting.push(resolve));
        if (mode === "abort") return route.abort("failed");
        return route.continue().catch(() => undefined);
      },
    );
    return {
      seen: () => seen,
      release: (count = 1) => {
        for (let i = 0; i < count; i += 1) waiting.shift()?.();
      },
      releaseAll: () => {
        while (waiting.length) waiting.shift()?.();
      },
    };
  }

  /** Append a space to a textarea and fire the `input` the live preview listens to. */
  const edit = (textarea: Locator) =>
    textarea.evaluate((el) => {
      const field = el as HTMLTextAreaElement;
      field.value = `${field.value} `;
      field.dispatchEvent(new Event("input", { bubbles: true }));
    });

  const busy = (page: Page) => page.locator(".run-busy");
  const meta = (page: Page) => page.locator("#run-results-meta");
  const notice = (page: Page) => page.locator("#run-notice > *");

  async function expectRunning(page: Page) {
    await expect(busy(page)).toBeVisible();
    await expect(busy(page)).toContainText("Running query…");
    await expect(meta(page)).toBeHidden();
    await expect(notice(page)).toBeHidden();
  }

  async function expectSettled(page: Page) {
    await expect(busy(page)).toBeHidden();
    await expect(meta(page)).toContainText(/rows/);
  }

  test("SQL Queries: changing a parameter value shows the running line, hides the old texts, then repaints", async ({ page, request }) => {
    const name = stamp("e2e_wa_run_q");
    try {
      const vdName = `${name}_vd`;
      await seedVd(request, vdName);
      const id = await createSqlQueryLibrary(
        request, name, `http://example.org/ViewDefinition/${vdName}`,
        "SELECT id FROM v WHERE id = :p", [{ name: "p", use: "in", type: "string" }],
      );
      await waitSearchable(request, "Library", id);
      await page.goto(`/ui/sql/queries?lib=${id}`);
      await expect(page.locator("#run-notice .notice")).toBeVisible();
      const gate = await gateRun(page, "/ui/sql/queries/run");
      await page.locator("#lib-params input[name='param:p']").fill("x");
      await expect.poll(() => gate.seen()).toBeGreaterThanOrEqual(1);
      await expect(busy(page)).toBeVisible();
      await expect(busy(page)).toContainText("Running query…");
      await expect(notice(page)).toBeHidden();
      gate.releaseAll();
      await expect(busy(page)).toBeHidden();
      await expect(meta(page)).toContainText(/rows/);
      await expect(page.locator("#run-results .run-busy")).toHaveCount(1);
      await expect(busy(page)).toHaveCount(1);
    } finally {
      await deleteByNamePrefix(request, "Library", name);
      await deleteByNamePrefix(request, "ViewDefinition", `${name}_vd`);
    }
  });

  test("SQL Views: editing the SQL shows the running line, hides the old texts, then repaints", async ({ page, request }) => {
    const name = stamp("e2e_wa_run_v");
    try {
      const id = await seedLibrary(request, name, "sql-view");
      await page.goto(`/ui/sql/views?lib=${id}`);
      await expect(meta(page)).toContainText(/rows/);
      const gate = await gateRun(page, "/ui/sql/views/run");
      await edit(page.locator("textarea[name='sql']"));
      await expect.poll(() => gate.seen()).toBeGreaterThanOrEqual(1);
      await expectRunning(page);
      gate.releaseAll();
      await expectSettled(page);
      await expect(busy(page)).toHaveCount(1);
    } finally {
      await deleteByNamePrefix(request, "Library", name);
    }
  });

  test("View Definitions: editing the JSON shows the running line, hides the old texts, then repaints", async ({ page, request }) => {
    const name = stamp("e2e_wa_run_vd");
    try {
      const id = await seedVd(request, name);
      await page.goto(`/ui/sql/view-definitions?vd=${id}`);
      await expect(meta(page)).toContainText(/rows/);
      const gate = await gateRun(page, "/ui/sql/view-definitions/run");
      await edit(page.locator("textarea[name='json']"));
      await expect.poll(() => gate.seen()).toBeGreaterThanOrEqual(1);
      await expectRunning(page);
      gate.releaseAll();
      await expectSettled(page);
      await expect(busy(page)).toHaveCount(1);
    } finally {
      await deleteByNamePrefix(request, "ViewDefinition", name);
    }
  });

  test("the initial load request shows the running line while it runs", async ({ page, request }) => {
    const name = stamp("e2e_wa_run_load");
    try {
      const id = await seedVd(request, name);
      const gate = await gateRun(page, "/ui/sql/view-definitions/run");
      await page.goto(`/ui/sql/view-definitions?vd=${id}`);
      await expect.poll(() => gate.seen()).toBeGreaterThanOrEqual(1);
      await expect(busy(page)).toBeVisible();
      await expect(busy(page)).toContainText("Running query…");
      gate.releaseAll();
      await expectSettled(page);
    } finally {
      await deleteByNamePrefix(request, "ViewDefinition", name);
    }
  });

  test("an error response leaves the running line hidden and the error notice visible", async ({ page, request }) => {
    const name = stamp("e2e_wa_run_err");
    try {
      const id = await seedLibrary(request, name, "sql-view");
      await page.goto(`/ui/sql/views?lib=${id}`);
      await expect(meta(page)).toContainText(/rows/);
      const gate = await gateRun(page, "/ui/sql/views/run");
      await page.locator("textarea[name='json']").evaluate((el) => {
        const field = el as HTMLTextAreaElement;
        field.value = "{ not json";
        field.dispatchEvent(new Event("input", { bubbles: true }));
      });
      await expect.poll(() => gate.seen()).toBeGreaterThanOrEqual(1);
      await expect(busy(page)).toBeVisible();
      gate.releaseAll();
      await expect(busy(page)).toBeHidden();
      await expect(page.locator("#run-notice .notice--warn")).toBeVisible();
    } finally {
      await deleteByNamePrefix(request, "Library", name);
    }
  });

  test("a network error does not leave the running line stuck", async ({ page, request }) => {
    const name = stamp("e2e_wa_run_net");
    try {
      const id = await seedVd(request, name);
      await page.goto(`/ui/sql/view-definitions?vd=${id}`);
      await expect(meta(page)).toContainText(/rows/);
      const gate = await gateRun(page, "/ui/sql/view-definitions/run", "abort");
      await edit(page.locator("textarea[name='json']"));
      await expect.poll(() => gate.seen()).toBeGreaterThanOrEqual(1);
      await expect(busy(page)).toBeVisible();
      gate.releaseAll();
      await expect(busy(page)).toBeHidden();
    } finally {
      await deleteByNamePrefix(request, "ViewDefinition", name);
    }
  });

  test("View Definitions: a request that finishes while a newer one runs does not switch the line off", async ({ page, request }) => {
    const name = stamp("e2e_wa_run_two");
    try {
      const id = await seedVd(request, name);
      await page.goto(`/ui/sql/view-definitions?vd=${id}`);
      await expect(meta(page)).toContainText(/rows/);
      const gate = await gateRun(page, "/ui/sql/view-definitions/run");
      const json = page.locator("textarea[name='json']");
      await edit(json);
      await expect.poll(() => gate.seen()).toBeGreaterThanOrEqual(1);
      await edit(json);
      // The second request has no hx-sync, so htmx queues it behind the
      // first: releasing the first lets it finish, and only then does the
      // second reach the gate. Its arrival proves the first one's
      // afterRequest has already run.
      gate.release();
      await expect.poll(() => gate.seen()).toBeGreaterThanOrEqual(2);
      await expect(busy(page)).toBeVisible();
      gate.releaseAll();
      await expect(busy(page)).toBeHidden();
      await expect(meta(page)).toContainText(/rows/);
      await expect(busy(page)).toHaveCount(1);
    } finally {
      await deleteByNamePrefix(request, "ViewDefinition", name);
    }
  });

  test("SQL Views: a replaced request does not switch off the newer request's line", async ({ page, request }) => {
    const name = stamp("e2e_wa_run_rep");
    try {
      const id = await seedLibrary(request, name, "sql-view");
      await page.goto(`/ui/sql/views?lib=${id}`);
      await expect(meta(page)).toContainText(/rows/);
      const gate = await gateRun(page, "/ui/sql/views/run");
      const sql = page.locator("textarea[name='sql']");
      await edit(sql);
      await expect.poll(() => gate.seen()).toBeGreaterThanOrEqual(1);
      const first = gate.seen();
      await edit(sql);
      await expect.poll(() => gate.seen()).toBeGreaterThan(first);
      gate.release();
      // A negative has to be proven by letting the released request's
      // afterRequest handler run: wait two animation frames, then assert.
      await page.evaluate(
        () => new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))),
      );
      await expect(busy(page)).toBeVisible();
      gate.releaseAll();
      await expect(busy(page)).toBeHidden();
      await expect(meta(page)).toContainText(/rows/);
      await expect(busy(page)).toHaveCount(1);
    } finally {
      await deleteByNamePrefix(request, "Library", name);
    }
  });
});

// #1754: the SQL editor forms own single-line fields (parameter values, the
// Add parameter / Add table inputs), so Enter in one of them used to press the
// form's first submit button: Duplicate on a saved item. Enter must now do
// nothing from those fields, while clicking Save / Duplicate or pressing Enter
// on the focused button still submits.
test.describe("Enter in an editor form field does not submit the form", () => {
  /** Every POST the page emits except the live-preview runs and lint checks
   * (`…/run`, `…/lint`), which are not writes and fire on their own. Each entry is `<path> <action>`. */
  function recordPosts(page: Page): string[] {
    const posts: string[] = [];
    page.on("request", (request) => {
      const path = new URL(request.url()).pathname;
      if (request.method() === "POST" && !/\/(run|lint)$/.test(path)) {
        posts.push(`${path} ${new URLSearchParams(request.postData() ?? "").get("action") ?? ""}`);
      }
    });
    return posts;
  }

  async function seedQueryWithParam(request: import("@playwright/test").APIRequestContext, name: string): Promise<string> {
    const id = await createSqlQueryLibrary(
      request, name, `http://example.org/ViewDefinition/${name}_vd`, "SELECT :ward AS w FROM v",
      [{ name: "ward", use: "in", type: "string" }],
    );
    await waitSearchable(request, "Library", id);
    return id;
  }

  test("SQL Queries: Enter in a parameter value sends no POST, keeps the value, and the preview still runs", async ({ page, request }) => {
    const name = stamp("e2e_wa_enter_q");
    try {
      const id = await seedQueryWithParam(request, name);
      await page.goto(`/ui/sql/queries?lib=${id}`);
      const posts = recordPosts(page);
      const field = page.locator("input[name='param:ward']");
      const before = await count(request, "Library", name);
      const run = page.waitForRequest((r) => new URL(r.url()).pathname === "/ui/sql/queries/run");
      await field.fill("north");
      await field.press("Enter");
      await run;
      await page.waitForTimeout(500);
      expect(posts).toEqual([]);
      expect(page.url()).toContain(`lib=${id}`);
      expect(page.url()).not.toContain("saved=1");
      await expect(field).toHaveValue("north");
      expect(await count(request, "Library", name)).toBe(before);
    } finally {
      await deleteByNamePrefix(request, "Library", name);
    }
  });

  test("SQL Queries: Enter in the Add parameter name sends no POST and keeps the panel open", async ({ page, request }) => {
    const name = stamp("e2e_wa_enter_addp");
    try {
      const id = await seedQueryWithParam(request, name);
      await page.goto(`/ui/sql/queries?lib=${id}`);
      const posts = recordPosts(page);
      await page.locator("#lib-params summary.editor-add__toggle").click();
      const field = page.locator("input[name='param_name']");
      await field.fill("clinic");
      await field.press("Enter");
      await page.waitForTimeout(500);
      expect(posts).toEqual([]);
      await expect(field).toBeVisible();
      await expect(field).toHaveValue("clinic");
    } finally {
      await deleteByNamePrefix(request, "Library", name);
    }
  });

  test("SQL Views: Enter in a single-line field of the edit form sends no POST", async ({ page, request }) => {
    const name = stamp("e2e_wa_enter_v");
    try {
      const id = await seedLibrary(request, name, "sql-view");
      await page.goto(`/ui/sql/views?lib=${id}`);
      const posts = recordPosts(page);
      const before = await count(request, "Library", name);
      const field = page.locator("input[form='lib-editor-form']:not([type='hidden'])").first();
      await field.evaluate((el) => (el.closest("details") as HTMLDetailsElement | null)?.setAttribute("open", ""));
      await field.fill("x");
      await field.press("Enter");
      await page.waitForTimeout(500);
      expect(posts).toEqual([]);
      expect(await count(request, "Library", name)).toBe(before);
    } finally {
      await deleteByNamePrefix(request, "Library", name);
    }
  });

  test("View Definitions: Enter in a single-line field of the edit form sends no POST", async ({ page, request }) => {
    const name = stamp("e2e_wa_enter_vd");
    try {
      const id = await seedVd(request, name);
      await page.goto(`/ui/sql/view-definitions?vd=${id}`);
      const posts = recordPosts(page);
      const before = await count(request, "ViewDefinition", name);
      // The page has no one-line field of its own on this form today; add one
      // associated with it, as any future field would be.
      const field = page.locator("#wa-probe");
      await page.evaluate(() => {
        const input = document.createElement("input");
        input.type = "text";
        input.id = "wa-probe";
        input.setAttribute("form", "vd-editor-form");
        document.querySelector("#vd-editor-form")!.after(input);
      });
      await field.fill("x");
      await field.press("Enter");
      await page.waitForTimeout(500);
      expect(posts).toEqual([]);
      expect(await count(request, "ViewDefinition", name)).toBe(before);
    } finally {
      await deleteByNamePrefix(request, "ViewDefinition", name);
    }
  });

  test("Create New: Enter in a single-line field saves nothing", async ({ page, request }) => {
    await page.goto("/ui/sql/queries?lib=new");
    const posts = recordPosts(page);
    await page.locator("#lib-params summary.editor-add__toggle").click();
    const field = page.locator("input[name='param_name']");
    await field.fill("clinic");
    await field.press("Enter");
    await page.waitForTimeout(500);
    expect(posts).toEqual([]);
    expect(page.url()).toContain("lib=new");
  });

  test("clicking Save posts action=save; clicking Duplicate posts action=duplicate and makes one copy", async ({ page, request }) => {
    const name = stamp("e2e_wa_enter_click");
    try {
      const id = await seedQueryWithParam(request, name);
      await page.goto(`/ui/sql/queries?lib=${id}`);
      const posts = recordPosts(page);
      await libSave(page).click();
      await page.waitForURL(/saved=1/);
      expect(posts).toHaveLength(1);
      expect(posts[0]).toBe("/ui/sql/queries save");
      posts.length = 0;
      await page.goto(`/ui/sql/queries?lib=${id}`);
      await vdDuplicate(page).click();
      await page.waitForURL((url) => url.searchParams.get("saved") === "1" && url.searchParams.get("lib") !== id);
      expect(posts).toHaveLength(1);
      expect(posts[0]).toBe("/ui/sql/queries duplicate");
      await expect.poll(() => count(request, "Library", name)).toBe(2);
    } finally {
      await deleteByNamePrefix(request, "Library", name);
    }
  });

  test("Enter on the focused Save button submits with action=save", async ({ page, request }) => {
    const name = stamp("e2e_wa_enter_btn");
    try {
      const id = await seedQueryWithParam(request, name);
      await page.goto(`/ui/sql/queries?lib=${id}`);
      const posts = recordPosts(page);
      await libSave(page).focus();
      await page.keyboard.press("Enter");
      await page.waitForURL(/saved=1/);
      expect(posts).toHaveLength(1);
      expect(posts[0]).toBe("/ui/sql/queries save");
    } finally {
      await deleteByNamePrefix(request, "Library", name);
    }
  });
});
