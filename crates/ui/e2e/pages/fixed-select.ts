import type { Locator } from "@playwright/test";

/** The native SELECT remains useful for value assertions. Interactions use
 * its visible projection so browser tests exercise the shipped popup. */
export function fixedControl(native: Locator): Locator {
  return native.locator('xpath=ancestor::*[@data-combobox-mode="fixed"][1]');
}

export function fixedTrigger(native: Locator): Locator {
  return fixedControl(native).locator("[data-combobox-trigger]");
}

export async function chooseFixed(native: Locator, value: string): Promise<void> {
  const root = fixedControl(native);
  const trigger = fixedTrigger(native);
  if (await trigger.getAttribute("aria-expanded") !== "true") await trigger.click();
  // Sort offers a fixed ID choice and a generated _id choice with the same
  // wire value. Choosing the first matches the native select's value setter.
  await root.locator(`[role="option"][data-value=${JSON.stringify(value)}]`).first().click();
}
