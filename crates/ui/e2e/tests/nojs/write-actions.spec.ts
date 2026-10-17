import { expect, test } from "../../pages/fixtures";
import { createSqlQueryLibrary, deleteByNamePrefix, waitSearchable } from "../../pages/api";

// #1754 with JavaScript disabled: the structural guard (a disabled default
// button) is plain HTML, so Enter in a parameter value must not submit the
// editor form, and Save still works on click.
async function seed(request: import("@playwright/test").APIRequestContext, name: string): Promise<string> {
  const id = await createSqlQueryLibrary(
    request, name, `http://example.org/ViewDefinition/${name}_vd`, "SELECT :ward AS w FROM v",
    [{ name: "ward", use: "in", type: "string" }],
  );
  await waitSearchable(request, "Library", id);
  return id;
}

test("with JavaScript disabled, Enter in a parameter value neither navigates nor saves", async ({ page, request }) => {
  const name = `e2e_wa_nojs_enter_${Date.now().toString(36)}`;
  try {
    const id = await seed(request, name);
    await page.goto(`/ui/sql/queries?lib=${id}`);
    await expect(page.locator("html")).not.toHaveClass(/\bjs\b/);
    const posts: string[] = [];
    page.on("request", (r) => {
      if (r.method() === "POST") posts.push(r.url());
    });
    const field = page.locator("input[name='param:ward']");
    await field.fill("north");
    await field.press("Enter");
    await page.waitForTimeout(700);
    expect(posts).toEqual([]);
    expect(page.url()).toContain(`lib=${id}`);
    expect(page.url()).not.toContain("saved=1");
    await expect(field).toHaveValue("north");
    const res = await request.get(`/Library?name=${encodeURIComponent(name)}&_summary=count`);
    expect((await res.json()).total).toBe(1);
  } finally {
    await deleteByNamePrefix(request, "Library", name);
  }
});

test("with JavaScript disabled, clicking Save still saves", async ({ page, request }) => {
  const name = `e2e_wa_nojs_save_${Date.now().toString(36)}`;
  try {
    const id = await seed(request, name);
    await page.goto(`/ui/sql/queries?lib=${id}`);
    await page.locator("#lib-editor-form button[name='action'][value='save']").click();
    await expect(page).toHaveURL(/saved=1/);
  } finally {
    await deleteByNamePrefix(request, "Library", name);
  }
});
