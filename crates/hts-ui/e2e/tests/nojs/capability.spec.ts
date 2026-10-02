import { expect, test } from "@playwright/test";

const PATH = "/ui/hts/capability-statement";

test("HTS exposes the first-level CapabilityStatement and a plain-JSON fallback without JavaScript", async ({
  page,
}) => {
  await page.goto(PATH);
  const card = page.locator("#capability-json-fold");
  await expect(card).toBeVisible();
  await expect(card.locator("[data-capability-json-actions]")).toBeHidden();
  await expect(card.locator("[data-capability-json-tree] > [data-path=''][data-offset='0']")).toBeVisible();
  await expect(card.locator("details[open]")).toHaveCount(0);

  const fallback = card.getByRole("link", { name: "Open plain JSON" });
  await expect(fallback).toBeVisible();
  await fallback.click();
  const url = new URL(page.url());
  expect(url.pathname).toBe(PATH);
  expect(url.searchParams.get("raw")).toBe("1");
  await expect(card.locator("pre.detail__code")).toBeVisible();
  await expect(card).toContainText(/Plain JSON fallback/i);
  await expect(card.locator("[data-capability-json-actions], #capability-json-body")).toHaveCount(0);
});
