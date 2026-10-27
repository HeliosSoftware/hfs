import { expect, type Page } from "@playwright/test";

/**
 * Assert the labelled-value spacing contract on every visible detail field:
 * the field owns exactly 5px between label and value, while its direct parent
 * owns a larger flex/grid row gap between sibling fields.
 */
export async function expectDetailFieldSpacing(page: Page, surface: string): Promise<void> {
  const result = await page.$$eval(".detail__field", (nodes) => {
    const visible = nodes.filter(
      (node): node is HTMLElement => node instanceof HTMLElement && node.offsetParent !== null,
    );
    const offenders = visible.flatMap((field) => {
      const label =
        field.querySelector(":scope > span")?.textContent?.trim().slice(0, 40) ||
        field.textContent?.trim().slice(0, 40) ||
        "unnamed field";
      const fieldStyle = getComputedStyle(field);
      const issues: string[] = [];
      if (fieldStyle.rowGap !== "5px") {
        issues.push(`field row-gap=${fieldStyle.rowGap}`);
      }

      const parent = field.parentElement;
      if (!parent) {
        issues.push("no parent");
      } else {
        const parentStyle = getComputedStyle(parent);
        const parentGap = Number.parseFloat(parentStyle.rowGap);
        if (
          (parentStyle.display !== "flex" && parentStyle.display !== "grid") ||
          !(parentGap > 5)
        ) {
          const parentName = `${parent.tagName.toLowerCase()}.${Array.from(parent.classList).join(".")}`;
          issues.push(
            `parent ${parentName} display=${parentStyle.display} row-gap=${parentStyle.rowGap}`,
          );
        }
      }

      return issues.length ? [`${label}: ${issues.join("; ")}`] : [];
    });
    return { count: visible.length, offenders };
  });

  expect(result.count, `${surface} should render at least one labelled detail field`).toBeGreaterThan(0);
  expect(
    result.offenders,
    `${surface} detail spacing violations:\n${result.offenders.join("\n")}`,
  ).toEqual([]);
}
