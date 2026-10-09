import { test, expect, acceptConfirm, dismissConfirm, confirmDialog, dialogsSeen, armDialog, expectNativeDialog } from "../pages/fixtures";
import { VdEditor } from "../pages/vd-editor";
import { createResource, deleteResources, waitSearchable } from "../pages/api";
import type { Page } from "@playwright/test";

const MESSAGE = "You have unsaved changes. Discard them and leave this page?";
const ACTION = "Discard and leave";
const cueSelector = "#vd-editor-form .tag--unsaved";
const starter = (name: string) => ({ resourceType: "ViewDefinition", name, status: "active", resource: "Patient", select: [{ column: [{ name: "id", path: "getResourceKey()" }] }] });

async function dirtyDraft(page: Page) {
  await page.goto("/ui/sql/view-definitions?vd=new&lang=en", { waitUntil: "networkidle" });
  const editor = new VdEditor(page);
  const baseline = await editor.doc();
  await editor.setDoc(JSON.stringify(starter(`issue1880_${Date.now()}`)));
  await expect(page.locator(cueSelector)).toBeVisible();
  return { editor, baseline, draft: await editor.doc(), before: page.url() };
}

async function fixtureLink(page: Page, href: string, attrs: Record<string, string> = {}) {
  await page.evaluate(({ href, attrs }) => {
    const a = document.createElement("a");
    a.id = "issue1880-link";
    a.href = href;
    a.textContent = "Navigation fixture";
    Object.entries(attrs).forEach(([key, value]) => a.setAttribute(key, value));
    document.querySelector("main")!.appendChild(a);
    (window as any).htmx.process(a);
  }, { href, attrs });
  return page.locator("#issue1880-link");
}

async function nativeReloadRemainsGuarded(page: Page) {
  dialogsSeen(page);
  armDialog(page, "dismiss");
  await page.evaluate(() => window.location.reload());
  await expect.poll(() => dialogsSeen(page).map(d => d.type)).toEqual(["beforeunload"]);
  await expect(page.locator(cueSelector)).toBeVisible();
}

for (const dismissal of ["cancel", "escape", "backdrop"] as const) {
  test(`issue1880 ${dismissal} preserves the draft, loaded baseline and focus`, async ({ page }) => {
    const state = await dirtyDraft(page);
    const trigger = page.locator("#vd-editor-cancel");
    await trigger.click();
    const dialog = confirmDialog(page);
    await expect(dialog.locator(".confirm-dialog__message")).toHaveText(MESSAGE);
    await expect(dialog.locator("[data-confirm-ok]")).toHaveText(ACTION);
    await expect(dialog.locator("[data-confirm-cancel]")).toBeFocused();
    if (dismissal === "cancel") await dismissConfirm(page);
    else if (dismissal === "escape") await page.keyboard.press("Escape");
    else {
      const box = await dialog.boundingBox();
      expect(box).not.toBeNull();
      await page.mouse.click(Math.max(1, box!.x - 8), Math.max(1, box!.y - 8));
    }
    await expect(dialog).toHaveCount(0);
    await expect(page).toHaveURL(state.before);
    await expect(trigger).toBeFocused();
    expect(await state.editor.doc()).toBe(state.draft);
    await expect(page.locator(cueSelector)).toBeVisible();
    expect(dialogsSeen(page)).toEqual([]);
    await state.editor.setDoc(state.baseline);
    await expect(page.locator(cueSelector)).toBeHidden();
  });
}

for (const action of ["rail", "create", "filter"] as const) {
  test(`issue1880 ${action} replaces the dirty editor exactly once`, async ({ page, request }) => {
    const first = await createResource(request, "ViewDefinition", starter(`issue1880_first_${Date.now()}`));
    const second = await createResource(request, "ViewDefinition", starter(`issue1880_second_${Date.now()}`));
    try {
      await waitSearchable(request, "ViewDefinition", first);
      await waitSearchable(request, "ViewDefinition", second);
      await page.goto(`/ui/sql/view-definitions?vd=${first}&lang=en`, { waitUntil: "networkidle" });
      const editor = new VdEditor(page);
      await editor.setDoc(JSON.stringify({ ...JSON.parse(await editor.doc()), name: "unsaved_navigation" }));
      let destinations = 0;
      page.on("request", r => { if (r.isNavigationRequest() && r.method() === "GET") destinations++; });
      if (action === "rail") await page.locator(`#vd-rail-list a[data-type='${second}']`).click();
      else if (action === "create") await page.locator(".page-head__actions a[data-editor-link]").click();
      else {
        const filter = page.locator("form.filter-rail__search input[name='filter']");
        await filter.fill("issue1880");
        await filter.press("Enter");
      }
      await acceptConfirm(page, MESSAGE);
      await page.waitForURL(url => action === "rail" ? url.searchParams.get("vd") === second : action === "create" ? url.searchParams.get("vd") === "new" : url.searchParams.get("filter") === "issue1880");
      expect(destinations).toBe(1);
      expect(dialogsSeen(page)).toEqual([]);
    } finally { await deleteResources(request, "ViewDefinition", [first, second]); }
  });
}

test("issue1880 GET replay preserves effective submitter overrides and values", async ({ page }) => {
  await dirtyDraft(page);
  await page.evaluate(() => {
    const form = document.createElement("form");
    form.id = "issue1880-form";
    form.method = "post";
    form.action = "/ui/should-not-submit";
    form.innerHTML = '<input name="filter" value="two words"><button name="from" value="submitter" formmethod="get" formaction="/ui/sql/view-definitions#opening" formtarget="_self">Go</button>';
    document.querySelector("main")!.appendChild(form);
  });
  let destinationRequests = 0;
  page.on("request", r => { if (r.isNavigationRequest()) destinationRequests++; });
  await page.locator("#issue1880-form button").click();
  await dismissConfirm(page, MESSAGE);
  await expect(page.locator("#issue1880-form button")).toBeEnabled();
  expect(destinationRequests).toBe(0);
  await page.locator("#issue1880-form button").click();
  await acceptConfirm(page, MESSAGE);
  await page.waitForURL(url => url.searchParams.get("from") === "submitter");
  const url = new URL(page.url());
  expect(url.pathname).toBe("/ui/sql/view-definitions");
  expect(url.searchParams.get("filter")).toBe("two words");
  expect(url.hash).toBe("#opening");
  expect(destinationRequests).toBe(1);
  expect(dialogsSeen(page)).toEqual([]);
});

test("issue1880 a primitive settling during the dialog needs only one decision", async ({ page }) => {
  await dirtyDraft(page);
  const input = page.locator("#vd-editor-grid [data-set='name']");
  await input.fill("issue1880_settled");
  const settled = page.waitForResponse(r => r.url().endsWith("/ui/editor/render") && new URLSearchParams(r.request().postData() ?? "").get("op") === "set");
  await page.locator("#vd-editor-cancel").click();
  await settled;
  await expect(confirmDialog(page)).toHaveCount(1);
  await acceptConfirm(page, MESSAGE);
  await page.waitForURL(url => !url.searchParams.has("vd"));
  expect(dialogsSeen(page)).toEqual([]);
});

test("issue1880 a changed draft while confirmation is open needs a new decision", async ({ page }) => {
  await dirtyDraft(page);
  await page.locator("#vd-editor-cancel").click();
  await expect(confirmDialog(page)).toBeVisible();
  await page.locator("textarea[name='json']").evaluate(field => {
    const textarea = field as HTMLTextAreaElement;
    textarea.value = JSON.stringify({ ...JSON.parse(textarea.value), name: "newer_draft" });
    textarea.dispatchEvent(new Event("input", { bubbles: true }));
  });
  // The first answer removes its own dialog and synchronously asks the next
  // question; answer directly instead of the helper's zero-dialog assertion.
  await confirmDialog(page).locator("[data-confirm-ok]").click();
  await expect(confirmDialog(page)).toBeVisible();
  await dismissConfirm(page, MESSAGE);
  await expect(page.locator(cueSelector)).toBeVisible();
  await expect(page.locator("textarea[name='json']")).toHaveValue(/newer_draft/);
  expect(dialogsSeen(page)).toEqual([]);
});

for (const failure of ["204", "aborted", "cancelled", "disconnected"] as const) {
  test(`issue1880 ${failure} navigation keeps the editor protected`, async ({ page }) => {
    const state = await dirtyDraft(page);
    const link = await fixtureLink(page, "/ui/issue1880/retained");
    let requests = 0;
    await page.route("**/ui/issue1880/retained", async route => {
      requests++;
      if (failure === "aborted") await route.abort("aborted");
      else await route.fulfill({ status: 204, body: "" });
    });
    await link.click();
    await expect(confirmDialog(page)).toBeVisible();
    if (failure === "cancelled") await link.evaluate(a => a.addEventListener("click", e => e.preventDefault(), { once: true }));
    if (failure === "disconnected") await link.evaluate(a => a.remove());
    await acceptConfirm(page, MESSAGE);
    if (failure === "204" || failure === "aborted") await expect.poll(() => requests).toBe(1);
    else expect(requests).toBe(0);
    await expect(page).toHaveURL(state.before);
    await expect(page.locator(cueSelector)).toBeVisible();
    expect(dialogsSeen(page)).toEqual([]);
    if (failure === "disconnected") await fixtureLink(page, "/ui/issue1880/retained");
    await page.locator("#issue1880-link").click();
    await dismissConfirm(page, MESSAGE);
    await nativeReloadRemainsGuarded(page);
  });
}

test("issue1880 validation blocking GET replay does not consume protection", async ({ page }) => {
  await dirtyDraft(page);
  await page.evaluate(() => {
    const form = document.createElement("form");
    form.id = "issue1880-form";
    form.action = "/ui/resources";
    form.noValidate = true;
    form.innerHTML = '<input name="required" required><button>Navigate</button>';
    document.querySelector("main")!.appendChild(form);
  });
  await page.locator("#issue1880-form button").click();
  await expect(confirmDialog(page)).toBeVisible();
  await page.locator("#issue1880-form").evaluate(form => (form as HTMLFormElement).noValidate = false);
  await acceptConfirm(page, MESSAGE);
  await expect(page.locator(cueSelector)).toBeVisible();
  await nativeReloadRemainsGuarded(page);
});

for (const variant of ["anchor", "new-tab", "modified", "download"] as const) {
  test(`issue1880 ${variant} preserves the dirty editor without confirmation`, async ({ page, context }) => {
    const state = await dirtyDraft(page);
    let popup;
    if (variant === "anchor") {
      const link = await fixtureLink(page, "#vd-editor");
      await link.click();
      await expect(page).toHaveURL(/#vd-editor$/);
    } else if (variant === "download") {
      await page.route("**/ui/issue1880/download", route => route.fulfill({ status: 200, contentType: "text/plain", body: "download fixture" }));
      const link = await fixtureLink(page, "/ui/issue1880/download", { download: "fixture.txt" });
      const downloaded = page.waitForEvent("download");
      await link.click();
      expect((await downloaded).suggestedFilename()).toBe("fixture.txt");
    } else {
      const link = await fixtureLink(page, "/ui/resources", variant === "new-tab" ? { target: "_blank" } : {});
      const opened = context.waitForEvent("page");
      await link.click(variant === "modified" ? { modifiers: ["ControlOrMeta"] } : {});
      popup = await opened;
      await popup.waitForLoadState();
      await expect(page).toHaveURL(state.before);
      await popup.close();
    }
    expect(await state.editor.doc()).toBe(state.draft);
    await expect(page.locator(cueSelector)).toBeVisible();
    await expect(confirmDialog(page)).toHaveCount(0);
    expect(dialogsSeen(page)).toEqual([]);
  });
}

for (const mode of ["replacement", "partial", "abort"] as const) {
  test(`issue1880 HTMX GET ${mode} only confirms an editor replacement`, async ({ page }) => {
    const state = await dirtyDraft(page);
    const target = mode === "partial" ? "#run-notice" : "main";
    const link = await fixtureLink(page, "/ui/issue1880/htmx", { "hx-get": "/ui/issue1880/htmx", "hx-target": target, "hx-swap": "innerHTML" });
    let requests = 0;
    await page.route("**/ui/issue1880/htmx", async route => {
      requests++;
      if (mode === "abort") await route.abort("aborted");
      else await route.fulfill({ contentType: "text/html", body: '<p id="issue1880-result">Loaded</p>' });
    });
    await link.click();
    if (mode !== "partial") await acceptConfirm(page, MESSAGE);
    await expect.poll(() => requests).toBe(1);
    if (mode === "abort") {
      await expect(page.locator(cueSelector)).toBeVisible();
      await link.click();
      await dismissConfirm(page, MESSAGE);
      await nativeReloadRemainsGuarded(page);
    } else {
      await expect(page.locator("#issue1880-result")).toBeVisible();
      await expect(confirmDialog(page)).toHaveCount(0);
      if (mode === "partial") {
        expect(await state.editor.doc()).toBe(state.draft);
        await expect(page.locator(cueSelector)).toBeVisible();
      }
    }
    expect(dialogsSeen(page)).toEqual([]);
  });
}

test("issue1880 missing shared modal uses one native fallback, without a second unload prompt", async ({ page }) => {
  await dirtyDraft(page);
  await page.evaluate(() => { (window as any).HfsConfirm = undefined; });
  dialogsSeen(page);
  expectNativeDialog(page, { type: "confirm", message: MESSAGE, action: "accept" });
  await page.locator("#vd-editor-cancel").click();
  await page.waitForURL(url => !url.searchParams.has("vd"));
  expect(dialogsSeen(page)).toEqual([{ type: "confirm", message: MESSAGE }]);
});

test("issue1880 missing translated navigation copy retains native protection", async ({ page }) => {
  await dirtyDraft(page);
  await page.evaluate(() => { delete document.body.dataset.msgUnsavedLeave; });
  armDialog(page, "dismiss");
  await page.locator("#vd-editor-cancel").click();
  expect(dialogsSeen(page).map(d => d.type)).toEqual(["beforeunload"]);
  await expect(page.locator(cueSelector)).toBeVisible();
  await expect(confirmDialog(page)).toHaveCount(0);
});

test("issue1880 reload and tab close retain native browser protection", async ({ page }) => {
  await dirtyDraft(page);
  await nativeReloadRemainsGuarded(page);
  dialogsSeen(page);
  armDialog(page, "dismiss");
  await page.close({ runBeforeUnload: true });
  await expect.poll(() => dialogsSeen(page).map(d => d.type)).toEqual(["beforeunload"]);
  expect(page.isClosed()).toBe(false);
  await expect(page.locator(cueSelector)).toBeVisible();
});

for (const section of ["queries", "views"] as const) {
  test(`issue1880 SQL ${section} failed Save leaves the submitted Library protected`, async ({ page }) => {
    await page.goto(`/ui/sql/${section}?lib=new&lang=en`, { waitUntil: "networkidle" });
    const details = page.locator("#lib-details-editor .cm-content");
    await details.click();
    await page.keyboard.press("ControlOrMeta+a");
    await page.keyboard.insertText("{invalid");
    await expect(page.locator("#lib-editor-form .tag--unsaved")).toBeVisible();
    await page.locator("#lib-editor-form button[value='save']").click();
    await expect(page.locator(".notice--warn:not(#run-notice *)")).toContainText(/JSON|json/);
    await expect(page.locator("#lib-editor-form")).toHaveAttribute("data-unsaved-draft", "");
    await expect(page.locator("textarea[name='json']")).toHaveValue("{invalid");
    await expect(page.locator("#lib-editor-form .tag--unsaved")).toBeVisible();
    expect(await page.evaluate(() => (window as any).HfsUnsaved.isDirty())).toBe(true);
    await page.locator("#lib-editor-cancel").click();
    await dismissConfirm(page, MESSAGE);
    await expect(page.locator("textarea[name='json']")).toHaveValue("{invalid");
    expect(dialogsSeen(page)).toEqual([]);
  });
}

test("issue1880 internal navigation from the dirty Resources modal asks before leaving", async ({ page, resources, chrome }) => {
  await resources.goto("Patient");
  await resources.openCreate();
  await resources.modal.editor.applyJson({ resourceType: "Patient", name: [{ family: "resources_navigation_draft" }] });
  await expect(resources.modal.unsavedCue).toBeVisible();
  const before = page.url();
  const draft = await resources.modal.editor.currentDoc();
  await chrome.navLink("/ui/sql/view-definitions").evaluate(link => (link as HTMLAnchorElement).click());
  await dismissConfirm(page, MESSAGE);
  await expect(page).toHaveURL(before);
  await expect(resources.modal.root).toBeVisible();
  await expect(resources.modal.unsavedCue).toBeVisible();
  expect(await resources.modal.editor.currentDoc()).toEqual(draft);
  let navigations = 0;
  page.on("request", r => { if (r.isNavigationRequest()) navigations++; });
  await chrome.navLink("/ui/sql/view-definitions").evaluate(link => (link as HTMLAnchorElement).click());
  await acceptConfirm(page, MESSAGE);
  await page.waitForURL(url => url.pathname === "/ui/sql/view-definitions");
  expect(navigations).toBe(1);
  expect(dialogsSeen(page)).toEqual([]);
});

test("issue1880 boosted body navigation keeps one guard after repeated script loads", async ({ page }) => {
  await dirtyDraft(page);
  await page.evaluate(() => { (document as any).issue1880Original = true; });
  const link = await fixtureLink(page, "/ui/sql/view-definitions?vd=new&lang=en&roundtrip=1", { "hx-boost": "true" });
  let boosted = 0;
  page.on("request", r => { if (r.headers()["hx-boosted"] === "true") boosted++; });
  await link.click();
  await acceptConfirm(page, MESSAGE);
  await page.waitForURL(url => url.searchParams.get("roundtrip") === "1");
  expect(await page.evaluate(() => (document as any).issue1880Original)).toBe(true);
  expect(boosted).toBe(1);
  const editor = new VdEditor(page);
  await editor.setDoc(JSON.stringify(starter("after_boosted_navigation")));
  await page.evaluate(() => {
    const api = (window as any).HfsConfirm;
    const ask = api.ask;
    (window as any).issue1880Asks = 0;
    api.ask = (...args: unknown[]) => { (window as any).issue1880Asks++; return ask(...args); };
  });
  await page.locator("#vd-editor-cancel").click();
  await expect(confirmDialog(page)).toHaveCount(1);
  await dismissConfirm(page, MESSAGE);
  expect(await page.evaluate(() => (window as any).issue1880Asks)).toBe(1);
  await expect(page.locator(cueSelector)).toBeVisible();
  expect(dialogsSeen(page)).toEqual([]);
});

for (const [lang, message, action] of [
  ["es", "Hay cambios sin guardar. ¿Descartarlos y salir de esta página?", "Descartar y salir"],
  ["de", "Es gibt ungespeicherte Änderungen. Verwerfen und diese Seite verlassen?", "Verwerfen und verlassen"],
]) {
  test(`issue1880 navigation dialog renders the negotiated ${lang} wording`, async ({ page }) => {
    await page.goto(`/ui/sql/view-definitions?vd=new&lang=${lang}`);
    await new VdEditor(page).setDoc(JSON.stringify(starter("translated_unsaved_navigation")));
    await page.locator("#vd-editor-cancel").click();
    await expect(confirmDialog(page).locator(".confirm-dialog__message")).toHaveText(message);
    await expect(confirmDialog(page).locator("[data-confirm-ok]")).toHaveText(action);
    await dismissConfirm(page, message);
    await expect(page.locator(cueSelector)).toBeVisible();
    expect(dialogsSeen(page)).toEqual([]);
  });
}

for (const change of ["unchanged", "draft", "destination", "target", "source"] as const) {
  test(`issue1880 HTMX sequential confirmations revalidate ${change} immediately before issuing`, async ({ page }) => {
    await dirtyDraft(page);
    // Keep the link connected when only its replacement target is detached.
    await page.locator("#vd-editor-grid").evaluate(grid => {
      const wrapper = document.createElement("section");
      wrapper.id = "issue1880-target";
      grid.replaceWith(wrapper);
      wrapper.appendChild(grid);
    });
    const link = await fixtureLink(page, "/ui/issue1880/sequential", {
      "hx-get": "/ui/issue1880/sequential", "hx-target": "#issue1880-target",
      "hx-swap": "innerHTML", "hx-confirm": "Continue navigation?",
    });
    let requests = 0;
    await page.route("**/ui/issue1880/sequential", async route => {
      requests++;
      expect(route.request().headers()["hx-request"]).toBe("true");
      await route.fulfill({ contentType: "text/html", body: '<p id="issue1880-sequential-result">Loaded</p>' });
    });
    await link.click();
    const message = confirmDialog(page).locator(".confirm-dialog__message");
    await expect(message).toHaveText(MESSAGE);
    // The next question opens as the current dialog closes; the usual helper
    // deliberately expects zero dialogs and is unsuitable for this transition.
    await confirmDialog(page).locator("[data-confirm-ok]").click();
    await expect(message).toHaveText("Continue navigation?");
    expect(requests).toBe(0);

    if (change === "draft") {
      await page.locator("textarea[name='json']").evaluate(field => {
        const textarea = field as HTMLTextAreaElement;
        textarea.value = JSON.stringify({ ...JSON.parse(textarea.value), name: "newer_sequential_draft" });
        textarea.dispatchEvent(new Event("input", { bubbles: true }));
      });
      await confirmDialog(page).locator("[data-confirm-ok]").click();
      await expect(message).toHaveText(MESSAGE);
      expect(requests).toBe(0);
      await dismissConfirm(page, MESSAGE);
      await expect(page.locator("textarea[name='json']")).toHaveValue(/newer_sequential_draft/);
      await expect(page.locator(cueSelector)).toBeVisible();
    } else {
      if (change === "destination") await link.evaluate(a => (a as HTMLAnchorElement).href = "/ui/issue1880/changed");
      if (change === "target") await page.locator("#issue1880-target").evaluate(target => target.remove());
      if (change === "source") await link.evaluate(a => a.remove());
      await acceptConfirm(page, "Continue navigation?");
      if (change === "unchanged") {
        await expect(page.locator("#issue1880-sequential-result")).toBeVisible();
        expect(requests).toBe(1);
      } else {
        // Let browser activation and any queued HTMX request complete before
        // asserting zero; this is a frame boundary rather than a timer delay.
        await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
        expect(requests).toBe(0);
        await expect(page.locator("#issue1880-sequential-result")).toHaveCount(0);
        if (change !== "target") await expect(page.locator(cueSelector)).toBeVisible();
      }
    }
    expect(dialogsSeen(page)).toEqual([]);
  });
}

test("issue1880 pageshow revokes the second HTMX confirmation and a fresh attempt recovers", async ({ page }) => {
  const state = await dirtyDraft(page);
  const link = await fixtureLink(page, "/ui/issue1880/lifecycle", {
    "hx-get": "/ui/issue1880/lifecycle", "hx-target": "main", "hx-swap": "innerHTML",
    "hx-confirm": "Continue navigation?",
  });
  let requests = 0;
  await page.route("**/ui/issue1880/lifecycle", async route => {
    requests++;
    expect(route.request().headers()["hx-request"]).toBe("true");
    await route.fulfill({ contentType: "text/html", body: '<p id="issue1880-lifecycle-result">Loaded</p>' });
  });
  const message = confirmDialog(page).locator(".confirm-dialog__message");
  await link.click();
  await expect(message).toHaveText(MESSAGE);
  await confirmDialog(page).locator("[data-confirm-ok]").click();
  await expect(message).toHaveText("Continue navigation?");
  // Drive the actual lifecycle hook in the browser with a controlled restore
  // event; this does not claim a real navigation through the BFCache.
  await page.evaluate(() => window.dispatchEvent(new PageTransitionEvent("pageshow", { persisted: true })));
  await acceptConfirm(page, "Continue navigation?");
  await page.evaluate(() => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve()))));
  expect(requests).toBe(0);
  expect(await state.editor.doc()).toBe(state.draft);
  await expect(page.locator(cueSelector)).toBeVisible();
  expect(await page.evaluate(() => {
    const unload = new Event("beforeunload", { cancelable: true });
    window.dispatchEvent(unload);
    return unload.defaultPrevented;
  })).toBe(true, "the lifecycle revocation leaves the native guard armed");
  expect(dialogsSeen(page)).toEqual([]);

  await link.click();
  await expect(message).toHaveText(MESSAGE);
  await confirmDialog(page).locator("[data-confirm-ok]").click();
  await expect(message).toHaveText("Continue navigation?");
  await acceptConfirm(page, "Continue navigation?");
  await expect(page.locator("#issue1880-lifecycle-result")).toBeVisible();
  expect(requests).toBe(1);
  expect(dialogsSeen(page)).toEqual([]);
});
