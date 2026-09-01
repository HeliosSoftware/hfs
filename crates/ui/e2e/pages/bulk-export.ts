// Bulk Export builder (/ui/bulk-export/new): scope, All Resources, individual
// resource types, narrowing controls, and native form actions.
import type { Locator, Page } from "@playwright/test";

export class BulkExportPage {
  constructor(readonly page: Page) {}

  async goto(): Promise<void> {
    await this.page.goto("/ui/bulk-export/new", { waitUntil: "networkidle" });
  }

  get form(): Locator {
    return this.page.locator("form.bulk-export-form");
  }

  get nameHeading(): Locator {
    return this.page.locator("[data-bulk-export-name-heading]");
  }

  get nameInput(): Locator {
    return this.form.locator('input[name="name"]');
  }

  get nameError(): Locator {
    return this.form.locator("#bulk-export-name-error");
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

  get sinceCustomError(): Locator {
    return this.form.locator("#bulk-export-since-custom-error");
  }

  scopeRadio(scope: "system" | "patient" | "group"): Locator {
    return this.form.locator(`input[name="scope"][value="${scope}"]`);
  }

  get patientCombobox(): Locator {
    return this.form.locator("#bulk-export-patients");
  }

  get patientFallback(): Locator {
    return this.patientCombobox.locator('textarea[name="patient"]');
  }

  get patientSearch(): Locator {
    return this.patientCombobox.getByRole("combobox");
  }

  get patientListbox(): Locator {
    return this.patientCombobox.getByRole("listbox");
  }

  get patientMessage(): Locator {
    return this.patientCombobox.locator("[data-combobox-message]");
  }

  get patientHint(): Locator {
    return this.patientCombobox.locator("[data-combobox-hint]");
  }

  get selectedPatients(): Locator {
    return this.patientCombobox.locator('[data-combobox-selected-input][name="patient"]');
  }

  get clearButton(): Locator {
    return this.form.getByRole("button", { name: "Clear", exact: true });
  }

  get clearLink(): Locator {
    return this.form.getByRole("link", { name: "Clear", exact: true });
  }

  get startButton(): Locator {
    return this.form.getByRole("button", { name: "Start Export", exact: true });
  }
}
