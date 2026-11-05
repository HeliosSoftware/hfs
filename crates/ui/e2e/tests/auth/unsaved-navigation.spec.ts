import { test, expect, acceptConfirm, dismissConfirm, dialogsSeen } from "../../pages/fixtures";
import { VdEditor } from "../../pages/vd-editor";

// A local draft needs no authenticated REST seed or write. Both bearer-auth
// server legs run this smoke, including the one with no outbound service token.
test("issue1880 internal navigation protects a draft with bearer auth enabled", async ({ page }) => {
  await page.goto("/ui/sql/view-definitions?vd=new&lang=en");
  const editor = new VdEditor(page);
  const draft = JSON.stringify({ resourceType: "ViewDefinition", name: "auth_unsaved_navigation", status: "active", resource: "Patient", select: [{ column: [{ name: "id", path: "getResourceKey()" }] }] });
  await editor.setDoc(draft);
  await expect(page.locator("#vd-editor-form .tag--unsaved")).toBeVisible();
  await page.locator("#vd-editor-cancel").click();
  await dismissConfirm(page, "You have unsaved changes. Discard them and leave this page?");
  expect(await editor.doc()).toBe(draft);
  await expect(page.locator("#vd-editor-form .tag--unsaved")).toBeVisible();
  await page.locator("#vd-editor-cancel").click();
  await acceptConfirm(page, "You have unsaved changes. Discard them and leave this page?");
  await page.waitForURL(url => !url.searchParams.has("vd"));
  expect(dialogsSeen(page)).toEqual([]);
});
