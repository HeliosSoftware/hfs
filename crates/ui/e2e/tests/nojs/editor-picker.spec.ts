import { test, expect } from "../../pages/fixtures";

// #1239: no page ever renders the add-element picker without JavaScript —
// the Resource Editor's body is fetched by editor.js itself, and the View
// Definitions guided form stays hidden behind `needs-js` until theme.js has
// run (nojs/sql-view-definitions.spec.ts's own
// "the guided-form card stays hidden" test). So this mounts the exact
// server fragment either host's client-side fetch would swap in — the same
// `POST /ui/editor/render` body `router_http.rs`'s own tests post — directly
// via `setContent`, to prove the picker itself is a plain nested
// `<details>`/`<summary>` disclosure that needs no script at all.
test("the add picker renders as nested native disclosures with JavaScript off", async ({
  page,
  request,
}) => {
  const response = await request.post("/ui/editor/render", {
    form: { doc: '{"resourceType":"Patient"}', op: "" },
  });
  expect(response.ok()).toBeTruthy();
  await page.setContent(await response.text());

  // Empty document: the root picker auto-opens (#547).
  const picker = page.locator("details.editor-add--picker");
  await expect(picker).toHaveAttribute("open", "");

  const elements = picker.locator("details.editor-add__group").first();
  await expect(elements).toHaveAttribute("open", "");
  const extensions = picker.locator("details.editor-add__group[data-add-group='extensions']");
  await expect(extensions).not.toHaveAttribute("open");

  // The native <details> toggle needs no script — a plain click on the
  // summary opens/closes it.
  await extensions.locator("summary").first().click();
  await expect(extensions).toHaveAttribute("open", "");

  await picker.locator("summary").first().click();
  await expect(picker).not.toHaveAttribute("open");

  await expect(page.locator("script")).toHaveCount(0);
});

test("issue1720 grouped arrays retain named headers and indexed native controls without JavaScript", async ({ page, request }) => {
  const response = await request.post("/ui/editor/render", {
    form: { doc: JSON.stringify({ resourceType: "Patient", name: [{ given: ["Ana", "Bea"] }] }), op: "" },
  });
  expect(response.ok()).toBeTruthy();
  await page.setContent(await response.text());
  const group = page.locator(".editor-row[data-path='name.0.given']");
  await expect(group).toHaveAttribute("data-collection", "");
  await expect(group.locator(".editor-row__label")).toHaveText("given");
  await expect(group.locator("[data-collection-add]")).toHaveAttribute("data-add", "name.0");
  await expect(group.locator("[data-collection-add]")).toHaveAttribute("data-name", "given");
  await expect(page.locator("[data-set='name.0.given.0']")).toHaveValue("Ana");
  await expect(page.locator("[data-set='name.0.given.1']")).toHaveAccessibleName("given[1] — name.0.given.1");
  await expect(page.locator("[data-add-status]")).toHaveAttribute("role", "status");
  await expect(page.locator("[data-add-undo-note]")).toBeHidden();
  await expect(page.locator(".editor-add__added, script")).toHaveCount(0);
});
