// Shared browser acceptance contract for the fixed-choice mode in both forms.
import { test, expect } from "./fixtures";
import { BulkExportPage } from "./bulk-export";
import { SqlExportPage } from "./sql-export";
import { createResource, deleteResources, waitSearchable } from "./api";
import type { Page, APIRequestContext } from "@playwright/test";

type Kind = "bulk-export" | "sql-export";
async function open(page: Page, request: APIRequestContext, kind: Kind) {
  let id: string | undefined;
  if (kind === "sql-export") {
    id = await createResource(request, "ViewDefinition", {
      name: `since_contract_${Date.now()}`, status: "active", resource: "Patient",
      select: [{ column: [{ name: "id", path: "getResourceKey()" }] }],
    });
    await waitSearchable(request, "ViewDefinition", id);
  }
  const picker = kind === "bulk-export" ? new BulkExportPage(page) : new SqlExportPage(page);
  await page.goto(`/ui/${kind === "bulk-export" ? "bulk-export" : "sql/export"}/new`, { waitUntil: "networkidle" });
  return { picker, cleanup: async () => { if (id) await deleteResources(request, "ViewDefinition", [id]); } };
}

export function fixedSinceTests(kind: Kind): void {
  test("Since fixed choices submit exactly one value and synchronize Custom", async ({ page, request }) => {
    const { picker, cleanup } = await open(page, request, kind);
    try {
      await expect(picker.sincePreset).toBeHidden();
      await expect(picker.sincePreset).toBeEnabled();
      await expect(picker.sinceTrigger).toHaveAccessibleName("Since All time");
      for (const value of ["", "day", "week", "month", "custom"]) {
        await picker.chooseSince(value);
        expect(await picker.form.evaluate(form => new FormData(form as HTMLFormElement).getAll("since_preset"))).toEqual([value]);
        await picker.sinceTrigger.click();
        await expect(picker.sinceCombobox.locator('[role="option"][aria-selected="true"]')).toHaveCount(1);
        await expect(picker.sinceCombobox.locator(`[role="option"][data-value="${value}"]`)).toHaveAttribute("aria-selected", "true");
        const label = await picker.sincePreset.locator(`option[value="${value}"]`).textContent();
        await expect(picker.sinceTrigger).toHaveAccessibleName(`Since ${label}`);
        await page.keyboard.press("Escape");
        await expect(picker.sinceTrigger).toBeFocused();
        if (value === "custom") await expect(picker.sinceCustom).toBeEnabled();
        else await expect(picker.sinceCustom).toBeDisabled();
      }
      await expect(picker.sinceCustom).not.toHaveAttribute("required", /.*/);
      await picker.sinceCustom.fill("2020-01-01T00:00:00Z");
      await picker.chooseSince("day");
      await picker.chooseSince("custom");
      await expect(picker.sinceCustom).toHaveValue("2020-01-01T00:00:00Z");
    } finally { await cleanup(); }
  });

  test("Since repeated asset execution mounts once and emits one change", async ({ page, request }) => {
    const { picker, cleanup } = await open(page, request, kind);
    try {
      await page.evaluate(async () => {
        await new Promise<void>((resolve, reject) => {
          const script = document.createElement("script");
          script.src = "/ui/assets/combobox.js";
          script.onload = () => resolve();
          script.onerror = () => reject(new Error("Repeated combobox asset failed to load"));
          document.head.appendChild(script);
        });
      });
      await picker.sinceCombobox.evaluate(root => {
        let changes = 0;
        root.querySelector("select")!.addEventListener("change", () => {
          root.setAttribute("data-test-changes", String(++changes));
        });
      });
      await picker.sinceTrigger.click();
      await expect(picker.sinceTrigger).toHaveAttribute("aria-expanded", "true");
      await expect(picker.sinceCombobox.getByRole("listbox")).toBeVisible();
      await picker.sinceCombobox.locator('[role="option"][data-value="custom"]').click();
      await expect(picker.sinceTrigger).toHaveAttribute("aria-expanded", "false");
      await expect(picker.sincePreset).toHaveValue("custom");
      await expect(picker.sinceCustom).toBeEnabled();
      await expect(picker.sinceCombobox).toHaveAttribute("data-test-changes", "1");
      await expect(picker.sinceCombobox.locator('[role="option"]')).toHaveCount(5);
    } finally { await cleanup(); }
  });

  test("Since Custom can submit an empty optional instant", async ({ page, request }) => {
    const { picker, cleanup } = await open(page, request, kind);
    try {
      await picker.nameInput.fill("Optional custom instant");
      if (kind === "sql-export") await picker.form.locator('input[name="subject"]').first().check();
      await picker.chooseSince("custom");
      await picker.sinceCustom.fill("");
      const target = kind === "bulk-export" ? "/ui/bulk-export" : "/ui/sql/export";
      await page.route(`**${target}`, route => route.request().method() === "POST"
        ? route.fulfill({ status: 204 }) : route.continue());
      const sent = page.waitForRequest(req => req.url().endsWith(target) && req.method() === "POST");
      await picker.startButton.click();
      const params = new URLSearchParams((await sent).postData() ?? "");
      expect(params.getAll("since_preset")).toEqual(["custom"]);
      expect(params.get("since_custom")).toBe("");
      await expect(picker.sinceCustomError).toBeHidden();
    } finally { await cleanup(); }
  });

  test("Since keyboard separates active and selected choices, closes and preserves focus", async ({ page, request }) => {
    const { picker, cleanup } = await open(page, request, kind);
    try {
      const trigger = picker.sinceTrigger;
      const list = picker.sinceCombobox.getByRole("listbox");
      await trigger.focus();
      await page.keyboard.press("ArrowDown");
      await expect(trigger).toHaveAttribute("aria-expanded", "true");
      await page.keyboard.press("End");
      await expect(trigger).toHaveAttribute("aria-activedescendant", `${kind}-since-option-4`);
      await expect(picker.sincePreset).toHaveValue("");
      await page.keyboard.press("Escape");
      await expect(trigger).not.toHaveAttribute("aria-activedescendant", /.*/);
      await expect(trigger).toBeFocused();
      await page.keyboard.press("Space");
      await page.keyboard.press("End");
      await page.keyboard.press("Enter");
      await expect(picker.sincePreset).toHaveValue("custom");
      await expect(list).toBeHidden();
      await page.keyboard.press("Enter");
      await page.keyboard.press("Home");
      await page.keyboard.press("ArrowDown");
      await page.keyboard.press("ArrowUp");
      await page.keyboard.press("Space");
      await expect(picker.sincePreset).toHaveValue("");
      await trigger.click();
      await page.keyboard.press("Tab");
      await expect(list).toBeHidden();
      await expect(trigger).not.toBeFocused();
      await trigger.click();
      await page.locator("h1").click();
      await expect(list).toBeHidden();
    } finally { await cleanup(); }
  });

  test("Since reset, native changes, page restore and repeat mounts stay synchronized", async ({ page, request }) => {
    const { picker, cleanup } = await open(page, request, kind);
    try {
      await picker.chooseSince("custom");
      await picker.form.evaluate(form => (form as HTMLFormElement).reset());
      await expect(picker.sincePreset).toHaveValue("");
      await expect(picker.sinceTrigger).toHaveText("All time");
      await expect(picker.sinceCustom).toBeDisabled();
      await picker.sincePreset.evaluate(select => {
        (select as HTMLSelectElement).value = "week";
        select.dispatchEvent(new Event("change", { bubbles: true }));
      });
      await expect(picker.sinceTrigger).toHaveText((await picker.sincePreset.locator('option[value="week"]').textContent())!);
      await picker.sincePreset.evaluate(select => { (select as HTMLSelectElement).value = "month"; });
      await page.evaluate(() => window.dispatchEvent(new Event("pageshow")));
      await expect(picker.sinceTrigger).toHaveText((await picker.sincePreset.locator('option[value="month"]').textContent())!);
      await picker.sincePreset.evaluate(select => { (select as HTMLSelectElement).value = "custom"; });
      await page.evaluate(() => window.dispatchEvent(new Event("pageshow")));
      await expect(picker.sinceCustom).toBeEnabled();
      await picker.sinceCombobox.evaluate(root => {
        root.dispatchEvent(new Event("htmx:afterSwap", { bubbles: true }));
        root.dispatchEvent(new Event("htmx:afterSwap", { bubbles: true }));
        let changes = 0;
        root.querySelector("select")!.addEventListener("change", () => { root.setAttribute("data-test-changes", String(++changes)); });
      });
      await picker.chooseSince("day");
      await expect(picker.sinceCombobox).toHaveAttribute("data-test-changes", "1");
      await expect(picker.sinceCombobox.locator('[role="option"]')).toHaveCount(5);
      // htmx history fragments clone HTML without its listeners, including
      // the ready marker. A cloned fixed field must mount afresh.
      await picker.sinceCombobox.evaluate(root => {
        const clone = root.cloneNode(true);
        root.replaceWith(clone);
        clone.dispatchEvent(new Event("htmx:afterSwap", { bubbles: true }));
      });
      await picker.chooseSince("custom");
      await expect(picker.sincePreset).toHaveValue("custom");
      await expect(picker.sinceCombobox.locator('[role="option"]')).toHaveCount(5);
    } finally { await cleanup(); }
  });

  for (const failure of ["missing asset", "partial mount"]) {
    test(`Since native fallback survives ${failure}`, async ({ page, request }) => {
      if (failure === "missing asset") await page.route("**/ui/assets/combobox.js", route => route.abort());
      else await page.addInitScript(() => {
        const add = EventTarget.prototype.addEventListener;
        let failed = false;
        EventTarget.prototype.addEventListener = function (...args: Parameters<typeof add>) {
          if (!failed && args[0] === "keydown" && this instanceof Element && this.matches(".combobox--fixed [role=combobox]")) {
            failed = true;
            throw new Error("Injected mount failure after click listener registration");
          }
          return add.apply(this, args);
        };
      });
      const { picker, cleanup } = await open(page, request, kind);
      try {
        await expect(picker.sincePreset).toBeVisible();
        await expect(picker.sincePreset).toBeEnabled();
        await expect(picker.sinceTrigger).toBeHidden();
        await picker.sincePreset.selectOption("custom");
        await expect(picker.sinceCustom).toBeEnabled();
        expect(await picker.form.evaluate(form => new FormData(form as HTMLFormElement).getAll("since_preset"))).toEqual(["custom"]);
        if (failure === "partial mount") {
          await expect(picker.sinceCombobox.locator('[role="option"]')).toHaveCount(0);
          // The failed transaction removes its first registered listener.
          await picker.sinceTrigger.evaluate(trigger => trigger.dispatchEvent(new Event("click")));
          await expect(picker.sinceTrigger).toHaveAttribute("aria-expanded", "false");
          await picker.sinceCombobox.evaluate(root => root.dispatchEvent(new Event("htmx:afterSwap", { bubbles: true })));
          await picker.chooseSince("week");
          await expect(picker.sinceCombobox.locator('[role="option"]')).toHaveCount(5);
          expect(await picker.form.evaluate(form => new FormData(form as HTMLFormElement).getAll("since_preset"))).toEqual(["week"]);
        }
      } finally { await cleanup(); }
    });
  }
}
