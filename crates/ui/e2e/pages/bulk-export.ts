// Bulk Export builder (/ui/bulk-export): scope, All Resources, individual
// resource types, narrowing controls, and native form actions.
import type { Locator, Page } from "@playwright/test";

export class BulkExportPage {
  constructor(readonly page: Page) {}

  async goto(): Promise<void> {
    await this.page.goto("/ui/bulk-export", { waitUntil: "networkidle" });
  }

  get form(): Locator {
    return this.page.locator("form.bulk-export-form");
  }

  get allResources(): Locator {
    return this.form.locator('input[name="all_types"]');
  }

  get typeCheckboxes(): Locator {
    return this.form.locator('input[name="types"]');
  }

  typeCheckbox(resourceType: string): Locator {
    return this.form.locator(`input[name="types"][value="${resourceType}"]`);
  }

  typeItem(resourceType: string): Locator {
    return this.form.locator("label.typegrid__item", {
      has: this.page.locator(`input[name="types"][value="${resourceType}"]`),
    });
  }

  typeLabel(resourceType: string): Locator {
    return this.typeItem(resourceType).locator(".typegrid__label");
  }

  get typeTooltip(): Locator {
    return this.page.locator("#filter-rail-tooltip");
  }

  get sincePreset(): Locator {
    return this.form.locator('select[name="since_preset"]');
  }

  get sinceCustom(): Locator {
    return this.form.locator('input[name="since_custom"]');
  }

  scopeRadio(scope: "system" | "patient" | "group"): Locator {
    return this.form.locator(`input[name="scope"][value="${scope}"]`);
  }

  get clearButton(): Locator {
    return this.form.getByRole("button", { name: "Clear", exact: true });
  }

  get startButton(): Locator {
    return this.form.getByRole("button", { name: "Start Export", exact: true });
  }
}
