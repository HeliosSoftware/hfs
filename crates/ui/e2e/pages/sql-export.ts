// Active SQL Exports (/ui/sql/export) and its builder (/ui/sql/export/new),
// #833. Card-internal controls (status chip, overflow actions, "View files")
// are addressed by role/text straight off `SqlExportPage.card()` in specs,
// matching the rest of this Page Object Model — this class only owns what
// every spec needs to name more than once: navigation and the builder's form.
import type { Locator, Page } from "@playwright/test";

export class SqlExportPage {
  constructor(readonly page: Page) {}

  async goto(): Promise<void> {
    await this.page.goto("/ui/sql/export", { waitUntil: "networkidle" });
  }

  async gotoNew(): Promise<void> {
    await this.page.goto("/ui/sql/export/new", { waitUntil: "networkidle" });
  }

  get newButton(): Locator {
    return this.page.getByRole("link", { name: "New SQL Export" });
  }

  get notice(): Locator {
    return this.page.locator(".notice");
  }

  get lede(): Locator {
    return this.page.locator(".page-head__lede");
  }

  /** One job card on the list, matched by its name (the subjects' names
   * when the job has none of its own — the only kind the form offers
   * today). Several cards can share a name (e.g. after Run again); narrow
   * further with `.first()`/`.nth()`, which the list's most-recent-first
   * order makes deterministic. */
  card(name: string): Locator {
    return this.page.locator(".job-card").filter({ hasText: name });
  }

  // --- The builder (/ui/sql/export/new) ---

  subjectCheckbox(reference: string): Locator {
    return this.page.locator(`input[name="subject"][value="${reference}"]`);
  }

  /** A subject row, by its `data-name` (#834's filterable subjects table). */
  subjectRow(name: string): Locator {
    return this.page.locator(`.table-card tbody tr[data-name="${name}"]`);
  }

  /** A subject row, by its own reference — unlike `subjectRow`, keyed the
   * same way the values row below it is (`param:{reference}:...` fields,
   * `.row--params[data-subject]`), which is what #837's own locators need. */
  private subjectRowByReference(reference: string): Locator {
    return this.page.locator(".table-card tbody tr[data-kind]").filter({
      has: this.page.locator(`input[name="subject"][value="${reference}"]`),
    });
  }

  // --- #837: a parameterized SQL Query's own values row (expand/collapse
  // chevron, collapsed chip summary, and its parameter fields). ---

  /** The row-toggle chevron in a parameterized subject's own Subject cell —
   * hidden by the server, revealed by sql-export-form.js only once the
   * query is checked. */
  rowToggle(reference: string): Locator {
    return this.subjectRowByReference(reference).locator(".row-toggle");
  }

  /** The collapsed-summary chip strip next to the chevron, filled in only
   * while the values row is folded. */
  paramSummary(reference: string): Locator {
    return this.subjectRowByReference(reference).locator(".param-summary");
  }

  /** The values row itself, right under the subject's own row. */
  paramsRow(reference: string): Locator {
    return this.page.locator(`.table-card tbody tr.row--params[data-subject="${reference}"]`);
  }

  /** One parameter's own input/select inside its subject's values row. */
  paramField(reference: string, name: string): Locator {
    return this.paramsRow(reference).locator(`[name="param:${reference}:${name}"]`);
  }

  /** One of the four output-format choice cards' radio inputs (#834). */
  formatOption(format: "ndjson" | "csv" | "json" | "parquet"): Locator {
    return this.page.locator(`input[name="format"][value="${format}"]`);
  }

  // --- The subjects table's filter/switch/select-all tools (#834), a
  // JavaScript-only enhancement (sql-export-form.js) over the plain table. ---

  get subjectTypeSwitch(): Locator {
    return this.page.locator(".card-head__tools--subjects .seg");
  }

  subjectTypeButton(kind: "all" | "view-definition" | "sql-query" | "sql-view"): Locator {
    return this.page.locator(`[data-subject-filter="${kind}"]`);
  }

  get subjectFilterInput(): Locator {
    return this.page.locator(".card-head__tools--subjects input[type='search']");
  }

  get subjectSelectAll(): Locator {
    return this.page.locator(".table-card thead .col-check input[type='checkbox']");
  }

  get subjectsEmptyRow(): Locator {
    return this.page.locator(".table-card tbody tr.data-table__empty");
  }

  get selectedCount(): Locator {
    return this.page.locator("#sql-export-subjects-count");
  }

  get startButton(): Locator {
    return this.page.locator("form[action='/ui/sql/export'] button[type='submit']");
  }

  // --- "Narrow it down" (#836): patients/groups comboboxes and Since. ---

  get patientCombobox(): Locator {
    return this.page.locator("#sql-export-patients");
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

  get selectedPatients(): Locator {
    return this.patientCombobox.locator('[data-combobox-selected-input][name="patient"]');
  }

  get groupCombobox(): Locator {
    return this.page.locator("#sql-export-groups");
  }

  get groupFallback(): Locator {
    return this.groupCombobox.locator('textarea[name="group"]');
  }

  get groupSearch(): Locator {
    return this.groupCombobox.getByRole("combobox");
  }

  get groupListbox(): Locator {
    return this.groupCombobox.getByRole("listbox");
  }

  get selectedGroups(): Locator {
    return this.groupCombobox.locator('[data-combobox-selected-input][name="group"]');
  }

  get sincePreset(): Locator {
    return this.page.locator('select[name="since_preset"]');
  }

  get sinceCustom(): Locator {
    return this.page.locator('input[name="since_custom"]');
  }

  get sinceCustomError(): Locator {
    return this.page.locator("#sql-export-since-custom-error");
  }

  // --- "Advanced" (#836): tracking id and the CSV header switch. ---

  get advancedDetails(): Locator {
    return this.page.locator("details.card").filter({
      has: this.page.locator("summary", { hasText: "Advanced" }),
    });
  }

  /** Opens the "Advanced" `<details>` disclosure if it is not already open —
   * its content (tracking id, the header switch) is native-hidden by the
   * browser until then, exactly like any other closed `<details>`. */
  async openAdvanced(): Promise<void> {
    const isOpen = await this.advancedDetails.evaluate((el) => (el as HTMLDetailsElement).open);
    if (!isOpen) await this.advancedDetails.locator("summary").click();
  }

  get trackingIdInput(): Locator {
    return this.page.locator('input[name="client_tracking_id"]');
  }

  get headerCheckbox(): Locator {
    return this.page.locator('input[name="header"]');
  }

  /** The header checkbox's own `<label>` — what `sql-export-form.js` hides
   * for a non-csv format (#836); the checkbox itself stays in the DOM (and
   * enabled) throughout, only its label's visibility changes. */
  get headerLabel(): Locator {
    return this.page.locator('label:has(input[name="header"])');
  }

  // --- The job detail page (`/ui/sql/export/{id}`, #835/#836). ---

  /** One `.detail__field` on the job detail page, matched by its exact
   * `<span>` label — the same "one field, one row" shape every detail page
   * in this crate shares (compartments.html, search-parameters.html, ...). */
  private detailField(label: string): Locator {
    return this.page
      .locator(".detail__field")
      .filter({ has: this.page.locator("span", { hasText: label }) });
  }

  get detailTrackingId(): Locator {
    return this.detailField("Tracking id").locator("code");
  }

  get detailFormat(): Locator {
    return this.detailField("Format").locator("div");
  }

  get detailSince(): Locator {
    return this.detailField("Since").locator("div");
  }

  get detailPatients(): Locator {
    return this.detailField("Patients").locator(".detail__tags .tag");
  }

  get detailGroups(): Locator {
    return this.detailField("Groups").locator(".detail__tags .tag");
  }

  /** The Job card's Subjects field on the detail page: subject kind/name
   * pills plus, for a parameterized SQL Query, its own `:name = value`
   * chips (#837). Checked with `toContainText`, not an exact multi-
   * element match: chip order follows the record's own `subjects` order,
   * which is submission order, not necessarily the order a test happens to
   * list its expectations in. */
  get detailSubjects(): Locator {
    return this.detailField("Subjects").locator(".job-detail__subjects");
  }
}
