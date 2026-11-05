import { test, expect } from "../pages/fixtures";
import type { Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";

// Exercise the shipped shared control against real DOM, independently of any
// page's query/JSON logic. Consumer specs cover those business contracts.
async function fixture(page: Page, options: { empty?: boolean; mount?: boolean } = {}) {
  await page.goto("/ui/bulk-export/new", { waitUntil: "networkidle" });
  await page.evaluate(options => {
    const host = document.createElement("section");
    host.id = "fixed-fixture";
    host.style.cssText = "padding:16px;margin:16px";
    const form = document.createElement("form");
    form.id = "fixed-fixture-form";
    const select = document.createElement("select");
    select.id = "fixed-fixture-native";
    select.name = "choice";
    select.className = "field__input";
    select.setAttribute("form", form.id);
    select.setAttribute("aria-label", "Selection");
    select.setAttribute("data-combobox-select", "");
    select.dataset.events = "[]";
    if (!options.empty) {
      select.replaceChildren(new Option("Default", ""), new Option("One", "one"), new Option("Two", "two"));
    }
    for (const type of ["input", "change"]) select.addEventListener(type, () => {
      select.dataset.events = JSON.stringify([...JSON.parse(select.dataset.events!), type]);
    });
    host.append(form, select);
    document.querySelector("main")!.append(host);
    if (options.mount !== false) (window as any).HfsCombobox.install(host);
  }, options);
  return {
    host: page.locator("#fixed-fixture"),
    native: page.locator("#fixed-fixture-native"),
    root: page.locator("#fixed-fixture-native-combobox"),
    trigger: page.locator("#fixed-fixture-native-combobox [data-combobox-trigger]"),
    listbox: page.locator("#fixed-fixture-native-combobox [role=listbox]"),
  };
}

test("an empty adopted select refreshes silently and keeps the sole form-associated value", async ({ page }) => {
  const f = await fixture(page, { empty: true });
  await expect(f.native).toBeHidden();
  await expect(f.trigger).toBeDisabled();
  await expect(f.trigger).toHaveAccessibleName("Selection");
  await f.native.evaluate(select => {
    select.replaceChildren(new Option("Default", ""), new Option("One", "one"), new Option("Two", "two"));
    (window as any).HfsCombobox.refresh(select);
  });
  await expect(f.trigger).toBeEnabled();
  await expect(f.trigger).toHaveAccessibleName("Selection Default");
  await expect(f.native).toHaveAttribute("data-events", "[]");
  expect(await page.locator("#fixed-fixture-form").evaluate(form => new FormData(form as HTMLFormElement).getAll("choice"))).toEqual([""]);
  await f.trigger.focus();
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("End");
  await page.keyboard.press("Enter");
  await expect(f.native).toHaveValue("two");
  await expect(f.native).toHaveAttribute("data-events", '["input","change"]');
  expect(await page.locator("#fixed-fixture-form").evaluate(form => new FormData(form as HTMLFormElement).getAll("choice"))).toEqual(["two"]);
  await f.native.evaluate(node => {
    const select = node as HTMLSelectElement;
    select.value = "";
    select.options[0].textContent = "Everything";
    (window as any).HfsCombobox.refresh(select);
  });
  await expect(f.trigger).toHaveAccessibleName("Selection Everything");
  await expect(f.native).toHaveAttribute("data-events", '["input","change"]');
  await f.trigger.click();
  await expect(f.listbox.locator('[aria-selected="true"]')).toHaveCount(1);
  await expect(f.listbox.locator('[data-value=""]')).toHaveAttribute("aria-selected", "true");
});

test("applicability survives silent refreshes and a source lock blocks a stale open popup", async ({ page }) => {
  const f = await fixture(page);
  await f.native.evaluate(node => {
    const select = node as HTMLSelectElement;
    select.disabled = true;
    (window as any).HfsCombobox.refresh(select);
  });
  await expect(f.trigger).toBeDisabled();
  await f.native.evaluate(node => {
    const select = node as HTMLSelectElement;
    select.disabled = false;
    (window as any).HfsCombobox.refresh(select, { available: false });
    select.value = "two";
    (window as any).HfsCombobox.refresh(select);
  });
  await expect(f.root).toBeHidden();
  await expect(f.native).toBeHidden();
  await f.native.evaluate(select => { (window as any).HfsCombobox.refresh(select, { available: true }); });
  await expect(f.trigger).toHaveAccessibleName("Selection Two");
  await f.trigger.click();
  await expect(f.listbox).toBeVisible();
  // The owner locks after open, before it has painted a refresh.
  await f.native.evaluate(select => { (select as HTMLSelectElement).disabled = true; });
  await f.listbox.locator('[data-value="one"]').click();
  await expect(f.native).toHaveValue("two");
  await expect(f.native).toHaveAttribute("data-events", "[]");
  await expect(f.trigger).toBeDisabled();
  await expect(f.listbox).toBeHidden();
});

test("keyboard skips disabled options and Escape dismisses only the innermost open popup", async ({ page }) => {
  const f = await fixture(page);
  await f.native.evaluate(node => {
    (node as HTMLSelectElement).options[1].disabled = true;
    (window as any).HfsCombobox.refresh(node);
  });
  await f.host.evaluate(host => {
    host.setAttribute("data-escapes", "0");
    host.addEventListener("keydown", event => {
      if ((event as KeyboardEvent).key === "Escape") host.setAttribute("data-escapes", String(Number(host.getAttribute("data-escapes")) + 1));
    });
  });
  await f.trigger.focus();
  await page.keyboard.press("ArrowDown");
  await page.keyboard.press("ArrowDown");
  await expect(f.trigger).toHaveAttribute("aria-activedescendant", "fixed-fixture-native-combobox-option-2");
  await page.keyboard.press("Enter");
  await expect(f.native).toHaveValue("two");
  await f.trigger.click();
  await page.keyboard.press("Home");
  await page.keyboard.press("Escape");
  await expect(f.native).toHaveValue("two");
  await expect(f.trigger).toBeFocused();
  await expect(f.host).toHaveAttribute("data-escapes", "0");
  await page.keyboard.press("Escape");
  await expect(f.host).toHaveAttribute("data-escapes", "1");
  await f.trigger.click();
  await page.keyboard.press("Tab");
  await expect(f.listbox).toBeHidden();
  await expect(f.trigger).not.toBeFocused();
});

test("idempotent installs, cloned HTMX fragments and teardown preserve one live mount", async ({ page }) => {
  const f = await fixture(page);
  expect(await f.native.evaluate(select => {
    const api = (window as any).HfsCombobox;
    const first = api.adopt(select);
    api.install(document.getElementById("fixed-fixture"));
    return first === api.adopt(select);
  })).toBe(true);
  await f.host.evaluate(host => {
    const clone = host.cloneNode(true) as HTMLElement;
    (window as any).HfsCombobox.destroy(host);
    host.replaceWith(clone);
    clone.dispatchEvent(new Event("htmx:afterSwap", { bubbles: true }));
    clone.querySelector("select")!.addEventListener("change", () => {
      clone.dataset.changes = String(Number(clone.dataset.changes || 0) + 1);
    });
  });
  await f.trigger.click();
  await expect(f.listbox).toBeVisible();
  await f.listbox.locator('[data-value="one"]').click();
  await expect(f.native).toHaveValue("one");
  await expect(f.host).toHaveAttribute("data-changes", "1");
  await expect(f.listbox.locator('[role="option"]')).toHaveCount(3);
  await f.host.evaluate(host => {
    (host.querySelector("form") as HTMLFormElement).reset();
    host.dispatchEvent(new Event("htmx:beforeCleanupElement", { bubbles: true }));
  });
  await expect(f.trigger).toHaveCount(0);
  await expect(f.native).toBeVisible();
  await page.evaluate(() => new Promise(resolve => setTimeout(resolve, 0)));
  await expect(f.host).toHaveAttribute("data-changes", "1");
});

test("a queued scroll event with an unchanged anchor keeps an opened popup usable", async ({ page }) => {
  const f = await fixture(page);
  await f.trigger.click();
  await expect(f.listbox).toBeVisible();
  await page.evaluate(() => document.dispatchEvent(new Event("scroll")));
  await expect(f.listbox).toBeVisible();
  await f.listbox.locator('[data-value="one"]').click();
  await expect(f.native).toHaveValue("one");
});

test("a partially failed adoption restores the native select and a later refresh can retry", async ({ page }) => {
  const f = await fixture(page, { mount: false });
  expect(await f.native.evaluate(select => {
    const original = EventTarget.prototype.addEventListener;
    EventTarget.prototype.addEventListener = function (...args: Parameters<typeof original>) {
      if (args[0] === "keydown" && this instanceof Element && this.matches("[data-combobox-trigger]")) throw new Error("Injected mount failure");
      return original.apply(this, args);
    };
    try { return (window as any).HfsCombobox.adopt(select) === null; }
    finally { EventTarget.prototype.addEventListener = original; }
  })).toBe(true);
  await expect(f.native).toBeVisible();
  await expect(f.root).toHaveCount(0);
  await f.native.selectOption("two"); // Native fallback is deliberately the surface under test.
  await f.native.evaluate(select => { (window as any).HfsCombobox.refresh(select); });
  await expect(f.native).toBeHidden();
  await expect(f.trigger).toHaveAccessibleName("Selection Two");
});

for (const theme of ["light", "dark"] as const) {
  test(`a long popup escapes editor clipping, keeps dialog ownership and fits the viewport in ${theme}`, async ({ page, chrome }) => {
    await chrome.seedTheme(theme);
    await page.setViewportSize({ width: 640, height: 480 });
    const f = await fixture(page);
    const scrollBefore = await f.native.evaluate(node => {
      const select = node as HTMLSelectElement;
      select.replaceChildren(...Array.from({ length: 100 }, (_, i) => new Option(`Option ${i} with a long readable label`, String(i))));
      const api = (window as any).HfsCombobox.refresh(select);
      const dialog = document.createElement("div");
      dialog.id = "fixed-fixture-dialog";
      dialog.className = "card editor-form";
      dialog.setAttribute("role", "dialog");
      dialog.setAttribute("aria-modal", "true");
      dialog.setAttribute("aria-label", "Test editor");
      dialog.style.cssText = "position:fixed;right:16px;bottom:16px;width:320px;height:160px;overflow:hidden;z-index:110;padding:8px";
      const tree = document.createElement("div");
      tree.className = "editor-tree";
      tree.style.cssText = "height:120px;overflow:auto";
      const spacer = document.createElement("div");
      spacer.style.height = "200px";
      tree.append(spacer, api.root);
      dialog.append(tree);
      document.body.append(dialog);
      tree.scrollTop = tree.scrollHeight;
      return tree.scrollTop;
    });
    await f.trigger.click();
    const popup = (await f.listbox.boundingBox())!;
    const dialog = (await page.locator("#fixed-fixture-dialog").boundingBox())!;
    expect(popup.x).toBeGreaterThanOrEqual(8);
    expect(popup.x + popup.width).toBeLessThanOrEqual(632);
    expect(popup.y).toBeGreaterThanOrEqual(8);
    expect(popup.y + popup.height).toBeLessThanOrEqual(472);
    expect(popup.y).toBeLessThan(dialog.y);
    expect(await f.listbox.evaluate(list => {
      const rect = list.getBoundingClientRect();
      return Boolean(list.closest('[role="dialog"]')) && list.contains(document.elementFromPoint(rect.left + 12, rect.top + 12));
    })).toBe(true);
    await page.keyboard.press("End");
    await expect.poll(() => f.listbox.evaluate(list => list.scrollTop)).toBeGreaterThan(0);
    expect(await page.locator("#fixed-fixture-dialog .editor-tree").evaluate(tree => tree.scrollTop)).toBe(scrollBefore);
    const { violations } = await new AxeBuilder({ page }).include("#fixed-fixture-dialog")
      .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"]).analyze();
    expect(violations).toEqual([]);
    await page.locator("#fixed-fixture-dialog .editor-tree").evaluate(tree => { tree.scrollTop -= 10; });
    await expect(f.listbox).toBeHidden();
    await f.trigger.click();
    await page.setViewportSize({ width: 620, height: 460 });
    await expect(f.listbox).toBeHidden();
  });
}
