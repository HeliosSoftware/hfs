// Bulk Import dialogs: dismissal clears the form (#682), the Add Manifest
// field is labeled Format (#684), and its headers textarea matches the other
// fields instead of overflowing the panel (#685).
import { expect, test } from "../pages/fixtures";
import { seedBulkImportDetail } from "../pages/routes";

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
