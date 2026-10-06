import { test, expect, acceptConfirm, dismissConfirm, confirmDialog, dialogsSeen } from "../pages/fixtures";
import { Editor } from "../pages/editor";
import type { Page } from "@playwright/test";
import type { CompartmentsPage } from "../pages/compartments";

async function navigateDefinitionsAndTabs(page: Page, compartments: CompartmentsPage): Promise<void> {
  for (let round = 0; round < 3; round++) {
    await compartments.railItem("Encounter").click();
    await page.waitForLoadState("networkidle");
    for (const tab of [/members/i, /test/i, /definition/i]) {
      await compartments.tab(tab).click();
      await page.waitForLoadState("networkidle");
    }
    await compartments.railItem("Patient").click();
    await page.waitForLoadState("networkidle");
  }
}

// The CompartmentDefinition viewer and its membership tester (/ui/compartments).
// The tester is a plain GET form that resolves against the same codegen'd table
// the REST compartment handler uses — so the four outcomes here are the API's.

test("the compartment rail and tabs render", async ({ compartments }) => {
  await compartments.goto();
  // The five spec compartments (Device/Encounter/Patient/Practitioner/RelatedPerson).
  await expect(compartments.railItems).toHaveCount(5);
  await expect(compartments.tab(/test/i)).toBeVisible();
});

// #754/#755: the server remembers the selected definition — no client
// script involved, since Compartments navigates via real (`hx-boost`) GETs
// and has no "Recently used" group to render — so picking one and returning
// through the nav with no `?def=` at all restores it: not just the
// current-request-only history entry a back button would exercise, but the
// actual stored `rails.compartments.last`.
test("picking a definition and returning through the nav (no ?def=) restores it", async ({
  compartments,
  chrome,
  page,
}) => {
  await compartments.goto();
  await expect(compartments.railItem("Patient")).toHaveAttribute("aria-current", "true");

  await compartments.railItem("Encounter").click();
  await page.waitForLoadState("networkidle");
  await expect(compartments.railItem("Encounter")).toHaveAttribute("aria-current", "true");

  await chrome.navLink("/ui/resources").click();
  await page.waitForLoadState("networkidle");
  await chrome.navLink("/ui/compartments").click();
  await page.waitForLoadState("networkidle");

  expect(new URL(page.url()).searchParams.has("def")).toBe(false);
  await expect(compartments.railItem("Encounter")).toHaveAttribute("aria-current", "true");
  await expect(compartments.railItem("Patient")).not.toHaveAttribute("aria-current", "true");
});

test("tester: a linked type is a member", async ({ compartments }) => {
  await compartments.gotoTester();
  await compartments.runTester("p1", "Observation");
  await expect(compartments.resultTitle).toHaveClass(/tester-result__title--ok/);
});

test("tester: the compartment's own type is a member", async ({ compartments }) => {
  await compartments.gotoTester();
  await compartments.runTester("p1", "Patient");
  await expect(compartments.resultTitle).toHaveClass(/tester-result__title--ok/);
  await expect(compartments.resultTitle).toContainText(/member/i);
});

test("tester: an unlinked type is not a member", async ({ compartments }) => {
  await compartments.gotoTester();
  await compartments.runTester("p1", "Medication");
  await expect(compartments.resultTitle).toHaveClass(/tester-result__title--danger/);
});

test("tester: the wildcard target fans out across member types", async ({ compartments }) => {
  await compartments.gotoTester();
  await compartments.runTester("p1", "*");
  // Fan-out title reports a count of member types, not an ok/danger verdict.
  await expect(compartments.resultTitle).toBeVisible();
  await expect(compartments.resultTitle).not.toHaveClass(/--danger/);
});

// CRUD (#237): the stored definitions carry ids, so the definition tab offers
// Edit (editor deep-link) and Delete; New sits in the page head. The delete
// round-trip restores the captured seed after testing the standalone editor.
test("the definition tab offers New, Edit, and Delete", async ({ page, compartments }) => {
  await compartments.goto();
  await expect(page.locator(".page-head__actions a.btn--primary")).toHaveAttribute(
    "href",
    /^\/ui\/editor\?type=CompartmentDefinition&return_to=/,
  );
  await expect(page.locator(".detail__actions a.btn")).toHaveAttribute(
    "href",
    /\/ui\/editor\?type=CompartmentDefinition&id=./,
  );
  await expect(page.locator(".detail__actions [data-crud-delete]")).toBeVisible();
});

// #1771: each boosted response carries conformance-crud.js again. Count asks
// as well as requests: the busy guard alone can conceal duplicate handlers.
test("Delete asks once after repeated navigation; Cancel, Escape and backdrop never delete", async ({
  page, compartments,
}) => {
  await compartments.goto();
  await page.evaluate(() => {
    const doc = document as Document & { hfs1771Original?: boolean; hfs1771Asks?: number };
    doc.hfs1771Original = true;
    doc.hfs1771Asks = 0;
    const confirm = (window as any).HfsConfirm;
    const ask = confirm.ask;
    confirm.ask = function (...args: unknown[]) {
      doc.hfs1771Asks!++;
      return ask.apply(confirm, args);
    };
  });
  await navigateDefinitionsAndTabs(page, compartments);
  expect(await page.evaluate(() => (document as any).hfs1771Original)).toBe(true);
  const del = page.locator(".detail__actions [data-crud-delete]");
  const id = await del.getAttribute("data-id");
  let deletes = 0;
  let release!: () => void;
  const parked = new Promise<void>((resolve) => { release = resolve; });
  await page.route(`**/CompartmentDefinition/${id}`, async (route) => {
    if (route.request().method() !== "DELETE") return route.continue();
    deletes++;
    await parked;
    await route.fulfill({ status: 204 });
  });

  for (const [index, dismissal] of ["cancel", "escape", "backdrop"].entries()) {
    await del.click();
    await expect(confirmDialog(page)).toHaveCount(1);
    await expect(confirmDialog(page)).toBeVisible();
    expect(await page.evaluate(() => (document as any).hfs1771Asks)).toBe(index + 1);
    if (dismissal === "cancel") await dismissConfirm(page);
    else if (dismissal === "escape") await page.keyboard.press("Escape");
    else {
      const box = await confirmDialog(page).boundingBox();
      expect(box).not.toBeNull();
      await page.mouse.click(Math.max(1, box!.x - 8), Math.max(1, box!.y - 8));
    }
    await expect(confirmDialog(page)).toHaveCount(0);
    expect(deletes).toBe(0);
    await expect(del).toBeEnabled();
  }

  await del.click();
  expect(await page.evaluate(() => (document as any).hfs1771Asks)).toBe(4);
  await acceptConfirm(page);
  await expect(del).toHaveAttribute("aria-busy", "true");
  await expect(del).toBeDisabled();
  await expect.poll(() => deletes).toBe(1);
  release();
  await page.waitForURL("**/ui/compartments?refresh=1");
  expect(deletes).toBe(1);
});

test("the native confirmation fallback asks once after repeated navigation", async ({ page, compartments }) => {
  await compartments.goto();
  await page.evaluate(() => { (document as any).hfs1771Original = true; });
  await navigateDefinitionsAndTabs(page, compartments);
  expect(await page.evaluate(() => (document as any).hfs1771Original)).toBe(true);
  const del = page.locator(".detail__actions [data-crud-delete]");
  const id = await del.getAttribute("data-id");
  let deletes = 0;
  await page.route(`**/CompartmentDefinition/${id}`, async (route) => {
    if (route.request().method() !== "DELETE") return route.continue();
    deletes++;
    await route.fulfill({ status: 500, body: "fallback test failure" });
  });
  // Exercise confirm.js's documented fallback without opening a real native
  // dialog or weakening the fixture's unexpected-dialog policy.
  await page.evaluate(() => {
    const state = window as any;
    state.hfs1771NativeConfirm = window.confirm;
    state.hfs1771ConfirmOk = document.body.dataset.msgConfirmOk;
    state.hfs1771FallbackCalls = 0;
    state.hfs1771FallbackAnswer = false;
    delete document.body.dataset.msgConfirmOk;
    window.confirm = () => { state.hfs1771FallbackCalls++; return state.hfs1771FallbackAnswer; };
  });
  try {
    await del.click();
    expect(await page.evaluate(() => (window as any).hfs1771FallbackCalls)).toBe(1);
    expect(deletes).toBe(0);
    await expect(confirmDialog(page)).toHaveCount(0);
    await page.evaluate(() => { (window as any).hfs1771FallbackAnswer = true; });
    await del.click();
    await expect(page.locator(".detail__actions .alert")).toBeVisible();
    expect(await page.evaluate(() => (window as any).hfs1771FallbackCalls)).toBe(2);
    expect(deletes).toBe(1);
    await expect(del).toBeEnabled();
  } finally {
    await page.evaluate(() => {
      const state = window as any;
      window.confirm = state.hfs1771NativeConfirm;
      document.body.dataset.msgConfirmOk = state.hfs1771ConfirmOk;
    });
  }
});

// Delete the actual selected seed: duplicate compartment codes would select the
// wrong definition. Restoration keeps the shared server's five seeds intact.
test("issue1772 dirty compartment editor deletion returns to refreshed Compartments", async ({
  page, request, compartments,
}) => {
  await compartments.goto();
  const edit = page.locator(".detail__actions a.btn");
  const href = await edit.getAttribute("href");
  const id = new URL(href!, "http://localhost").searchParams.get("id")!;
  const path = `/CompartmentDefinition/${id}`;
  const read = await request.get(path);
  expect(read.ok()).toBe(true);
  const original = await read.json();
  try {
    await edit.click();
    await page.waitForURL(/\/ui\/editor/);
    const editor = new Editor(page, page.locator("#editor-body"));
    const document = await editor.currentDoc();
    await editor.applyJson({ ...document, name: "Issue1772Unsaved" });
    await expect(page.locator("#editor .tag--unsaved")).toBeVisible();
    dialogsSeen(page);
    const deleted = page.waitForResponse(response =>
      new URL(response.url()).pathname === path && response.request().method() === "DELETE",
    );
    await page.locator("#editor-delete").click();
    await acceptConfirm(page);
    expect((await deleted).ok()).toBe(true);
    await page.waitForURL(url => url.pathname === "/ui/compartments" && url.searchParams.get("refresh") === "1");
    await expect(compartments.railItems).toHaveCount(4);
    await expect(page.locator(`[data-crud-delete][data-id="${id}"]`)).toHaveCount(0);
    await expect(page.locator(`a[href="${href}"]`)).toHaveCount(0);
    expect([404, 410]).toContain((await request.get(path)).status());
    expect(dialogsSeen(page).filter(dialog => dialog.type === "beforeunload")).toEqual([]);
  } finally {
    const restored = await request.put(path, {
      headers: { "Content-Type": "application/fhir+json" }, data: original,
    });
    expect(restored.ok(), "restore the selected compartment seed").toBe(true);
    await compartments.goto("?refresh=1");
    await expect(compartments.railItems).toHaveCount(5);
  }
});
