import { test, expect, acceptConfirm, armDialog, dialogsSeen } from "../pages/fixtures";
import { Editor } from "../pages/editor";
import { createResource } from "../pages/api";

for (const mode of ["raw", "guided", "queued"] as const) {
  test(`standalone Cancel checks fresh ${mode} edits and prompts once per attempt`, async ({ page, request }) => {
    const id = await createResource(request, "Patient", { name: [{ family: "Original" }] });
    const origin = "/ui/resources?type=Patient#opening";
    await page.goto(`/ui/editor?type=Patient&id=${id}&return_to=${encodeURIComponent(origin)}`, { waitUntil: "networkidle" });
    const ed = new Editor(page, page.locator("#editor-body"));
    let release = () => {};
    if (mode === "raw") await ed.fillRaw({ ...await ed.currentDoc(), active: true });
    else {
      if (mode === "queued") {
        const hold = new Promise<void>(resolve => { release = resolve; });
        await page.route("**/ui/editor/render", async route => {
          if (new URLSearchParams(route.request().postData() ?? "").get("op") === "set") await hold;
          await route.continue().catch(() => {});
        });
      }
      const input = ed.rowAt("name.0.family").locator("[data-set]");
      await input.fill("Changed");
      if (mode === "queued") {
        const pending = page.waitForRequest(r => r.url().endsWith("/ui/editor/render") && new URLSearchParams(r.postData() ?? "").get("op") === "set");
        await input.evaluate(el => (el as HTMLElement).blur());
        await pending;
      }
    }
    dialogsSeen(page);
    armDialog(page, "dismiss");
    await page.locator("#editor-cancel").click({ noWaitAfter: true });
    await expect.poll(() => page.url()).toContain("/ui/editor?");
    expect(dialogsSeen(page).map(d => d.type)).toEqual(["beforeunload"]);
    armDialog(page, "accept");
    await page.locator("#editor-back").click({ noWaitAfter: true });
    await page.waitForURL(`**${origin}`);
    expect(dialogsSeen(page).map(d => d.type)).toEqual(["beforeunload"]);
    release();
    await request.delete(`/Patient/${id}`);
  });
}

for (const failProjection of [false, true]) {
  test(`standalone create installs canonical identity${failProjection ? " after projection retry" : ""} and next save uses PUT`, async ({ page }) => {
    await page.goto("/ui/editor?type=Patient", { waitUntil: "networkidle" });
    const ed = new Editor(page, page.locator("#editor-body"));
    await expect(page.locator("#editor-delete")).toBeHidden();
    const writes: string[] = [];
    const payload = { resourceType: "Patient", id: "canonical-1723", active: true, meta: { versionId: "1" } };
    let failOnce = failProjection;
    await page.route("**/ui/editor/render", async route => {
      const doc = JSON.parse(new URLSearchParams(route.request().postData() ?? "").get("doc") ?? "{}");
      if (doc.id === payload.id && failOnce) { failOnce = false; await route.fulfill({ status: 503, body: "projection unavailable" }); }
      else await route.continue();
    });
    await page.route(/\/Patient(?:\/canonical-1723)?$/, async route => {
      writes.push(route.request().method());
      await route.fulfill({ status: 200, contentType: "application/fhir+json", body: JSON.stringify(payload) });
    });
    await ed.fillRaw({ resourceType: "Patient", active: true });
    await page.locator("#editor-save").click();
    await expect(page.locator("#editor-delete")).toBeVisible();
    await expect(page.locator("#editor-save")).toBeEnabled();
    if (failProjection) {
      await expect(page.locator("#editor-status")).not.toBeEmpty();
      await expect(page.locator("#editor .tag--unsaved")).toBeVisible();
      await page.locator("#editor-save").click();
      await expect.poll(async () => (await ed.currentDoc()).id).toBe(payload.id);
      await expect(page.locator("#editor-save")).toBeEnabled();
      expect(writes).toEqual(["POST"]);
    }
    await expect.poll(async () => (await ed.currentDoc()).id).toBe(payload.id);
    await expect(page.locator("#editor .tag--unsaved")).toBeHidden();
    await page.locator("#editor-save").click();
    await expect.poll(() => writes).toEqual(["POST", "PUT"]);
  });
}

for (const type of ["Patient", "SearchParameter", "CompartmentDefinition"] as const) {
  test(`${type} authored id and failed load do not expose Delete`, async ({ page }) => {
    await page.goto(`/ui/editor?type=${type}`, { waitUntil: "networkidle" });
    const ed = new Editor(page, page.locator("#editor-body"));
    await ed.applyJson({ resourceType: type, id: "unsaved-1723" });
    await expect(page.locator("#editor-delete")).toBeHidden();
    await page.route(`**/${type}/missing-1723`, route => route.fulfill({ status: 404, contentType: "application/fhir+json", body: '{}' }));
    await page.goto(`/ui/editor?type=${type}&id=missing-1723`, { waitUntil: "networkidle" });
    await expect(page.locator("#editor-delete")).toBeHidden();
    await expect(page.locator("#editor-status")).not.toBeEmpty();
    await expect(page.locator("#editor-save")).toBeDisabled();
  });
}

test("Delete uses confirmed identity despite invalid JSON, and failed Delete keeps dirty state", async ({ page, request }) => {
  const id = await createResource(request, "Patient", { active: true });
  const origin = "/ui/resources?type=Patient#opening";
  await page.goto(`/ui/editor?type=Patient&id=${id}&return_to=${encodeURIComponent(origin)}`, { waitUntil: "networkidle" });
  const ed = new Editor(page, page.locator("#editor-body"));
  await ed.enterRaw();
  await ed.source.fill("{invalid");
  const targets: string[] = [];
  let fail = true;
  await page.route(`**/Patient/${id}`, async route => {
    targets.push(route.request().url());
    await route.fulfill({ status: fail ? 500 : 204, body: "" });
  });
  await page.locator("#editor-delete").click();
  await acceptConfirm(page);
  await expect(page.locator("#editor-status")).toHaveText("500");
  await expect(page.locator("#editor .tag--unsaved")).toBeVisible();
  await expect(ed.source).toHaveValue("{invalid");
  fail = false;
  dialogsSeen(page);
  await page.locator("#editor-delete").click();
  await acceptConfirm(page);
  await page.waitForURL(`**${origin}`);
  // The in-page confirmation (#1667), and a suspended guard: no native dialog.
  expect(dialogsSeen(page)).toEqual([]);
  expect(targets).toHaveLength(2);
  expect(targets.every(url => new URL(url).pathname === `/Patient/${id}`)).toBe(true);
  await request.delete(`/Patient/${id}`);
});

for (const [type, section, selection, selected] of [
  ["SearchParameter", "search-parameters", "sel", "http://example.org/confirmed-1723"],
  ["CompartmentDefinition", "compartments", "def", "Patient"],
]) {
  test(`${type} Delete clears only the confirmed selection and preserves origin query/hash`, async ({ page }) => {
    const resource = type === "SearchParameter"
      ? { resourceType: type, id: "confirmed-1723", url: selected, name: "Confirmed", status: "draft", description: "Example", code: "confirmed", base: ["Patient"], type: "string" }
      : { resourceType: type, id: "confirmed-1723", url: "http://example.org/compartment", name: "Confirmed", status: "draft", code: selected, search: true };
    await page.route(`**/${type}/confirmed-1723`, route => route.fulfill({ status: route.request().method() === "DELETE" ? 204 : 200, contentType: "application/fhir+json", body: route.request().method() === "DELETE" ? "" : JSON.stringify(resource) }));
    const origin = `/ui/${section}?q=keep&${selection}=${encodeURIComponent(selected)}#opening`;
    await page.goto(`/ui/editor?type=${type}&id=confirmed-1723&return_to=${encodeURIComponent(origin)}`, { waitUntil: "networkidle" });
    const ed = new Editor(page, page.locator("#editor-body"));
    await ed.fillRaw({ ...resource, id: "authored-other", url: "http://example.org/unsaved", code: "Other" });
    await page.locator("#editor-delete").click();
    await acceptConfirm(page);
    await page.waitForURL(url => url.pathname === `/ui/${section}`);
    const returned = new URL(page.url());
    expect(returned.searchParams.get("q")).toBe("keep");
    expect(returned.searchParams.get("refresh")).toBe("1");
    expect(returned.searchParams.has(selection)).toBe(false);
    expect(returned.hash).toBe("#opening");
  });
}

test("caller New links carry current query and browser fragment", async ({ page }) => {
  const origin = "/ui/search-parameters?type=Patient&q=name#opening";
  await page.goto(origin, { waitUntil: "networkidle" });
  await page.locator("a[data-editor-link].btn--primary").click();
  await page.waitForURL(url => url.pathname === "/ui/editor");
  expect(new URL(page.url()).searchParams.get("return_to")).toBe(origin);
  await expect(page.locator("#editor-cancel")).toHaveAttribute("href", origin);
});

test("failed queued primitive prevents Save from persisting an earlier document", async ({ page, request }) => {
  const id = await createResource(request, "Patient", { name: [{ family: "Original" }] });
  await page.goto(`/ui/editor?type=Patient&id=${id}`, { waitUntil: "networkidle" });
  const ed = new Editor(page, page.locator("#editor-body"));
  let writes = 0;
  page.on("request", r => { if (r.method() === "PUT" && new URL(r.url()).pathname === `/Patient/${id}`) writes++; });
  await page.route("**/ui/editor/render", async route => {
    if (new URLSearchParams(route.request().postData() ?? "").get("op") === "set") await route.fulfill({ status: 503, body: "failed set" });
    else await route.continue();
  });
  await ed.rowAt("name.0.family").locator("[data-set]").fill("Unsaved");
  await page.locator("#editor-save").click();
  await expect(page.locator("#editor-status")).not.toBeEmpty();
  await expect(page.locator("#editor .tag--unsaved")).toBeVisible();
  expect(writes).toBe(0);
  await request.delete(`/Patient/${id}`);
});

test("standalone action wrapping keeps Cancel and Save inside a narrow viewport", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  const id = "a".repeat(64);
  await page.route(`**/Patient/${id}`, route => route.fulfill({ status: 200, contentType: "application/fhir+json", body: JSON.stringify({ resourceType: "Patient", id, active: true, meta: { lastUpdated: "2026-01-01T00:00:00Z" } }) }));
  await page.goto(`/ui/editor?type=Patient&id=${id}`, { waitUntil: "networkidle" });
  // Exercise the action bar independently of the existing mobile JSON/form
  // column layout: changing the in-flight field raises the same dirty cue.
  await page.locator("#editor-doc").evaluate(field => {
    const input = field as HTMLInputElement;
    input.value = JSON.stringify({ ...JSON.parse(input.value), active: false });
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await expect(page.locator("#editor .tag--unsaved")).toBeVisible();
  for (const selector of ["#editor-delete", "#editor-cancel", "#editor-save"]) {
    await expect(page.locator(selector)).toBeVisible();
    const rect = await page.locator(selector).boundingBox();
    expect(rect).not.toBeNull();
    expect(rect!.x).toBeGreaterThanOrEqual(0);
    expect(rect!.x + rect!.width).toBeLessThanOrEqual(390);
  }
});

for (const [type, section, selection, selected, recover] of [
  ["SearchParameter", "search-parameters", "sel", "http://example.org/saved-1723", false],
  ["CompartmentDefinition", "compartments", "def", "Patient", false],
  ["SearchParameter", "search-parameters", "sel", "http://example.org/saved-1723", true],
] as const) {
  test(`${type} successful ${recover ? "save recovery" : "save"} refreshes catalog Cancel without clearing opening selection`, async ({ page }) => {
    const doc = type === "SearchParameter"
      ? { resourceType: type, url: selected, name: "Saved1723", status: "draft", description: "Example", code: "saved1723", base: ["Patient"], type: "string" }
      : { resourceType: type, url: "http://example.org/compartment-saved-1723", name: "Saved1723", status: "draft", code: selected, search: true, resource: [{ code: "Patient", param: ["_id"] }] };
    const payload = { ...doc, id: "saved-1723" };
    const origin = `/ui/${section}?q=keep&${selection}=${encodeURIComponent(selected)}#opening`;
    await page.goto(`/ui/editor?type=${type}&return_to=${encodeURIComponent(origin)}`, { waitUntil: "networkidle" });
    const ed = new Editor(page, page.locator("#editor-body"));
    await ed.fillRaw(doc);
    for (const id of ["editor-back", "editor-cancel"]) await expect(page.locator(`#${id}`)).toHaveAttribute("href", origin);
    let failOnce = recover;
    await page.route("**/ui/editor/render", async route => {
      const submitted = JSON.parse(new URLSearchParams(route.request().postData() ?? "").get("doc") ?? "{}");
      if (submitted.id === payload.id && failOnce) {
        failOnce = false;
        await route.fulfill({ status: 503, body: "projection unavailable" });
      } else await route.continue();
    });
    await page.route(`**/${type}`, route => route.fulfill({ status: 201, contentType: "application/fhir+json", body: JSON.stringify(payload) }));
    await page.locator("#editor-save").click();
    if (recover) {
      await expect(page.locator("#editor-status")).not.toBeEmpty();
      await expect(page.locator("#editor-save")).toBeEnabled();
      await expect(page.locator("#editor-cancel")).toHaveAttribute("href", origin);
      await page.locator("#editor-save").click();
    }
    await expect(page.locator("#editor-announce")).toContainText(/saved/i);
    await expect(page.locator("#editor .tag--unsaved")).toBeHidden();
    for (const id of ["editor-back", "editor-cancel"]) {
      const href = await page.locator(`#${id}`).getAttribute("href");
      const destination = new URL(href!, page.url());
      expect(destination.pathname).toBe(`/ui/${section}`);
      expect(destination.searchParams.get("refresh")).toBe("1");
      expect(destination.searchParams.get("q")).toBe("keep");
      expect(destination.searchParams.get(selection)).toBe(selected);
      expect(destination.hash).toBe("#opening");
    }
    dialogsSeen(page);
    await page.locator("#editor-cancel").click();
    await page.waitForURL(url => url.pathname === `/ui/${section}`);
    expect(new URL(page.url()).searchParams.get("refresh")).toBe("1");
    expect(dialogsSeen(page)).toEqual([]);
  });
}

test("a confirmed catalog save preserves an explicit origin in another section", async ({ page }) => {
  const origin = "/ui/queries?q=keep#opening";
  await page.goto(`/ui/editor?type=SearchParameter&return_to=${encodeURIComponent(origin)}`, { waitUntil: "networkidle" });
  const ed = new Editor(page, page.locator("#editor-body"));
  const doc = { resourceType: "SearchParameter", url: "http://example.org/cross-1723", name: "Cross1723", status: "draft", description: "Example", code: "cross1723", base: ["Patient"], type: "string" };
  await ed.fillRaw(doc);
  await page.route("**/SearchParameter", route => route.fulfill({ status: 201, contentType: "application/fhir+json", body: JSON.stringify({ ...doc, id: "cross-1723" }) }));
  await page.locator("#editor-save").click();
  await expect(page.locator("#editor-announce")).toContainText(/saved/i);
  await expect(page.locator("#editor .tag--unsaved")).toBeHidden();
  for (const id of ["editor-back", "editor-cancel"]) await expect(page.locator(`#${id}`)).toHaveAttribute("href", origin);
});

for (const [loaded, edited] of [["Original", " Original "], ['{"a":1}', '{ "a": 1 }']]) {
  test(`standalone protects exact unblurred primitive text ${JSON.stringify(edited)}`, async ({ page }) => {
    const id = "exact-string-1723";
    const patient = { resourceType: "Patient", id, name: [{ family: loaded }] };
    await page.route(`**/Patient/${id}`, route => route.fulfill({ status: 200, contentType: "application/fhir+json", body: JSON.stringify(patient) }));
    await page.goto(`/ui/editor?type=Patient&id=${id}`, { waitUntil: "networkidle" });
    const ed = new Editor(page, page.locator("#editor-body"));
    const input = ed.rowAt("name.0.family").locator("[data-set]");
    await input.fill(edited);
    await expect(input).toBeFocused();
    expect(await page.evaluate(rootId => (window as any).HfsUnsaved.isDirty(document.getElementById(rootId)), "editor")).toBe(true);
    dialogsSeen(page);
    armDialog(page, "dismiss");
    await page.evaluate(() => window.location.assign("/ui/resources?type=Patient#native-exit"));
    expect(dialogsSeen(page).map(d => d.type)).toEqual(["beforeunload"]);
    await expect(input).toHaveValue(edited);
    armDialog(page, "accept");
    await page.evaluate(() => window.location.assign("/ui/resources?type=Patient#native-exit"));
    await page.waitForURL("**/ui/resources?type=Patient#native-exit");
    expect(dialogsSeen(page).map(d => d.type)).toEqual(["beforeunload"]);
  });
}

for (const mode of ["raw", "projected", "projected-reformatted"] as const) {
  test(`standalone explicit ${mode} replacement recovers a failed guided mutation and saves once`, async ({ page }) => {
    const id = "raw-recovery-1723";
    const original = { resourceType: "Patient", id, name: [{ family: "Original" }] };
    let writes = 0;
    let persisted = original;
    await page.route(`**/Patient/${id}`, async route => {
      if (route.request().method() === "PUT") { writes++; persisted = JSON.parse(route.request().postData()!); }
      await route.fulfill({ status: 200, contentType: "application/fhir+json", body: JSON.stringify(persisted) });
    });
    await page.goto(`/ui/editor?type=Patient&id=${id}`, { waitUntil: "networkidle" });
    const ed = new Editor(page, page.locator("#editor-body"));
    await page.route("**/ui/editor/render", async route => {
      if (new URLSearchParams(route.request().postData() ?? "").get("op") === "set") await route.fulfill({ status: 503, body: "failed primitive" });
      else await route.continue();
    });
    const field = ed.rowAt("name.0.family").locator("[data-set]");
    await field.fill("Failed guided value");
    const failed = page.waitForResponse(r => r.url().endsWith("/ui/editor/render") && r.status() === 503);
    await field.evaluate(input => (input as HTMLElement).blur());
    await failed;
    await page.waitForFunction(rootId => !(window as any).HfsEditorAdd.hasPendingMutations(document.getElementById(rootId)), "editor-body");
    const replacement = { ...original, name: [{ family: "Raw replacement" }] };
    await ed.fillRaw(replacement);
    if (mode === "projected" || mode === "projected-reformatted") {
      const projected = page.waitForResponse(r => r.url().endsWith("/ui/editor/render") && !new URLSearchParams(r.request().postData() ?? "").get("op"));
      await ed.leaveRaw();
      await projected;
      await expect.poll(async () => ((await ed.currentDoc()).name as any[])[0].family).toBe("Raw replacement");
      if (mode === "projected-reformatted") {
        await ed.enterRaw();
        await ed.source.fill((await ed.source.inputValue()) + "\n");
      }
    }
    await page.locator("#editor-save").click();
    await expect(page.locator("#editor-announce")).toContainText(/saved/i);
    await expect(page.locator("#editor .tag--unsaved")).toBeHidden();
    expect(writes).toBe(1);
    expect(persisted.name[0].family).toBe("Raw replacement");
    await expect.poll(async () => ((await ed.currentDoc()).name as any[])[0].family).toBe("Raw replacement");
  });
}

test("standalone format-only raw edit cannot bypass a failed unapplied guided mutation", async ({ page }) => {
  const id = "raw-format-1723";
  const patient = { resourceType: "Patient", id, name: [{ family: "Original" }] };
  let writes = 0;
  await page.route(`**/Patient/${id}`, route => { if (route.request().method() === "PUT") writes++; return route.fulfill({ status: 200, contentType: "application/fhir+json", body: JSON.stringify(patient) }); });
  await page.goto(`/ui/editor?type=Patient&id=${id}`, { waitUntil: "networkidle" });
  const ed = new Editor(page, page.locator("#editor-body"));
  await page.route("**/ui/editor/render", async route => {
    if (new URLSearchParams(route.request().postData() ?? "").get("op") === "set") await route.fulfill({ status: 503, body: "failed primitive" });
    else await route.continue();
  });
  const field = ed.rowAt("name.0.family").locator("[data-set]");
  await field.fill("Failed guided value");
  const failed = page.waitForResponse(r => r.url().endsWith("/ui/editor/render") && r.status() === 503);
  await field.evaluate(input => (input as HTMLElement).blur());
  await failed;
  await ed.enterRaw();
  await ed.source.fill((await ed.source.inputValue()) + "\n\n");
  await page.locator("#editor-save").click();
  await expect(page.locator("#editor-status")).toContainText("503");
  await expect(page.locator("#editor .tag--unsaved")).toBeVisible();
  expect(writes).toBe(0);
});
