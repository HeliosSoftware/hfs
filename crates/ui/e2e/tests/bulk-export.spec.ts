import { test, expect, acceptConfirm, dismissConfirm, confirmDialog } from "../pages/fixtures";
import type { Locator } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { axeSummary } from "../pages/axe";
import { createResource, deleteResources, waitSearchable } from "../pages/api";

const patientOptions = `
  <button type="button" class="combobox__option" data-combobox-option
          data-value="Patient/p-104" data-label="Ana Rivera">Ana Rivera · Patient/p-104</button>
  <button type="button" class="combobox__option" data-combobox-option
          data-value="Patient/p-205" data-label="Andrés Silva">Andrés Silva · Patient/p-205</button>`;

function patientMessage(message: string, options: { error?: boolean; alternate?: boolean } = {}) {
  return `<div id="bulk-export-patients-message" hx-swap-oob="innerHTML">
    ${options.alternate ? '<span data-combobox-use-alternate hidden></span>' : ""}
    <span class="field__hint${options.error ? " field__hint--error" : ""}"
          data-combobox-message-content>${message}</span>
  </div>`;
}

const clearPatientMessage =
  '<div id="bulk-export-patients-message" hx-swap-oob="innerHTML"></div>';

// Compare every non-type control, including disabled values and selected
// patients, plus the heading and validation feedback across the card action.
async function nonTypeState(form: Locator) {
  return form.evaluate((element) => ({
    fields: Array.from(
      element.querySelectorAll<HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement>(
        "input, select, textarea",
      ),
    )
      .filter((field) => field.name !== "types" && field.name !== "all_types")
      .map((field) => ({
        name: field.name,
        value: field.value,
        checked: field instanceof HTMLInputElement ? field.checked : null,
        disabled: field.disabled,
        invalid: field.getAttribute("aria-invalid"),
        describedBy: field.getAttribute("aria-describedby"),
      })),
    heading: document.querySelector("[data-bulk-export-name-heading]")?.textContent,
    errors: Array.from(element.querySelectorAll<HTMLElement>('[role="alert"]'))
      .map((error) => ({ text: error.textContent, hidden: error.hidden })),
  }));
}

test("All Resources is the enhanced default", async ({ bulkExport }) => {
  await bulkExport.goto();

  await expect(bulkExport.allResources).toBeChecked();
  await expect(bulkExport.typeCheckboxes).not.toHaveCount(0);
  expect(
    await bulkExport.typeCheckboxes.evaluateAll((types) =>
      types.every((type) => (type as HTMLInputElement).checked && (type as HTMLInputElement).disabled),
    ),
  ).toBe(true);
});

for (const selection of ["all", "individual"] as const) {
  test(`Clear only removes ${selection} resource types and preserves the filled form`, async ({
    page,
    bulkExport,
  }) => {
    await page.route("**/ui/lookup/patient-options*", (route) =>
      route.fulfill({ status: 200, contentType: "text/html", body: patientOptions }),
    );
    await bulkExport.goto();
    await bulkExport.nameInput.fill("Preserved export");
    await bulkExport.scopeRadio("group").check();
    await bulkExport.form.locator('input[name="group_id"]').fill("preserved-group");
    await bulkExport.scopeRadio("patient").check();
    await bulkExport.patientSearch.fill("an");
    await bulkExport.patientListbox.locator('[data-value="Patient/p-104"]').click();
    await bulkExport.patientSearch.press("ArrowDown");
    await bulkExport.patientListbox.locator('[data-value="Patient/p-205"]').click();
    await bulkExport.patientSearch.press("Escape");
    await expect(bulkExport.selectedPatients).toHaveCount(2);
    await bulkExport.form.locator('input[name="elements"]').fill("id,meta");
    await bulkExport.form.locator('input[name="type_filter"]').fill("Patient?active=true");
    await bulkExport.sincePreset.selectOption("custom");
    await bulkExport.sinceCustom.fill("2026-08-01T00:00:00Z");
    if (selection === "all") await bulkExport.sincePreset.selectOption("day");
    await bulkExport.until.fill("2099-01-01T00:00:00Z");
    if (selection === "individual") {
      await bulkExport.allResources.uncheck();
      await bulkExport.typeCheckbox("Patient").check();
      await bulkExport.typeCheckbox("Observation").check();
    }
    const before = await nonTypeState(bulkExport.form);
    const url = page.url();

    // The button must work through native keyboard activation too.
    await bulkExport.clearButton.focus();
    await bulkExport.clearButton.press("Enter");

    expect(page.url()).toBe(url);
    expect(await nonTypeState(bulkExport.form)).toEqual(before);
    await expect(bulkExport.nameHeading).toHaveText("Preserved export");
    await expect(bulkExport.scopeRadio("patient")).toBeChecked();
    await expect(bulkExport.selectedPatients).toHaveCount(2);
    await expect(bulkExport.allResources).not.toBeChecked();
    expect(await bulkExport.typeCheckboxes.evaluateAll((types) => types.every((type) =>
      !(type as HTMLInputElement).checked && !(type as HTMLInputElement).disabled,
    ))).toBe(true);
    await bulkExport.typeCheckbox("Patient").check();
    await expect(bulkExport.typeCheckbox("Patient")).toBeChecked();
  });
}

test("Name updates the heading as safe text and clearing restores the default", async ({
  page,
  bulkExport,
}) => {
  await bulkExport.goto();

  const defaultDocumentTitle = await page.title();
  await expect(bulkExport.nameHeading).toHaveText("Bulk Export");
  await expect(bulkExport.nameHeading).not.toHaveAttribute("aria-live", /.+/);
  await expect(bulkExport.nameHeading).not.toHaveAttribute("role", "status");

  await bulkExport.nameInput.fill("  Diabetes registry 2024  ");
  await expect(bulkExport.nameHeading).toHaveText("Diabetes registry 2024");
  await expect(page).toHaveTitle(defaultDocumentTitle);

  await bulkExport.nameInput.fill("<img src=x>");
  await expect(bulkExport.nameHeading).toHaveText("<img src=x>");
  await expect(bulkExport.nameHeading.locator("img")).toHaveCount(0);

  await bulkExport.nameInput.fill("   ");
  await expect(bulkExport.nameHeading).toHaveText("Bulk Export");

  await page.goto("/ui/bulk-export/new?lang=es", { waitUntil: "networkidle" });
  await expect(bulkExport.nameHeading).toHaveText("Exportación masiva");
  await bulkExport.nameInput.fill("Registro de diabetes");
  await expect(bulkExport.nameHeading).toHaveText("Registro de diabetes");
  await bulkExport.nameInput.fill("");
  await expect(bulkExport.nameHeading).toHaveText("Exportación masiva");
});

test("a long unbroken Name stays inside the heading and card at narrow width", async ({
  page,
  bulkExport,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await bulkExport.goto();
  const longName = "Export".repeat(41);

  await bulkExport.nameInput.fill(longName);
  await expect(bulkExport.nameHeading).toHaveText(longName);
  const headingGeometry = await bulkExport.nameHeading.evaluate((heading) => {
    const headingRect = heading.getBoundingClientRect();
    const containerRect = heading.parentElement!.getBoundingClientRect();
    return {
      withinContainer:
        headingRect.left >= containerRect.left - 1 &&
        headingRect.right <= containerRect.right + 1,
      noPageOverflow:
        document.documentElement.scrollWidth <= document.documentElement.clientWidth,
    };
  });
  expect(headingGeometry).toEqual({ withinContainer: true, noPageOverflow: true });

  await bulkExport.form.evaluate((form, name) => {
    const card = document.createElement("div");
    card.className = "card job-card";
    const head = document.createElement("div");
    head.className = "job-card__head";
    const cardName = document.createElement("h2");
    cardName.className = "job-card__name";
    cardName.textContent = name;
    const actions = document.createElement("div");
    actions.className = "job-card__actions";
    const status = document.createElement("span");
    status.className = "tag tag--in-progress";
    status.textContent = "In progress";
    const cancel = document.createElement("button");
    cancel.type = "button";
    cancel.className = "btn";
    cancel.textContent = "Cancel";
    actions.append(status, cancel);
    head.append(cardName, actions);
    card.append(head);
    form.after(card);
  }, longName);

  const card = page.locator(".job-card");
  const cardName = card.locator(".job-card__name");
  await expect(cardName).toHaveText(longName);
  const cardGeometry = await cardName.evaluate((name) => {
    const nameRect = name.getBoundingClientRect();
    const cardRect = name.closest(".job-card")!.getBoundingClientRect();
    return {
      withinCard:
        nameRect.left >= cardRect.left - 1 && nameRect.right <= cardRect.right + 1,
      noPageOverflow:
        document.documentElement.scrollWidth <= document.documentElement.clientWidth,
    };
  });
  expect(cardGeometry).toEqual({ withinCard: true, noPageOverflow: true });
});

test("All Resources is visually separated from the resource grid", async ({
  page,
  bulkExport,
}) => {
  for (const viewport of [
    { width: 1280, height: 800 },
    { width: 390, height: 844 },
  ]) {
    await page.setViewportSize(viewport);
    await bulkExport.goto();

    const allResourcesBottom = await bulkExport.allResources.evaluate(
      (input) => input.closest("label")!.getBoundingClientRect().bottom,
    );
    const firstResourceTop = await bulkExport.typeCheckboxes.first().evaluate(
      (input) => input.closest("label")!.getBoundingClientRect().top,
    );

    // 14px of visible air: 8px margin + 3px container padding + 3px item padding,
    // and the item's own 3px padding sits inside its label box (#1758).
    expect(Math.abs(firstResourceTop - allResourcesBottom - 11)).toBeLessThanOrEqual(0.5);
  }
});

async function typegridBoxes(page: import("@playwright/test").Page) {
  return page.locator(".typegrid > label.typegrid__item").evaluateAll((items) =>
    items.map((item) => {
      const rect = item.getBoundingClientRect();
      return { x: rect.x, y: rect.y };
    }),
  );
}

test("resource types flow down the first column", async ({ page, bulkExport }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await bulkExport.goto();
  const boxes = await typegridBoxes(page);
  expect(boxes.length).toBeGreaterThan(2);
  expect(Math.abs(boxes[1].x - boxes[0].x)).toBeLessThanOrEqual(1);
  expect(boxes[1].y).toBeGreaterThan(boxes[0].y);
});

test("resource types never run across rows within a column", async ({ page, bulkExport }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await bulkExport.goto();
  const boxes = await typegridBoxes(page);
  expect(new Set(boxes.map((box) => Math.round(box.x))).size).toBeGreaterThan(1);
  for (let i = 0; i + 1 < boxes.length; i++) {
    const a = boxes[i];
    const b = boxes[i + 1];
    const sameColumnBelow = Math.abs(b.x - a.x) <= 1 && b.y > a.y;
    const nextColumnNotLower = b.x > a.x + 1 && b.y <= a.y;
    expect(sameColumnBelow || nextColumnNotLower, `item ${i} -> ${i + 1}`).toBe(true);
  }
});

test("resource types use a single column without horizontal scroll on mobile", async ({
  page,
  bulkExport,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await bulkExport.goto();
  const boxes = await typegridBoxes(page);
  expect(boxes.length).toBeGreaterThan(2);
  for (const box of boxes) {
    expect(Math.abs(box.x - boxes[0].x)).toBeLessThanOrEqual(1);
  }
  const noOverflow = await page.evaluate(
    () => document.documentElement.scrollWidth <= window.innerWidth,
  );
  expect(noOverflow).toBe(true);
});

test("long resource names stay in their grid cell and reveal the full name", async ({
  page,
  bulkExport,
}) => {
  await page.setViewportSize({ width: 1120, height: 900 });
  await bulkExport.goto();

  const resourceType = "MedicinalProductContraindication";
  const label = bulkExport.typeLabel(resourceType);
  const checkbox = bulkExport.typeCheckbox(resourceType);

  await expect(label).toHaveCSS("overflow", "hidden");
  await expect(label).toHaveCSS("text-overflow", "ellipsis");
  await expect(label).toHaveCSS("white-space", "nowrap");
  const geometry = await label.evaluate((element) => {
    const labelRect = element.getBoundingClientRect();
    const item = element.closest(".typegrid__item")!;
    const itemRect = item.getBoundingClientRect();
    const peers = Array.from(item.parentElement!.children)
      .filter((peer) => peer !== item)
      .map((peer) => peer.getBoundingClientRect())
      .filter(
        (peerRect) => peerRect.top < itemRect.bottom && peerRect.bottom > itemRect.top,
      );
    return {
      scrollWidth: element.scrollWidth,
      clientWidth: element.clientWidth,
      contained:
        labelRect.left >= itemRect.left - 1 && labelRect.right <= itemRect.right + 1,
      overlapsPeer: peers.some(
        (peerRect) => labelRect.left < peerRect.right && labelRect.right > peerRect.left,
      ),
    };
  });
  expect(geometry.scrollWidth).toBeGreaterThan(geometry.clientWidth + 1);
  expect(geometry.contained).toBe(true);
  expect(geometry.overlapsPeer).toBe(false);

  await label.hover();
  await expect(bulkExport.typeTooltip).toBeVisible();
  await expect(bulkExport.typeTooltip).toHaveText(resourceType);

  await bulkExport.allResources.uncheck();
  await page.mouse.move(0, 0);
  await checkbox.evaluate((element) => {
    const inputs = Array.from(
      element.closest(".typegrid")!.querySelectorAll<HTMLInputElement>('input[name="types"]'),
    );
    inputs[inputs.indexOf(element as HTMLInputElement) - 1].focus();
  });
  await page.keyboard.press("Tab");
  await expect(checkbox).toBeFocused();
  await expect(checkbox).toHaveAttribute("aria-describedby", "filter-rail-tooltip");
  await expect(bulkExport.typeTooltip).toBeVisible();
  await expect(bulkExport.typeTooltip).toHaveText(resourceType);

  const hoveredResourceType = "MedicinalProductUndesirableEffect";
  const hoveredLabel = bulkExport.typeLabel(hoveredResourceType);
  expect(
    await hoveredLabel.evaluate((element) => element.scrollWidth > element.clientWidth + 1),
  ).toBe(true);
  await hoveredLabel.hover();
  await expect(bulkExport.typeTooltip).toHaveText(hoveredResourceType);

  await page.mouse.move(0, 0);
  await expect(checkbox).toHaveAttribute("aria-describedby", "filter-rail-tooltip");
  await expect(bulkExport.typeTooltip).toHaveText(resourceType);

  await bulkExport.allResources.focus();
  const patientLabel = bulkExport.typeLabel("Patient");
  expect(
    await patientLabel.evaluate((element) => element.scrollWidth <= element.clientWidth + 1),
  ).toBe(true);
  await patientLabel.hover();
  await expect(bulkExport.typeTooltip).toBeHidden();
  await expect(bulkExport.typeCheckbox("Patient")).not.toHaveAttribute("aria-describedby", /.+/);
});

test("fractional clipping still reveals ImmunizationRecommendation", async ({
  page,
  bulkExport,
}) => {
  await page.setViewportSize({ width: 1120, height: 900 });
  await bulkExport.goto();

  const resourceType = "ImmunizationRecommendation";
  const label = bulkExport.typeLabel(resourceType);
  const geometry = await label.evaluate((element) => {
    const measureText = () => {
      const range = document.createRange();
      range.selectNodeContents(element);
      return range.getBoundingClientRect().width;
    };
    const textWidth = measureText();

    // Derive a sub-pixel box from the rendered text instead of relying on a
    // particular viewport rounding. Find a width where the integer DOM
    // metrics tie even though fractional geometry proves the text is clipped.
    for (let delta = 0.03125; delta < 0.5; delta += 0.03125) {
      const width = textWidth - delta;
      element.style.flex = `0 0 ${width}px`;
      element.style.width = `${width}px`;
      const boxWidth = element.getBoundingClientRect().width;
      const currentTextWidth = measureText();
      if (
        element.scrollWidth === element.clientWidth &&
        currentTextWidth > boxWidth + 0.01
      ) {
        return {
          boxWidth,
          textWidth: currentTextWidth,
          scrollWidth: element.scrollWidth,
          clientWidth: element.clientWidth,
        };
      }
    }
    throw new Error("could not create a fractionally clipped label");
  });

  expect(geometry.scrollWidth).toBe(geometry.clientWidth);
  expect(geometry.textWidth).toBeGreaterThan(geometry.boxWidth);
  await label.hover();
  await expect(bulkExport.typeTooltip).toBeVisible();
  await expect(bulkExport.typeTooltip).toHaveText(resourceType);
});

test("Start Export reveals both inline errors and enables reactive validation", async ({
  page,
  bulkExport,
}) => {
  await bulkExport.goto();
  let submissions = 0;
  page.on("request", (request) => {
    if (request.url().endsWith("/ui/bulk-export") && request.method() === "POST") {
      submissions += 1;
    }
  });

  await expect(bulkExport.nameInput).toHaveAttribute("required", "");
  await expect(bulkExport.form).toHaveAttribute("novalidate", "");
  await bulkExport.nameInput.fill("   ");
  await bulkExport.sincePreset.selectOption("custom");
  await bulkExport.sinceCustom.fill("not-an-instant");
  await bulkExport.sinceCustom.press("Tab");

  await expect(bulkExport.nameError).toBeHidden();
  await expect(bulkExport.sinceCustomError).toBeHidden();
  await expect(bulkExport.nameInput).not.toHaveAttribute("aria-invalid", /.+/);
  await expect(bulkExport.sinceCustom).not.toHaveAttribute("aria-invalid", /.+/);

  await bulkExport.startButton.click();

  await expect(bulkExport.nameInput).toBeFocused();
  await expect(bulkExport.nameInput).toHaveAttribute("aria-invalid", "true");
  await expect(bulkExport.nameInput).toHaveAttribute(
    "aria-describedby",
    "bulk-export-name-error",
  );
  await expect(bulkExport.nameError).toBeVisible();
  await expect(bulkExport.nameError).toHaveText("Enter a name for this export.");
  await expect(bulkExport.sinceCustom).toHaveAttribute("aria-invalid", "true");
  await expect(bulkExport.sinceCustom).toHaveAttribute(
    "aria-describedby",
    "bulk-export-since-custom-error",
  );
  await expect(bulkExport.sinceCustomError).toBeVisible();
  await expect(bulkExport.sinceCustomError).toHaveText(
    "Enter a valid FHIR instant, such as 2026-08-01T00:00:00Z.",
  );
  expect(submissions).toBe(0);

  const errorColors = await bulkExport.nameInput.evaluate((input) => {
    const error = document.querySelector("#bulk-export-name-error")!;
    return {
      border: getComputedStyle(input).borderColor,
      text: getComputedStyle(error).color,
    };
  });
  expect(errorColors.border).toBe(errorColors.text);

  await bulkExport.nameInput.fill("Reactive export");
  await expect(bulkExport.nameError).toBeHidden();
  await expect(bulkExport.nameInput).not.toHaveAttribute("aria-invalid", /.+/);
  await bulkExport.nameInput.fill("");
  await expect(bulkExport.nameError).toBeVisible();

  await bulkExport.sinceCustom.fill("  2026-08-01T03:30:00.123+03:30  ");
  await expect(bulkExport.sinceCustomError).toBeHidden();
  await expect(bulkExport.sinceCustom).not.toHaveAttribute("aria-invalid", /.+/);
  await bulkExport.sinceCustom.fill("invalid-again");
  await expect(bulkExport.sinceCustomError).toBeVisible();

  for (const instant of [
    "2026-13-01T00:00:00Z",
    "2026-04-31T00:00:00Z",
    "2026-08-01T24:00:00Z",
    "2026-08-01T00:00:00+24:00",
  ]) {
    await bulkExport.sinceCustom.fill(instant);
    await expect(bulkExport.sinceCustomError).toBeVisible();
  }

  await bulkExport.sincePreset.selectOption("week");
  await expect(bulkExport.sinceCustom).toBeDisabled();
  await expect(bulkExport.sinceCustomError).toBeHidden();
  await bulkExport.sincePreset.selectOption("custom");
  await expect(bulkExport.sinceCustom).toBeEnabled();
  await expect(bulkExport.sinceCustomError).toBeVisible();

  await bulkExport.nameInput.fill("Reactive export");
  await bulkExport.sinceCustom.fill("   ");
  await expect(bulkExport.nameError).toBeHidden();
  await expect(bulkExport.sinceCustomError).toBeHidden();

  await page.route("**/ui/bulk-export", (route) =>
    route.request().method() === "POST"
      ? route.fulfill({ status: 204 })
      : route.continue(),
  );
  const submitted = page.waitForRequest(
    (request) => request.url().endsWith("/ui/bulk-export") && request.method() === "POST",
  );
  await bulkExport.startButton.click();
  await submitted;
  expect(submissions).toBe(1);
});

test("Custom instant follows the Since preset and form serialization", async ({ bulkExport }) => {
  await bulkExport.goto();
  await bulkExport.nameInput.fill("Preset validation");

  for (const preset of ["", "day", "week", "month"]) {
    await bulkExport.sincePreset.selectOption(preset);
    await expect(bulkExport.sinceCustom).toBeDisabled();
    await expect(bulkExport.sinceCustom).not.toHaveAttribute("pattern", /.+/);
  }

  const instant = "2026-08-01T00:00:00Z";
  await bulkExport.sincePreset.selectOption("custom");
  await expect(bulkExport.sinceCustom).toBeEnabled();
  await expect(bulkExport.sinceCustom).not.toHaveAttribute("pattern", /.+/);
  const customPattern = await bulkExport.sinceCustom.getAttribute("data-pattern");
  expect(customPattern).not.toBeNull();

  await bulkExport.sinceCustom.fill("   ");
  await expect(bulkExport.sinceCustom).not.toHaveAttribute("pattern", /.+/);

  const paddedInstant = `  ${instant}  `;
  await bulkExport.sinceCustom.fill(paddedInstant);
  await expect(bulkExport.sinceCustom).not.toHaveAttribute("pattern", /.+/);

  await bulkExport.sinceCustom.fill(instant);
  await expect(bulkExport.sinceCustom).not.toHaveAttribute("pattern", /.+/);

  await bulkExport.sincePreset.selectOption("week");
  await expect(bulkExport.sinceCustom).toBeDisabled();
  await expect(bulkExport.sinceCustom).toHaveValue(instant);
  expect(
    await bulkExport.form.evaluate(
      (form) => new FormData(form as HTMLFormElement).has("since_custom"),
    ),
  ).toBe(false);

  await bulkExport.sincePreset.selectOption("custom");
  await expect(bulkExport.sinceCustom).toBeEnabled();
  await expect(bulkExport.sinceCustom).toHaveValue(instant);
  expect(
    await bulkExport.form.evaluate(
      (form) => new FormData(form as HTMLFormElement).get("since_custom"),
    ),
  ).toBe(instant);

  await bulkExport.sinceCustom.fill("not-an-instant");
  expect(
    await bulkExport.sinceCustom.evaluate(
      (input) => (input as HTMLInputElement).validity.patternMismatch,
    ),
  ).toBe(false);

  await bulkExport.sincePreset.selectOption("week");
  await expect(bulkExport.sinceCustom).toBeDisabled();
  await expect(bulkExport.sinceCustom).not.toHaveAttribute("pattern", /.+/);
  await expect(bulkExport.sinceCustom).toHaveValue("not-an-instant");
});

test("FHIR R4 leap second and timezone offset boundaries validate reactively", async ({
  page,
  bulkExport,
}) => {
  await bulkExport.goto();
  await bulkExport.nameInput.fill("FHIR R4 instant boundaries");
  await bulkExport.sincePreset.selectOption("custom");
  let submissions = 0;
  page.on("request", (request) => {
    if (request.url().endsWith("/ui/bulk-export") && request.method() === "POST") {
      submissions += 1;
    }
  });

  await bulkExport.sinceCustom.fill("not-an-instant");
  await bulkExport.startButton.click();
  await expect(bulkExport.sinceCustomError).toBeVisible();
  expect(submissions).toBe(0);

  for (const instant of ["2026-12-31T23:59:60Z", "2026-08-01T00:00:00+14:00"]) {
    await bulkExport.sinceCustom.fill(instant);
    await expect(bulkExport.sinceCustomError).toBeHidden();
  }

  for (const instant of [
    "0000-08-01T00:00:00Z",
    "2026-08-01T00:00:00+14:01",
    "2026-08-01T00:00:00+15:00",
  ]) {
    await bulkExport.sinceCustom.fill(instant);
    await expect(bulkExport.sinceCustomError).toBeVisible();
    await bulkExport.startButton.click();
    expect(submissions).toBe(0);
  }
});

test("keyboard submit starts inline validation", async ({ bulkExport }) => {
  await bulkExport.goto();
  await bulkExport.sincePreset.selectOption("custom");
  await bulkExport.sinceCustom.fill("not-an-instant");
  await bulkExport.nameInput.focus();
  await bulkExport.nameInput.press("Enter");

  await expect(bulkExport.nameError).toBeVisible();
  await expect(bulkExport.sinceCustomError).toBeVisible();
  await expect(bulkExport.nameInput).toBeFocused();
});

test("inactive malformed Custom does not block submission", async ({ page, bulkExport }) => {
  await bulkExport.goto();
  await bulkExport.nameInput.fill("Inactive custom value");
  await bulkExport.sincePreset.selectOption("custom");
  await bulkExport.sinceCustom.fill("not-an-instant");
  await bulkExport.sincePreset.selectOption("week");

  await page.route("**/ui/bulk-export", (route) =>
    route.request().method() === "POST"
      ? route.fulfill({ status: 204 })
      : route.continue(),
  );
  const submitted = page.waitForRequest(
    (request) => request.url().endsWith("/ui/bulk-export") && request.method() === "POST",
  );
  await bulkExport.startButton.click();

  const params = new URLSearchParams((await submitted).postData() ?? "");
  expect(params.get("since_preset")).toBe("week");
  expect(params.has("since_custom")).toBe(false);
});

test("a patient-only server rejection starts reactive field validation", async ({
  page,
  bulkExport,
}) => {
  const exportName = "Patient-only rejection";
  await bulkExport.goto();
  await bulkExport.nameInput.fill(exportName);
  await bulkExport.scopeRadio("patient").check();
  await bulkExport.sincePreset.selectOption("custom");
  await bulkExport.sinceCustom.fill("2026-08-01T00:00:00Z");
  // Inject the malformed value as a combobox chip (`data-combobox-selected-input`)
  // rather than a bare form field: T2's inline validation now blocks an empty
  // Patients selection before submit, so the client must see a selection here
  // for this test to reach the server-side format rejection it exercises.
  await bulkExport.patientCombobox.evaluate((fieldset) => {
    const patient = document.createElement("input");
    patient.type = "hidden";
    patient.name = "patient";
    patient.value = "Patient/not/valid";
    patient.setAttribute("data-combobox-selected-input", "");
    fieldset.append(patient);
  });

  const submitted = page.waitForResponse(
    (response) =>
      response.url().endsWith("/ui/bulk-export") &&
      response.request().method() === "POST",
  );
  await bulkExport.startButton.click();
  expect((await submitted).status()).toBe(400);

  await expect(bulkExport.form).toHaveAttribute("data-validation-started", "true");
  await expect(page.locator(".notice")).toContainText("valid logical Patient IDs");
  // The attribute only arms the reactive validation; the deferred
  // bulk-export.js is what acts on it, and everything asserted above is server
  // markup that is already there while the scripts are still loading. Wait for
  // the enhancement itself: the server renders the static page title in the
  // heading, and the name reaches it in the same synchronous pass that binds
  // the input listeners the edits below depend on.
  await expect(bulkExport.nameHeading).toHaveText(exportName);
  await expect(bulkExport.nameError).toBeHidden();
  await expect(bulkExport.sinceCustomError).toBeHidden();

  await bulkExport.nameInput.fill("");
  await expect(bulkExport.nameError).toBeVisible();
  await bulkExport.sinceCustom.fill("not-an-instant");
  await expect(bulkExport.sinceCustomError).toBeVisible();
});

test("server rejects an impossible Custom date without creating an export", async ({
  page,
  bulkExport,
}) => {
  const exportName = "Browser impossible date must not start";
  await page.route("**/ui/lookup/patient-options*", (route) =>
    route.fulfill({ status: 200, contentType: "text/html", body: patientOptions }),
  );
  await bulkExport.goto();
  await bulkExport.nameInput.fill(exportName);
  await bulkExport.scopeRadio("group").check();
  await bulkExport.form.locator('input[name="group_id"]').fill("preserved-group");
  await bulkExport.scopeRadio("patient").check();
  await bulkExport.patientSearch.fill("an");
  await bulkExport.patientListbox.locator('[data-value="Patient/p-104"]').click();
  await bulkExport.patientSearch.press("ArrowDown");
  await bulkExport.patientListbox.locator('[data-value="Patient/p-205"]').click();
  await bulkExport.patientSearch.press("Escape");
  await bulkExport.allResources.uncheck();
  await bulkExport.typeCheckbox("Patient").check();
  await bulkExport.typeCheckbox("Observation").check();
  await bulkExport.form.locator('input[name="elements"]').fill("id,meta");
  await bulkExport.form
    .locator('input[name="type_filter"]')
    .fill("Patient?active=true");
  await bulkExport.sincePreset.selectOption("custom");
  await bulkExport.sinceCustom.fill("2026-02-31T00:00:00Z");
  await bulkExport.until.fill("2099-01-01T00:00:00Z");
  await expect(bulkExport.sinceCustomError).toBeHidden();

  const submitted = page.waitForResponse(
    (response) =>
      response.url().endsWith("/ui/bulk-export") &&
      response.request().method() === "POST",
  );
  // Bypass the enhanced submit handler to exercise the authoritative server
  // response exactly as a no-JavaScript or hostile client can.
  await bulkExport.form.evaluate((form) => (form as HTMLFormElement).submit());
  expect((await submitted).status()).toBe(400);

  await expect(page).toHaveURL(/\/ui\/bulk-export$/);
  await expect(bulkExport.nameInput).toHaveValue(exportName);
  await expect(bulkExport.nameHeading).toHaveText(exportName);
  await expect(bulkExport.nameError).toBeHidden();
  await expect(bulkExport.scopeRadio("patient")).toBeChecked();
  await expect(bulkExport.selectedPatients).toHaveCount(2);
  expect(await bulkExport.selectedPatients.evaluateAll((patients) =>
    patients.map((patient) => (patient as HTMLInputElement).value),
  )).toEqual(["Patient/p-104", "Patient/p-205"]);
  await expect(bulkExport.form.locator('input[name="group_id"]')).toHaveValue(
    "preserved-group",
  );
  await expect(bulkExport.allResources).not.toBeChecked();
  await expect(bulkExport.typeCheckbox("Patient")).toBeChecked();
  await expect(bulkExport.typeCheckbox("Observation")).toBeChecked();
  await expect(bulkExport.form.locator('input[name="elements"]')).toHaveValue("id,meta");
  await expect(bulkExport.form.locator('input[name="type_filter"]')).toHaveValue(
    "Patient?active=true",
  );
  await expect(bulkExport.sincePreset).toHaveValue("custom");
  await expect(bulkExport.sinceCustom).toHaveValue("2026-02-31T00:00:00Z");
  await expect(bulkExport.sinceCustom).toHaveAttribute("aria-invalid", "true");
  await expect(bulkExport.sinceCustom).toHaveAttribute(
    "aria-describedby",
    "bulk-export-since-custom-error",
  );
  await expect(bulkExport.sinceCustomError).toHaveText(
    "Enter a valid FHIR instant, such as 2026-08-01T00:00:00Z.",
  );
  await expect(bulkExport.sinceCustom).toBeFocused();
  const rejectedState = await nonTypeState(bulkExport.form);
  await bulkExport.clearButton.click();
  await expect(page).toHaveURL(/\/ui\/bulk-export$/);
  expect(await nonTypeState(bulkExport.form)).toEqual(rejectedState);
  await expect(bulkExport.allResources).not.toBeChecked();
  expect(await bulkExport.typeCheckboxes.evaluateAll((types) => types.every((type) =>
    !(type as HTMLInputElement).checked && !(type as HTMLInputElement).disabled,
  ))).toBe(true);
  await expect(bulkExport.sinceCustomError).toBeVisible();

  await bulkExport.sinceCustom.fill("2025-02-29T00:00:00Z");
  await expect(bulkExport.sinceCustomError).toBeVisible();
  await expect(bulkExport.sinceCustom).toHaveAttribute("aria-invalid", "true");
  await bulkExport.sinceCustom.fill("2026-08-01T00:00:00Z");
  await expect(bulkExport.sinceCustomError).toBeHidden();
  await expect(bulkExport.sinceCustom).not.toHaveAttribute("aria-invalid", /.+/);
  await bulkExport.sinceCustom.fill("not-an-instant");
  await expect(bulkExport.sinceCustomError).toBeVisible();

  const { violations } = await new AxeBuilder({ page }).analyze();
  expect(violations, axeSummary(violations)).toEqual([]);

  await page.goto("/ui/bulk-export");
  await expect(page.locator(".job-card").filter({ hasText: exportName })).toHaveCount(0);
});

// #1271: Until used to accept any text, so a typo created a job that only
// failed at kick-off. It now validates inline, independent of the Since preset.
test("Start Export rejects a malformed Until inline without submitting", async ({
  page,
  bulkExport,
}) => {
  await bulkExport.goto();
  let submissions = 0;
  page.on("request", (request) => {
    if (request.url().endsWith("/ui/bulk-export") && request.method() === "POST") {
      submissions += 1;
    }
  });

  await expect(bulkExport.until).toHaveAttribute("data-pattern", /\[1-9\]000\)-/);
  await expect(bulkExport.until).not.toHaveAttribute("pattern", /.+/);
  await expect(bulkExport.until).toHaveAttribute("aria-describedby", "bulk-export-until-hint");
  await bulkExport.nameInput.fill("Until typo must not start");
  await expect(bulkExport.sincePreset).toHaveValue("");
  await bulkExport.until.fill("026-09-17T10:38:49Z");
  await bulkExport.until.press("Tab");
  await expect(bulkExport.untilError).toBeHidden();
  await expect(bulkExport.until).not.toHaveAttribute("aria-invalid", /.+/);

  await bulkExport.startButton.click();

  await expect(bulkExport.until).toBeFocused();
  await expect(bulkExport.until).toHaveAttribute("aria-invalid", "true");
  await expect(bulkExport.until).toHaveAttribute(
    "aria-describedby",
    "bulk-export-until-hint bulk-export-until-error",
  );
  await expect(bulkExport.untilError).toBeVisible();
  await expect(bulkExport.untilError).toHaveText(
    "Enter a valid FHIR instant, such as 2026-08-01T00:00:00Z.",
  );
  await expect(bulkExport.nameError).toBeHidden();
  await expect(bulkExport.sinceCustomError).toBeHidden();
  expect(submissions).toBe(0);

  await bulkExport.until.fill("2026-09-17T10:38:49Z");
  await expect(bulkExport.untilError).toBeHidden();
  await expect(bulkExport.until).not.toHaveAttribute("aria-invalid", /.+/);
  await expect(bulkExport.until).toHaveAttribute("aria-describedby", "bulk-export-until-hint");

  for (const instant of ["2026-02-31T00:00:00Z", "2026-08-01T24:00:00Z", "not-an-instant"]) {
    await bulkExport.until.fill(instant);
    await expect(bulkExport.untilError).toBeVisible();
  }

  await bulkExport.until.fill("   ");
  await expect(bulkExport.untilError).toBeHidden();

  await page.route("**/ui/bulk-export", (route) =>
    route.request().method() === "POST"
      ? route.fulfill({ status: 204 })
      : route.continue(),
  );
  const submitted = page.waitForRequest(
    (request) => request.url().endsWith("/ui/bulk-export") && request.method() === "POST",
  );
  await bulkExport.startButton.click();
  await submitted;
  expect(submissions).toBe(1);
});

test("server rejects a malformed Until without creating an export", async ({
  page,
  bulkExport,
}) => {
  const exportName = "Browser malformed Until must not start";
  await bulkExport.goto();
  await bulkExport.nameInput.fill(exportName);
  await bulkExport.allResources.uncheck();
  await bulkExport.typeCheckbox("Patient").check();
  await bulkExport.until.fill("026-09-17T10:38:49Z");

  const submitted = page.waitForResponse(
    (response) =>
      response.url().endsWith("/ui/bulk-export") &&
      response.request().method() === "POST",
  );
  // Bypass the enhanced submit handler, as a no-JavaScript client would.
  await bulkExport.form.evaluate((form) => (form as HTMLFormElement).submit());
  expect((await submitted).status()).toBe(400);

  await expect(page).toHaveURL(/\/ui\/bulk-export$/);
  await expect(bulkExport.nameInput).toHaveValue(exportName);
  await expect(bulkExport.typeCheckbox("Patient")).toBeChecked();
  await expect(bulkExport.until).toHaveValue("026-09-17T10:38:49Z");
  await expect(bulkExport.until).toHaveAttribute("aria-invalid", "true");
  await expect(bulkExport.until).toHaveAttribute(
    "aria-describedby",
    "bulk-export-until-hint bulk-export-until-error",
  );
  await expect(bulkExport.untilError).toBeVisible();
  await expect(bulkExport.untilError).toHaveText(
    "Enter a valid FHIR instant, such as 2026-08-01T00:00:00Z.",
  );
  await expect(bulkExport.until).toBeFocused();
  await expect(bulkExport.sinceCustomError).toBeHidden();

  await bulkExport.until.fill("2026-09-17T10:38:49Z");
  await expect(bulkExport.untilError).toBeHidden();
  await expect(bulkExport.until).not.toHaveAttribute("aria-invalid", /.+/);

  const { violations } = await new AxeBuilder({ page }).analyze();
  expect(violations, axeSummary(violations)).toEqual([]);

  await page.goto("/ui/bulk-export");
  await expect(page.locator(".job-card").filter({ hasText: exportName })).toHaveCount(0);
});

test("Until earlier than Since is rejected inline and revalidates when Since changes", async ({
  page,
  bulkExport,
}) => {
  await bulkExport.goto();
  let submissions = 0;
  page.on("request", (request) => {
    if (request.url().endsWith("/ui/bulk-export") && request.method() === "POST") {
      submissions += 1;
    }
  });

  await bulkExport.nameInput.fill("Until before Since must not start");
  await bulkExport.sincePreset.selectOption("custom");
  await bulkExport.sinceCustom.fill("2026-09-17T10:38:49Z");
  await bulkExport.until.fill("2026-01-01T00:00:00Z");
  await bulkExport.startButton.click();

  await expect(bulkExport.until).toBeFocused();
  await expect(bulkExport.until).toHaveAttribute("aria-invalid", "true");
  await expect(bulkExport.untilError).toHaveText("Until must not be earlier than Since.");
  await expect(bulkExport.sinceCustomError).toBeHidden();
  expect(submissions).toBe(0);

  // Moving Since back clears the error without touching Until.
  await bulkExport.sinceCustom.fill("2025-12-01T00:00:00Z");
  await expect(bulkExport.untilError).toBeHidden();
  await expect(bulkExport.until).not.toHaveAttribute("aria-invalid", /.+/);

  // A preset resolves to a recent instant, so an old Until is rejected again.
  await bulkExport.sincePreset.selectOption("day");
  await expect(bulkExport.untilError).toHaveText("Until must not be earlier than Since.");

  // A malformed Until still reports the format message, not the order one.
  await bulkExport.until.fill("026-09-17T10:38:49Z");
  await expect(bulkExport.untilError).toHaveText(
    "Enter a valid FHIR instant, such as 2026-08-01T00:00:00Z.",
  );

  await bulkExport.sincePreset.selectOption("");
  await bulkExport.until.fill("2026-01-01T00:00:00Z");
  await expect(bulkExport.untilError).toBeHidden();
});

test("Patient combobox supports keyboard selection, dedupe, removal, and scope serialization", async ({
  page,
  bulkExport,
}) => {
  const queries: string[] = [];
  await page.route("**/ui/lookup/patient-options*", (route) => {
    queries.push(new URLSearchParams(route.request().postData() ?? "").get("q") ?? "");
    return route.fulfill({ status: 200, contentType: "text/html", body: patientOptions });
  });
  await bulkExport.goto();

  await expect(bulkExport.patientCombobox).toBeHidden();
  await expect(bulkExport.patientFallback).toBeDisabled();
  await bulkExport.scopeRadio("patient").check();
  await expect(bulkExport.patientCombobox).toBeVisible();
  await expect(bulkExport.patientSearch).toBeEnabled();
  await expect(bulkExport.patientSearch).toHaveAttribute("aria-expanded", "false");

  await bulkExport.patientSearch.fill("an");
  await expect(bulkExport.patientListbox).toBeVisible();
  await expect(bulkExport.patientListbox.getByRole("option")).toHaveCount(2);
  expect(queries).toEqual(["an"]);

  await bulkExport.patientSearch.press("End");
  await expect(bulkExport.patientSearch).toHaveAttribute("aria-activedescendant", /option-1$/);
  await bulkExport.patientSearch.press("Home");
  await expect(bulkExport.patientSearch).toHaveAttribute("aria-activedescendant", /option-0$/);
  await bulkExport.patientSearch.press("Escape");
  await expect(bulkExport.patientListbox).toBeHidden();
  await bulkExport.patientSearch.press("ArrowDown");
  await bulkExport.patientSearch.press("Enter");

  await expect(bulkExport.selectedPatients).toHaveCount(1);
  await expect(bulkExport.selectedPatients).toHaveValue("Patient/p-104");
  await expect(bulkExport.patientCombobox.getByText("Ana Rivera", { exact: true })).toBeVisible();
  await expect(bulkExport.patientCombobox.getByRole("option", { includeHidden: true }).first()).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await expect(bulkExport.patientCombobox.locator("[data-combobox-status]")).toContainText(
    "Added Ana Rivera",
  );
  expect(
    await bulkExport.form.evaluate((form) =>
      new FormData(form as HTMLFormElement).getAll("patient"),
    ),
  ).toEqual(["Patient/p-104"]);
  expect(
    await bulkExport.form.evaluate((form) => new FormData(form as HTMLFormElement).has("q")),
  ).toBe(false);

  await bulkExport.patientSearch.fill("an");
  await expect(bulkExport.patientListbox).toBeVisible();
  await bulkExport.patientListbox.getByRole("option").first().click();
  await expect(bulkExport.selectedPatients).toHaveCount(1);
  await expect(bulkExport.patientListbox).toBeHidden();
  await expect(bulkExport.patientSearch).toHaveValue("");
  await expect(bulkExport.patientSearch).toBeFocused();

  await bulkExport.scopeRadio("system").check();
  await expect(bulkExport.patientCombobox).toBeHidden();
  await expect(bulkExport.selectedPatients).toBeDisabled();
  expect(
    await bulkExport.form.evaluate((form) =>
      new FormData(form as HTMLFormElement).has("patient"),
    ),
  ).toBe(false);

  await bulkExport.scopeRadio("patient").check();
  await bulkExport.patientCombobox.getByRole("button", { name: "Remove Ana Rivera" }).click();
  await expect(bulkExport.selectedPatients).toHaveCount(0);
  await expect(bulkExport.patientSearch).toBeFocused();
});

for (const dismissal of ["selection", "keyboard selection", "Escape", "Tab", "outside click", "focus", "pending reference", "scope", "reset"] as const) {
  test(`Patient suggestions stay closed after ${dismissal}, including a delayed response`, async ({ page, bulkExport }) => {
    let release!: () => void;
    let requested!: () => void;
    const pending = new Promise<void>((resolve) => { requested = resolve; });
    const responseGate = new Promise<void>((resolve) => { release = resolve; });
    let holdResponse = false;
    await page.route("**/ui/lookup/patient-options*", async (route) => {
      if (holdResponse) {
        requested();
        await responseGate;
      }
      await route.fulfill({ status: 200, contentType: "text/html", body: patientOptions });
    });
    await bulkExport.goto();
    await bulkExport.scopeRadio("patient").check();
    await bulkExport.patientSearch.fill("an");
    await expect(bulkExport.patientListbox.getByRole("option")).toHaveCount(2);
    holdResponse = true;
    await bulkExport.patientSearch.fill("Patient/p-205");
    await pending;

    if (dismissal === "selection") {
      await bulkExport.patientListbox.getByRole("option").first().click();
    } else if (dismissal === "keyboard selection") {
      await bulkExport.patientSearch.press("ArrowDown");
      await bulkExport.patientSearch.press("Enter");
    } else if (dismissal === "outside click") {
      await bulkExport.nameHeading.click();
    } else if (dismissal === "focus") {
      await bulkExport.nameInput.focus();
    } else if (dismissal === "pending reference") {
      await bulkExport.patientSearch.press("Enter");
    } else if (dismissal === "scope") {
      await bulkExport.scopeRadio("system").check();
    } else if (dismissal === "reset") {
      await bulkExport.form.evaluate((form) => (form as HTMLFormElement).reset());
    } else {
      await bulkExport.patientSearch.press(dismissal);
    }
    const search = bulkExport.patientCombobox.locator('[role="combobox"]');
    const listbox = bulkExport.patientCombobox.locator("[data-combobox-listbox]");
    await expect(listbox).toBeHidden();
    await expect(listbox).toHaveJSProperty("hidden", true);
    await expect(search).toHaveAttribute("aria-expanded", "false");
    await expect(search).not.toHaveAttribute("aria-activedescendant", /.+/);
    if (dismissal.includes("selection")) {
      await expect(search).toHaveValue("");
      await expect(search).toBeFocused();
      await expect(bulkExport.selectedPatients).toHaveValue("Patient/p-104");
    }
    // Wait for the actual htmx swap, not just network completion: late
    // options must still be installed, without revoking the user's close.
    await listbox.evaluate((element) => {
      element.addEventListener("htmx:afterSwap", () => {
        element.setAttribute("data-test-response-swapped", "true");
      }, { once: true });
    });
    release();
    await expect(listbox).toHaveAttribute("data-test-response-swapped", "true");
    await expect(listbox.locator("[data-combobox-option]")).toHaveCount(2);
    await expect(listbox).toBeHidden();
    await expect(listbox).toHaveJSProperty("hidden", true);
    await expect(search).toHaveAttribute("aria-expanded", "false");

    if (dismissal === "scope" || dismissal === "reset") {
      await bulkExport.scopeRadio("patient").check();
    }
    // A fresh user interaction reopens cached suggestions even if the
    // delayed request was dismissed while the input still held focus.
    await bulkExport.patientSearch.press("ArrowDown");
    await expect(listbox).toBeVisible();
  });
}

for (const reopen of ["typing", "focus", "ArrowDown"] as const) {
  test(`Patient suggestions reopen by ${reopen} while the first response is pending`, async ({ page, bulkExport }) => {
    let release!: () => void;
    let requested!: () => void;
    const pending = new Promise<void>((resolve) => { requested = resolve; });
    const responseGate = new Promise<void>((resolve) => { release = resolve; });
    await page.route("**/ui/lookup/patient-options*", async (route) => {
      requested();
      await responseGate;
      await route.fulfill({ status: 200, contentType: "text/html", body: patientOptions });
    });
    await bulkExport.goto();
    await bulkExport.scopeRadio("patient").check();
    await bulkExport.patientSearch.fill("an");
    await pending;
    await bulkExport.patientSearch.press("Escape");
    await expect(bulkExport.patientListbox).toBeHidden();
    if (reopen === "typing") await bulkExport.patientSearch.fill("ana");
    else if (reopen === "focus") {
      await bulkExport.nameInput.focus();
      await bulkExport.patientSearch.focus();
    } else await bulkExport.patientSearch.press("ArrowDown");
    release();
    await expect(bulkExport.patientListbox.getByRole("option")).toHaveCount(2);
    await expect(bulkExport.patientListbox).toBeVisible();
    await expect(bulkExport.patientSearch).toHaveAttribute("aria-expanded", "true");
  });
}

test("Start Export with the Patients scope and an empty Patients field submits every patient", async ({
  page,
  bulkExport,
}) => {
  await bulkExport.goto();
  await bulkExport.nameInput.fill("Patients scope without a selection");
  await bulkExport.scopeRadio("patient").check();
  await expect(bulkExport.patientSearch).toHaveValue("");
  await expect(bulkExport.patientHint).toContainText("Leave empty to export every patient.");

  await page.route("**/ui/bulk-export", (route) =>
    route.request().method() === "POST"
      ? route.fulfill({ status: 204 })
      : route.continue(),
  );
  const submitted = page.waitForRequest(
    (request) => request.url().endsWith("/ui/bulk-export") && request.method() === "POST",
  );
  await bulkExport.startButton.click();
  const params = new URLSearchParams((await submitted).postData() ?? "");
  expect(params.get("scope")).toBe("patient");
  expect(params.getAll("patient")).toEqual([]);
  await expect(bulkExport.patientSearch).not.toHaveAttribute("aria-invalid", /.+/);
});

test("Patient combobox finds and selects a patient by exact identifier", async ({
  request,
  bulkExport,
}) => {
  const identifier = "MRN-829-E2E";
  const patientId = await createResource(request, "Patient", {
    name: [{ given: ["Marisol"], family: "Vega" }],
    identifier: [{ system: "urn:example:mrn", value: identifier }],
  });
  await waitSearchable(request, "Patient", patientId);

  await bulkExport.goto();
  await bulkExport.scopeRadio("patient").check();
  await bulkExport.patientSearch.fill(identifier);
  const option = bulkExport.patientListbox.locator(
    `[data-combobox-option][data-value="Patient/${patientId}"]`,
  );
  await expect(option).toBeVisible();
  await expect(option).toContainText("Marisol Vega");
  await option.click();

  await expect(bulkExport.selectedPatients).toHaveCount(1);
  await expect(bulkExport.selectedPatients).toHaveValue(`Patient/${patientId}`);
});

test("Patient combobox closes on Tab and removing a chip removes selected patients", async ({
  page,
  bulkExport,
}) => {
  await page.route("**/ui/lookup/patient-options*", (route) =>
    route.fulfill({ status: 200, contentType: "text/html", body: patientOptions }),
  );
  await bulkExport.goto();
  await bulkExport.scopeRadio("patient").check();
  await bulkExport.patientSearch.fill("an");
  await expect(bulkExport.patientListbox).toBeVisible();
  await bulkExport.patientSearch.press("Tab");
  await expect(bulkExport.patientListbox).toBeHidden();

  await bulkExport.patientSearch.focus();
  await bulkExport.patientSearch.press("ArrowUp");
  await bulkExport.patientSearch.press("Enter");
  await expect(bulkExport.selectedPatients).toHaveValue("Patient/p-205");
  await bulkExport.patientSearch.press("Escape");
  await bulkExport.patientCombobox.locator('[data-combobox-remove="Patient/p-205"]').click();
  await expect(bulkExport.scopeRadio("patient")).toBeChecked();
  await expect(bulkExport.selectedPatients).toHaveCount(0);
});

test("open Patient combobox has no automated accessibility violations", async ({
  page,
  bulkExport,
}) => {
  await page.route("**/ui/lookup/patient-options*", (route) =>
    route.fulfill({ status: 200, contentType: "text/html", body: patientOptions }),
  );
  await bulkExport.goto();
  await bulkExport.scopeRadio("patient").check();
  await bulkExport.patientSearch.fill("an");
  await expect(bulkExport.patientListbox).toBeVisible();
  await bulkExport.patientSearch.press("ArrowDown");
  await bulkExport.patientSearch.press("Enter");
  await bulkExport.patientSearch.press("ArrowDown");
  await expect(bulkExport.patientListbox).toBeVisible();

  const { violations } = await new AxeBuilder({ page }).analyze();
  expect(violations, axeSummary(violations)).toEqual([]);
});

test("Patient combobox shows empty and error messages and persists a runtime ID-only downgrade", async ({
  page,
  bulkExport,
}) => {
  await page.route("**/ui/lookup/patient-options*", (route) => {
    const query = new URLSearchParams(route.request().postData() ?? "").get("q");
    if (query === "transport") {
      return route.fulfill({ status: 500, contentType: "text/plain", body: "server error" });
    }
    let body = patientMessage("No matching patients found.");
    if (query === "") {
      body = clearPatientMessage;
    } else if (query === "an") {
      body = patientOptions + clearPatientMessage;
    } else if (query === "one") {
      body = patientOptions.split("</button>")[0] + "</button>" + clearPatientMessage;
    } else if (query === "fail") {
      body = patientMessage("Suggestions could not be loaded. Try again.", { error: true });
    } else if (query === "Nobody") {
      body = patientMessage("No matching patients found.", { alternate: true });
    }
    return route.fulfill({ status: 200, contentType: "text/html", body });
  });
  await bulkExport.goto();
  await bulkExport.scopeRadio("patient").check();
  await expect(bulkExport.patientSearch).toHaveAttribute("placeholder", "Search patients");
  await expect(bulkExport.patientHint).toContainText(
    "Search by name, surname or exact identifier.",
  );

  await bulkExport.patientSearch.fill("zz");
  await expect(bulkExport.patientListbox).toBeHidden();
  await expect(bulkExport.patientMessage).toBeVisible();
  await expect(bulkExport.patientMessage).toHaveText("No matching patients found.");
  await expect(bulkExport.patientCombobox.locator("[data-combobox-status]")).toHaveText(
    "No matching patients found.",
  );
  let violations = (await new AxeBuilder({ page }).analyze()).violations;
  expect(violations, axeSummary(violations)).toEqual([]);

  await bulkExport.patientSearch.fill("an");
  await expect(bulkExport.patientListbox).toBeVisible();
  await expect(bulkExport.patientListbox.getByRole("option")).toHaveCount(2);
  await expect(bulkExport.patientMessage).toBeHidden();

  await bulkExport.patientSearch.press("End");
  await expect(bulkExport.patientSearch).toHaveAttribute("aria-activedescendant", /option-1$/);
  await bulkExport.patientSearch.fill("one");
  await expect(bulkExport.patientListbox.getByRole("option")).toHaveCount(1);
  await expect(bulkExport.patientSearch).not.toHaveAttribute("aria-activedescendant", /.+/);
  await bulkExport.patientSearch.press("ArrowDown");
  await expect(bulkExport.patientSearch).toHaveAttribute("aria-activedescendant", /option-0$/);

  await bulkExport.patientSearch.fill("zz");
  await expect(bulkExport.patientListbox).toBeHidden();
  await expect(bulkExport.patientSearch).not.toHaveAttribute("aria-activedescendant", /.+/);
  await bulkExport.patientSearch.fill("");
  await expect(bulkExport.patientMessage).toBeHidden();

  await bulkExport.patientSearch.fill("fail");
  await expect(bulkExport.patientMessage.locator(".field__hint--error")).toBeVisible();
  await expect(bulkExport.patientMessage).toContainText("Suggestions could not be loaded");
  violations = (await new AxeBuilder({ page }).analyze()).violations;
  expect(violations, axeSummary(violations)).toEqual([]);

  await bulkExport.patientSearch.fill("");
  await expect(bulkExport.patientMessage).toBeHidden();

  await bulkExport.patientSearch.fill("transport");
  await expect(bulkExport.patientListbox).toBeHidden();
  await expect(bulkExport.patientMessage.locator(".field__hint--error")).toBeVisible();
  await expect(bulkExport.patientMessage).toContainText("Suggestions could not be loaded");
  await expect(bulkExport.patientCombobox.locator("[data-combobox-status]")).toHaveText(
    "Suggestions could not be loaded. Try again.",
  );
  violations = (await new AxeBuilder({ page }).analyze()).violations;
  expect(violations, axeSummary(violations)).toEqual([]);

  await bulkExport.patientSearch.fill("an");
  await expect(bulkExport.patientListbox).toBeVisible();
  await bulkExport.patientSearch.evaluate((input) =>
    input.dispatchEvent(new CustomEvent("htmx:sendError", { bubbles: true })),
  );
  await expect(bulkExport.patientListbox).toBeHidden();
  await expect(bulkExport.patientMessage.locator(".field__hint--error")).toBeVisible();
  await expect(bulkExport.patientMessage).toContainText("Suggestions could not be loaded");

  await bulkExport.patientSearch.fill("Nobody");
  await expect(bulkExport.patientMessage).toHaveText("No matching patients found.");
  await expect(bulkExport.patientCombobox).toHaveAttribute("data-combobox-mode", "alternate");
  await expect(bulkExport.patientHint).toContainText("exact FHIR ID");
  await expect(bulkExport.patientSearch).toHaveAttribute("placeholder", "Search patients");
  await expect(bulkExport.patientMessage).not.toHaveText(await bulkExport.patientHint.innerText());
  await bulkExport.patientSearch.press("Escape");
  await expect(bulkExport.patientHint).toContainText("exact FHIR ID");
  await expect(bulkExport.patientSearch).toHaveAttribute("placeholder", "Search patients");
  await bulkExport.patientSearch.fill("");
  await expect(bulkExport.patientMessage).toBeHidden();
  await expect(bulkExport.patientCombobox).toHaveAttribute("data-combobox-mode", "alternate");
  await expect(bulkExport.patientHint).toContainText("exact FHIR ID");
  await expect(bulkExport.patientSearch).toHaveAttribute("placeholder", "Search patients");
  violations = (await new AxeBuilder({ page }).analyze()).violations;
  expect(violations, axeSummary(violations)).toEqual([]);
});

test("keyboard narrowing submits exactly the two selected resource types", async ({
  page,
  bulkExport,
}) => {
  await bulkExport.goto();
  await bulkExport.nameInput.fill("Two selected resource types");

  await bulkExport.allResources.focus();
  await bulkExport.allResources.press("Space");
  await expect(bulkExport.allResources).not.toBeChecked();
  expect(
    await bulkExport.typeCheckboxes.evaluateAll((types) =>
      types.every(
        (type) => !(type as HTMLInputElement).checked && !(type as HTMLInputElement).disabled,
      ),
    ),
  ).toBe(true);

  await bulkExport.typeCheckbox("Patient").check();
  await bulkExport.typeCheckbox("Observation").check();

  const expectedBody = await bulkExport.form.evaluate((form) => {
    const body = new URLSearchParams();
    for (const [name, value] of new FormData(form as HTMLFormElement).entries()) {
      body.append(name, String(value));
    }
    return body.toString();
  });
  await page.route("**/ui/bulk-export", (route) =>
    route.request().method() === "POST"
      ? route.fulfill({ status: 204 })
      : route.continue(),
  );
  const submitted = page.waitForRequest(
    (request) => request.url().endsWith("/ui/bulk-export") && request.method() === "POST",
  );
  await bulkExport.startButton.click();

  const request = await submitted;
  expect(request.postData()).toBe(expectedBody);
  const params = new URLSearchParams(request.postData() ?? "");
  expect(params.has("all_types")).toBe(false);
  expect(params.getAll("types").sort()).toEqual(["Observation", "Patient"]);
});

test("critical destination assets are ready before kickoff navigation starts", async ({
  page,
  bulkExport,
}) => {
  await bulkExport.goto();
  await bulkExport.nameInput.fill("Busy state export");
  await page.evaluate(() => {
    type PrefetchState = { links: HTMLLinkElement[]; submitCalls: number };
    type PrefetchWindow = Window & typeof globalThis & { __issue831Prefetch: PrefetchState };
    const state: PrefetchState = { links: [], submitCalls: 0 };
    (window as PrefetchWindow).__issue831Prefetch = state;

    const nativeAppendChild = document.head.appendChild.bind(document.head);
    document.head.appendChild = ((node: Node) => {
      if (node instanceof HTMLLinkElement && node.rel === "prefetch") {
        state.links.push(node);
        return node;
      }
      return nativeAppendChild(node);
    }) as typeof document.head.appendChild;
    HTMLFormElement.prototype.submit = function () {
      state.submitCalls++;
    };
  });

  let exportRequests = 0;
  let markPostReceived!: () => void;
  const postReceived = new Promise<void>((resolve) => {
    markPostReceived = resolve;
  });
  await page.route("**/ui/bulk-export", async (route) => {
    if (route.request().method() !== "POST") return route.continue().catch(() => {});
    exportRequests++;
    markPostReceived();
    await route
      .fulfill({ status: 303, headers: { location: "/ui/bulk-export" } })
      .catch(() => {});
  });

  const startButton = await bulkExport.startButton.elementHandle();
  expect(startButton).not.toBeNull();
  const criticalAssetsReady = page
    .waitForFunction(() => {
      type PrefetchWindow = Window &
        typeof globalThis & { __issue831Prefetch: { links: HTMLLinkElement[] } };
      return (window as PrefetchWindow).__issue831Prefetch.links.length === 2;
    })
    .then(() => "critical-assets" as const);
  await startButton!.click({ noWaitAfter: true });

  expect(
    await Promise.race([criticalAssetsReady, postReceived.then(() => "post" as const)]),
  ).toBe("critical-assets");
  expect(exportRequests).toBe(0);
  expect(
    await page.evaluate(() => {
      type PrefetchWindow = Window &
        typeof globalThis & {
          __issue831Prefetch: { links: HTMLLinkElement[]; submitCalls: number };
        };
      const state = (window as PrefetchWindow).__issue831Prefetch;
      return {
        assets: state.links.map((link) => [new URL(link.href).pathname, link.as]),
        submitCalls: state.submitCalls,
      };
    }),
  ).toEqual({
    assets: [
      ["/ui/assets/app.css", "style"],
      ["/ui/assets/theme.js", "script"],
    ],
    submitCalls: 0,
  });
  expect(
    await startButton!.evaluate((button) => ({
      ariaBusy: button.getAttribute("aria-busy"),
      disabled: button.disabled,
    })),
  ).toEqual({ ariaBusy: "true", disabled: true });

  await page.evaluate(() => {
    type PrefetchWindow = Window &
      typeof globalThis & { __issue831Prefetch: { links: HTMLLinkElement[] } };
    for (const link of (window as PrefetchWindow).__issue831Prefetch.links) {
      link.dispatchEvent(new Event("load"));
    }
  });
  await expect
    .poll(() =>
      page.evaluate(() => {
        type PrefetchWindow = Window &
          typeof globalThis & { __issue831Prefetch: { submitCalls: number } };
        return (window as PrefetchWindow).__issue831Prefetch.submitCalls;
      }),
    )
    .toBe(1);
  expect(exportRequests).toBe(0);
});

test("Start Export stays busy for the kickoff navigation and ignores repeat clicks", async ({
  page,
  bulkExport,
}) => {
  let exportRequests = 0;
  let markRequestReceived!: () => void;
  const requestReceived = new Promise<void>((resolve) => {
    markRequestReceived = resolve;
  });
  let releaseRequest!: () => void;
  const parkedRequest = new Promise<void>((resolve) => {
    releaseRequest = resolve;
  });
  await page.route("**/ui/bulk-export", async (route) => {
    if (route.request().method() !== "POST") return route.continue().catch(() => {});
    exportRequests++;
    markRequestReceived();
    await parkedRequest;
    await route
      .fulfill({ status: 303, headers: { location: "/ui/bulk-export" } })
      .catch(() => {});
  });

  await bulkExport.goto();
  await bulkExport.nameInput.fill("Busy state export");
  const startButton = await bulkExport.startButton.elementHandle();
  expect(startButton).not.toBeNull();
  const stateWhileHeld = startButton!.evaluate(async (button) => {
    await new Promise<void>((resolve) => {
      if (button.getAttribute("aria-busy") === "true") return resolve();
      const observer = new MutationObserver(() => {
        if (button.getAttribute("aria-busy") !== "true") return;
        observer.disconnect();
        resolve();
      });
      observer.observe(button, { attributes: true, attributeFilter: ["aria-busy"] });
    });
    const entered = {
      ariaBusy: button.getAttribute("aria-busy"),
      disabled: button.disabled,
      spinnerContent: getComputedStyle(button, "::after").content,
      spinnerAnimation: getComputedStyle(button, "::after").animationName,
    };
    button.click();
    button.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await new Promise<void>((resolve) =>
      requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
    );
    return {
      entered,
      held: {
        ariaBusy: button.getAttribute("aria-busy"),
        disabled: button.disabled,
      },
    };
  });
  await startButton!.click({ noWaitAfter: true });
  await requestReceived;

  const { entered, held } = await stateWhileHeld;
  expect(entered).toEqual({
    ariaBusy: "true",
    disabled: true,
    spinnerContent: '""',
    spinnerAnimation: "spin",
  });
  expect(exportRequests).toBe(1);
  expect(held).toEqual({ ariaBusy: "true", disabled: true });

  releaseRequest();
  await expect(page).toHaveURL(/\/ui\/bulk-export$/);
});

test("a persisted pageshow clears the restored Start Export busy state", async ({
  page,
  bulkExport,
}) => {
  await bulkExport.goto();
  await bulkExport.nameInput.fill("Busy state export");
  const restingWidth = await bulkExport.startButton.evaluate(
    (button) => button.getBoundingClientRect().width,
  );
  await bulkExport.form.evaluate((form) => {
    HTMLFormElement.prototype.submit = function () {};
    form.dispatchEvent(new SubmitEvent("submit", { bubbles: true, cancelable: true }));
  });

  await expect(bulkExport.startButton).toHaveAttribute("aria-busy", "true");
  await expect(bulkExport.startButton).toBeDisabled();
  // The ring's room is reserved at rest (.btn--busy-slot, #1253 review): the
  // label stays and Start Export keeps its width while busy.
  await expect(bulkExport.startButton).toHaveText("Start Export");
  expect(
    await bulkExport.startButton.evaluate((button) => button.getBoundingClientRect().width),
  ).toBe(restingWidth);
  await page.evaluate(() => {
    window.dispatchEvent(new PageTransitionEvent("pageshow", { persisted: true }));
  });
  await expect(bulkExport.startButton).toBeEnabled();
  expect(await bulkExport.startButton.getAttribute("aria-busy")).toBeNull();
});

test("native validation rejects the form before Start Export becomes busy", async ({
  page,
  bulkExport,
}) => {
  let exportRequests = 0;
  await page.route("**/ui/bulk-export", (route) => {
    if (route.request().method() !== "POST") return route.continue();
    exportRequests++;
    return route.fulfill({ status: 204 });
  });
  await bulkExport.goto();

  const name = bulkExport.form.locator('input[name="name"]');
  await name.evaluate((input: HTMLInputElement) => {
    input.required = true;
  });
  await bulkExport.startButton.click();

  await expect(name).toBeFocused();
  expect(await name.evaluate((input: HTMLInputElement) => input.validity.valid)).toBe(false);
  await expect(bulkExport.startButton).toBeEnabled();
  expect(await bulkExport.startButton.getAttribute("aria-busy")).toBeNull();
  expect(exportRequests).toBe(0);
});

test("re-checking restores All Resources and Clear empties only types", async ({ bulkExport }) => {
  await bulkExport.goto();

  await bulkExport.allResources.uncheck();
  await bulkExport.typeCheckbox("Patient").check();
  await bulkExport.allResources.check();

  await expect(bulkExport.allResources).toBeChecked();
  expect(
    await bulkExport.typeCheckboxes.evaluateAll((types) =>
      types.every((type) => (type as HTMLInputElement).checked && (type as HTMLInputElement).disabled),
    ),
  ).toBe(true);

  await bulkExport.allResources.uncheck();
  expect(
    await bulkExport.typeCheckboxes.evaluateAll((types) =>
      types.every(
        (type) => !(type as HTMLInputElement).checked && !(type as HTMLInputElement).disabled,
      ),
    ),
  ).toBe(true);

  await bulkExport.typeCheckbox("Observation").check();
  await bulkExport.nameInput.fill("temporary name");
  await bulkExport.scopeRadio("patient").check();
  await bulkExport.sincePreset.selectOption("custom");
  await bulkExport.sinceCustom.fill("2026-08-01T00:00:00Z");
  await bulkExport.clearButton.click();

  await expect(bulkExport.nameInput).toHaveValue("temporary name");
  await expect(bulkExport.nameHeading).toHaveText("temporary name");
  await expect(bulkExport.scopeRadio("patient")).toBeChecked();
  await expect(bulkExport.sincePreset).toHaveValue("custom");
  await expect(bulkExport.sinceCustom).toHaveValue("2026-08-01T00:00:00Z");
  await expect(bulkExport.sinceCustom).toBeEnabled();
  await expect(bulkExport.nameError).toBeHidden();
  await expect(bulkExport.sinceCustomError).toBeHidden();
  await expect(bulkExport.allResources).not.toBeChecked();
  expect(
    await bulkExport.typeCheckboxes.evaluateAll((types) =>
      types.every((type) => !(type as HTMLInputElement).checked && !(type as HTMLInputElement).disabled),
    ),
  ).toBe(true);

  await bulkExport.nameInput.fill("   ");
  await bulkExport.sincePreset.selectOption("custom");
  await bulkExport.sinceCustom.fill("not-an-instant");
  await expect(bulkExport.nameError).toBeHidden();
  await expect(bulkExport.sinceCustomError).toBeHidden();
});

// #1583: use the real mounted page and real card responses. Only the jobs'
// stored lifecycle and response timing are controlled; no HTML is fabricated.
test("terminal polls refresh the export summary and slow cards share one request slot", async ({ page, request }) => {
  const settings = await request.get("/_user/settings");
  const previous = (await settings.json()).bulkExport ?? null;
  const jobs = Object.fromEntries(["summary-a", "summary-b"].map((id, index) => [id, {
    name: id, scope: "system", status: "in-progress",
    startedAt: `2026-01-01T09:00:0${index}Z`,
  }]));
  const seeded = await request.patch("/_user/settings", { data: { bulkExport: null } });
  expect(seeded.ok()).toBe(true);
  expect((await request.patch("/_user/settings", { data: { bulkExport: { jobs } } })).ok()).toBe(true);
  let arrivals = 0;
  let inFlight = 0;
  let maxInFlight = 0;
  let markFirst!: () => void;
  const firstArrived = new Promise<void>((resolve) => { markFirst = resolve; });
  let releaseFirst!: () => void;
  const firstGate = new Promise<void>((resolve) => { releaseFirst = resolve; });
  await page.route("**/ui/bulk-export/active/*/card", async (route) => {
    arrivals++;
    inFlight++;
    maxInFlight = Math.max(maxInFlight, inFlight);
    const first = arrivals === 1;
    if (first) markFirst();
    const id = new URL(route.request().url()).pathname.split("/").at(-2)!;
    expect((await request.patch("/_user/settings", { data: { bulkExport: { jobs: {
      [id]: { status: "complete", finishedAt: "2026-01-01T09:00:10Z", files: [{ type: "Patient", url: "ignored" }] },
    } } } })).ok()).toBe(true);
    const response = await route.fetch();
    expect(response.status()).toBe(200);
    if (first) await firstGate;
    await route.fulfill({ response });
    inFlight--;
  });
  try {
    await page.goto("/ui/bulk-export");
    const summary = page.locator("#bulk-export-summary");
    await expect(summary).toHaveText("2 exports · 2 running");
    await expect(page.locator(".job-card[hx-get]")).toHaveCount(2);
    await firstArrived;
    // Hold the first real response across another full 5s tick, making both
    // same-card backlogs and overlapping sibling requests observable.
    await page.waitForTimeout(6_200);
    expect(arrivals).toBe(1);
    expect(maxInFlight).toBe(1);
    await expect(summary).toHaveText("2 exports · 2 running");
    releaseFirst();
    await expect(summary).toHaveText("2 exports · 1 running", { timeout: 5_000 });
    await expect(page.locator(".job-card[hx-get]")).toHaveCount(1);
    await expect(summary).toHaveText("2 exports · 0 running", { timeout: 20_000 });
    await expect(page.locator(".job-card[hx-get]")).toHaveCount(0);
    await expect(page.locator(".job-card .tag--complete")).toHaveCount(2);
    await expect(page.locator(".job-card__meta").filter({ hasText: "1 file" })).toHaveCount(2);
    expect(arrivals, "no queued callback may poll a replaced terminal card").toBe(2);
    expect(maxInFlight).toBe(1);
    await expect(page.locator(".job-card").nth(1)).toHaveCSS("margin-top", "20px");
  } finally {
    releaseFirst();
    await page.unroute("**/ui/bulk-export/active/*/card");
    await request.patch("/_user/settings", { data: { bulkExport: null } });
    await request.patch("/_user/settings", { data: { bulkExport: previous } });
  }
});

test("a failed fragment response leaves polling intact until the terminal summary recovers", async ({ page, request }) => {
  const previous = (await (await request.get("/_user/settings")).json()).bulkExport ?? null;
  await request.patch("/_user/settings", { data: { bulkExport: null } });
  await request.patch("/_user/settings", { data: { bulkExport: { jobs: { recovery: {
    name: "Recovery", status: "in-progress", scope: "system", startedAt: "2026-01-01T09:00:00Z",
  } } } } });
  let arrivals = 0;
  await page.route("**/ui/bulk-export/active/recovery/card", async (route) => {
    arrivals++;
    if (arrivals === 1) return route.fulfill({ status: 503 });
    await request.patch("/_user/settings", { data: { bulkExport: { jobs: { recovery: { status: "cancelled" } } } } });
    await route.fulfill({ response: await route.fetch() });
  });
  try {
    const failed = page.waitForResponse((response) => response.url().endsWith("/recovery/card") && response.status() === 503);
    await page.goto("/ui/bulk-export");
    await failed;
    await expect(page.locator("#bulk-export-summary")).toHaveText("1 export · 1 running");
    await expect(page.locator("#job-recovery")).toHaveAttribute("hx-trigger", "every 5s");
    await expect(page.locator("#bulk-export-summary")).toHaveText("1 export · 0 running", { timeout: 15_000 });
    await expect(page.locator("#job-recovery .tag")).toHaveText("Cancelled");
    await expect(page.locator("#job-recovery")).not.toHaveAttribute("hx-get", /.+/);
    expect(arrivals).toBe(2);
  } finally {
    await page.unroute("**/ui/bulk-export/active/recovery/card");
    await request.patch("/_user/settings", { data: { bulkExport: null } });
    await request.patch("/_user/settings", { data: { bulkExport: previous } });
  }
});

test.describe("pending Bulk Export Patients (#1575)", () => {
  let patientIds: string[];
  let ownJobId: string;
  let existingJobIds: string[];

  test.beforeEach(async ({ page, request, bulkExport }) => {
    patientIds = [];
    ownJobId = "";
    const settings = await (await request.get("/_user/settings")).json();
    existingJobIds = Object.keys(settings.bulkExport?.jobs ?? {});
    await page.route("**/ui/lookup/patient-options*", (route) => route.fulfill({
      status: 200, contentType: "text/html", body: "",
    }));
    await bulkExport.goto();
    await bulkExport.nameInput.fill(`e2e_pending_bulk_${Date.now()}`);
    await bulkExport.scopeRadio("patient").check();
  });

  test.afterEach(async ({ request }) => {
    if (ownJobId) {
      await request.post(`/ui/bulk-export/active/${ownJobId}/cancel`);
      await request.post(`/ui/bulk-export/active/${ownJobId}/delete`);
    }
    await deleteResources(request, "Patient", patientIds);
  });

  test("Enter commits a raw id without submitting, including composition guard (#1575)", async ({ page, bulkExport }) => {
    let submissions = 0;
    page.on("request", (req) => {
      if (req.method() === "POST" && new URL(req.url()).pathname === "/ui/bulk-export") submissions++;
    });
    await bulkExport.patientSearch.fill("pending-1575");
    // A composing Enter is delivered by the IME, not an instruction to add a chip.
    await bulkExport.patientSearch.evaluate((input) => input.dispatchEvent(new KeyboardEvent("keydown", {
      key: "Enter", isComposing: true, bubbles: true, cancelable: true,
    })));
    await expect(bulkExport.selectedPatients).toHaveCount(0);
    await expect(bulkExport.patientSearch).toHaveValue("pending-1575");
    await bulkExport.patientSearch.press("Enter");
    await expect(bulkExport.selectedPatients).toHaveValue("pending-1575");
    await expect(bulkExport.patientSearch).toHaveValue("");
    await expect(page).toHaveURL(/\/ui\/bulk-export\/new$/);
    expect(submissions).toBe(0);
  });

  test("typed then clicked serializes Patients before inline validation and creates a filtered job (#1575)", async ({ page, request, bulkExport }) => {
    const id = await createResource(request, "Patient", { name: [{ family: `PendingBulk${Date.now()}` }] });
    patientIds.push(id);
    await waitSearchable(request, "Patient", id);
    await bulkExport.allResources.uncheck();
    await bulkExport.typeCheckbox("Patient").check();
    await bulkExport.patientSearch.fill(id);
    const name = await bulkExport.nameInput.inputValue();
    const submitted = page.waitForRequest((req) => req.method() === "POST" && new URL(req.url()).pathname === "/ui/bulk-export");
    await bulkExport.startButton.click();
    expect(new URLSearchParams((await submitted).postData() ?? "").getAll("patient")).toEqual([id]);
    await expect(page).toHaveURL(/\/ui\/bulk-export$/);
    const settings = await (await request.get("/_user/settings")).json();
    const ownEntry = Object.entries(settings.bulkExport.jobs).find(([, value]) => (value as { name: string }).name === name)!;
    ownJobId = ownEntry[0];
    expect((ownEntry[1] as { patientRefs: string[] }).patientRefs).toEqual([`Patient/${id}`]);
    const card = page.locator(".job-card").filter({ has: page.locator(".job-card__name", { hasText: name }) });
    await expect(card.locator(".tag")).toHaveText("Complete", { timeout: 30_000 });
  });

  test("invalid pending text is rejected before submission and preserves all other fields (#1575)", async ({ page, request, bulkExport }) => {
    const name = await bulkExport.nameInput.inputValue();
    await bulkExport.sincePreset.selectOption("custom");
    await bulkExport.sinceCustom.fill("2020-01-01T00:00:00Z");
    await bulkExport.allResources.uncheck();
    await bulkExport.typeCheckbox("Patient").check();
    await bulkExport.patientSearch.fill("not a valid id!");
    let submissions = 0;
    await page.route("**/ui/bulk-export", (route) => {
      if (route.request().method() === "POST") submissions++;
      return route.fulfill({ status: 204 });
    });
    await bulkExport.startButton.click();
    await expect(bulkExport.patientCombobox.locator("[data-combobox-validation]")).toHaveText("Enter only valid logical Patient IDs, separated by commas or new lines.");
    await expect(bulkExport.patientSearch).toHaveAttribute("aria-invalid", "true");
    await expect(bulkExport.patientSearch).toHaveValue("not a valid id!");
    expect(submissions).toBe(0);
    await expect(bulkExport.nameInput).toHaveValue(name);
    await expect(bulkExport.scopeRadio("patient")).toBeChecked();
    await expect(bulkExport.typeCheckbox("Patient")).toBeChecked();
    await expect(bulkExport.allResources).not.toBeChecked();
    await expect(bulkExport.sinceCustom).toHaveValue("2020-01-01T00:00:00Z");
    await expect(bulkExport.selectedPatients).toHaveCount(0);
    const settings = await (await request.get("/_user/settings")).json();
    expect(Object.keys(settings.bulkExport?.jobs ?? {})).toEqual(existingJobIds);
  });

  test("real clipboard lists create deduplicated chips before search strips newlines; single paste stays pending (#1575)", async ({ page, bulkExport }) => {
    await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
    for (const separator of [",", "\n", "\r", "\r\n"]) {
      await bulkExport.patientSearch.focus();
      await page.evaluate((text) => navigator.clipboard.writeText(text), `p-1575-a${separator}p-1575-b${separator}p-1575-a`);
      await bulkExport.patientSearch.press("ControlOrMeta+v");
      expect(await bulkExport.selectedPatients.evaluateAll((inputs) => inputs.map((input) => (input as HTMLInputElement).value))).toEqual(["p-1575-a", "p-1575-b"]);
      await expect(bulkExport.patientSearch).toHaveValue("");
      // Clear only affects Resource types; exercise the native form reset.
      await bulkExport.form.evaluate((form: HTMLFormElement) => form.reset());
      await expect(bulkExport.selectedPatients).toHaveCount(0);
      await bulkExport.scopeRadio("patient").check();
    }
    await bulkExport.patientSearch.fill("prefix-REPLACE-suffix");
    await bulkExport.patientSearch.evaluate((input) => (input as HTMLInputElement).setSelectionRange(7, 14));
    await page.evaluate(() => navigator.clipboard.writeText("one,two"));
    await bulkExport.patientSearch.press("ControlOrMeta+v");
    expect(await bulkExport.selectedPatients.evaluateAll((inputs) => inputs.map((input) => (input as HTMLInputElement).value))).toEqual(["prefix-one", "two-suffix"]);
    await bulkExport.form.evaluate((form: HTMLFormElement) => form.reset());
    await expect(bulkExport.selectedPatients).toHaveCount(0);
    await bulkExport.scopeRadio("patient").check();
    await bulkExport.patientSearch.focus();
    await page.evaluate(() => navigator.clipboard.writeText("single-1575"));
    await bulkExport.patientSearch.press("ControlOrMeta+v");
    await expect(bulkExport.selectedPatients).toHaveCount(0);
    await expect(bulkExport.patientSearch).toHaveValue("single-1575");
    await bulkExport.patientSearch.press("Enter");
    await expect(bulkExport.selectedPatients).toHaveValue("single-1575");
  });

  for (const scope of ["system", "group"] as const) {
    test(`${scope} scope omits disabled pending Patients; reset clears chips and text (#1575)`, async ({ page, bulkExport }) => {
      await bulkExport.patientSearch.fill("selected-1575");
      await bulkExport.patientSearch.press("Enter");
      await bulkExport.patientSearch.fill("pending-1575");
      await bulkExport.scopeRadio(scope).check();
      await expect(bulkExport.patientCombobox.locator('[role="combobox"]')).toBeDisabled();
      if (scope === "group") await bulkExport.form.locator('input[name="group_id"]').fill("group-1575");
      await page.route("**/ui/bulk-export", (route) => route.request().method() === "POST" ? route.fulfill({ status: 204 }) : route.continue());
      const submitted = page.waitForRequest((req) => req.method() === "POST" && new URL(req.url()).pathname === "/ui/bulk-export");
      await bulkExport.startButton.click();
      const params = new URLSearchParams((await submitted).postData() ?? "");
      expect(params.get("scope")).toBe(scope);
      expect(params.getAll("patient")).toEqual([]);
      await expect(bulkExport.selectedPatients).toHaveValue("selected-1575");
      // Simulate returning from the intercepted native navigation (bfcache).
      await page.evaluate(() => window.dispatchEvent(new PageTransitionEvent("pageshow", { persisted: true })));
      await bulkExport.scopeRadio("patient").check();
      await expect(bulkExport.patientSearch).toHaveValue("pending-1575");
      // Clear preserves Patients; reset the form itself to clear chips/text.
      await bulkExport.form.evaluate((form: HTMLFormElement) => form.reset());
      await expect(bulkExport.selectedPatients).toHaveCount(0);
      await expect(bulkExport.patientCombobox.locator('[role="combobox"]')).toHaveValue("");
    });
  }
});

test("invalid Patient lists never create partial chips, survive late lookup, and recover on correction", async ({ page, bulkExport }) => {
  let release!: () => void;
  let requested!: () => void;
  const pending = new Promise<void>((resolve) => { requested = resolve; });
  const gate = new Promise<void>((resolve) => { release = resolve; });
  await page.route("**/ui/lookup/patient-options*", async (route) => {
    requested();
    await gate;
    await route.fulfill({ status: 200, contentType: "text/html", body: patientOptions + clearPatientMessage });
  });
  await bulkExport.goto();
  await bulkExport.scopeRadio("patient").check();
  const invalid = "p-valid,Andreshttp://example.test/ui/editor?resource=Patient";
  await bulkExport.patientSearch.fill(invalid);
  await expect(bulkExport.patientSearch).not.toHaveAttribute("aria-invalid", "true");
  await pending;
  await bulkExport.patientSearch.press("Enter");
  await expect(bulkExport.selectedPatients).toHaveCount(0);
  await expect(bulkExport.patientSearch).toHaveValue(invalid);
  const validation = bulkExport.patientCombobox.locator("[data-combobox-validation]");
  await expect(validation).toBeVisible();
  await expect(validation).toHaveAttribute("role", "alert");
  await expect(bulkExport.patientSearch).toHaveAttribute("aria-invalid", "true");
  release();
  await expect(bulkExport.patientCombobox.locator("[data-combobox-option]")).toHaveCount(2);
  await expect(validation).toBeVisible();
  await expect(bulkExport.patientListbox).toBeHidden();
  await bulkExport.patientSearch.press("ArrowDown");
  await bulkExport.patientSearch.press("Enter");
  await expect(validation).toBeHidden();
  await expect(bulkExport.patientSearch).not.toHaveAttribute("aria-invalid", /.+/);
  await expect(bulkExport.selectedPatients).toHaveValue("Patient/p-104");

  await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.evaluate(() => navigator.clipboard.writeText("p-valid\nGroup/wrong"));
  await bulkExport.patientSearch.press("ControlOrMeta+v");
  await expect(bulkExport.selectedPatients).toHaveCount(1);
  await expect(bulkExport.patientSearch).toHaveValue("p-valid, Group/wrong");
  await expect(validation).toBeVisible();
  await bulkExport.patientSearch.fill("p-valid,Patient/manual-id");
  await expect(validation).toBeHidden();
  await bulkExport.patientSearch.press("Enter");
  await expect(bulkExport.selectedPatients).toHaveCount(3);
  await expect(bulkExport.patientSearch).toHaveValue("");
  await bulkExport.patientSearch.fill("Patient/");
  await bulkExport.patientSearch.press("Enter");
  await expect(validation).toBeVisible();
  await bulkExport.form.evaluate((form: HTMLFormElement) => form.reset());
  await expect(validation).toBeHidden();
  await expect(bulkExport.patientCombobox.locator('[role="combobox"]')).not.toHaveAttribute("aria-invalid", /.+/);
});

test("ID-only Patient lookup preserves valid manual references without matching suggestions", async ({ page, bulkExport }) => {
  await page.route("**/ui/lookup/patient-options*", (route) => route.fulfill({
    status: 200, contentType: "text/html", body: patientMessage("Enter a Patient ID.", { alternate: true }),
  }));
  await bulkExport.goto();
  await bulkExport.scopeRadio("patient").check();
  await bulkExport.patientSearch.fill("manual-id");
  await expect(bulkExport.patientCombobox).toHaveAttribute("data-combobox-mode", "alternate");
  await expect(bulkExport.patientListbox).toBeHidden();
  await bulkExport.patientSearch.press("Enter");
  await bulkExport.patientSearch.fill("Patient/another-id");
  await bulkExport.patientSearch.press("Enter");
  expect(await bulkExport.selectedPatients.evaluateAll((inputs) => inputs.map((input) => (input as HTMLInputElement).value))).toEqual(["manual-id", "Patient/another-id"]);
});

test("server-rejected Patient fallback remains editable on enhancement and reset", async ({ page, bulkExport }) => {
  await bulkExport.goto();
  await bulkExport.nameInput.fill("Server rejection hydration");
  await bulkExport.scopeRadio("patient").check();
  const rejected = page.waitForResponse((response) => response.request().method() === "POST" && new URL(response.url()).pathname === "/ui/bulk-export");
  // Exercise the server's independent validation by bypassing enhancement.
  await bulkExport.form.evaluate((form: HTMLFormElement) => {
    const fallback = form.querySelector<HTMLTextAreaElement>('textarea[name="patient"]')!;
    fallback.disabled = false;
    fallback.value = "Patient/valid,Group/invalid";
    HTMLFormElement.prototype.submit.call(form);
  });
  expect((await rejected).status()).toBe(400);
  await expect(bulkExport.patientSearch).toHaveValue("Patient/valid,Group/invalid");
  await expect(bulkExport.selectedPatients).toHaveCount(0);
  await expect(bulkExport.patientSearch).toHaveAttribute("aria-invalid", "true");
  await bulkExport.patientSearch.fill("corrected");
  await bulkExport.patientSearch.press("Enter");
  await expect(bulkExport.selectedPatients).toHaveValue("corrected");
  await bulkExport.form.evaluate((form: HTMLFormElement) => form.reset());
  await expect(bulkExport.selectedPatients).toHaveCount(0);
  await expect(bulkExport.patientSearch).toHaveValue("Patient/valid,Group/invalid");
  await expect(bulkExport.patientSearch).toHaveAttribute("aria-invalid", "true");
});

// Real card templates at each persisted lifecycle state; only polling timing
// is parked so desktop and wrapping layouts see the same records.
test("status precedes every export card action at desktop and narrow widths", async ({ page, request }) => {
  const previous = (await (await request.get("/_user/settings")).json()).bulkExport ?? null;
  const states = ["in-progress", "complete", "failed", "cancelled"];
  const jobs = Object.fromEntries(states.map((status) => [status, {
    name: `Status order ${status}`, status, scope: "system", remoteJob: "no-remote-job",
    startedAt: "2026-01-01T09:00:00Z", finishedAt: "2026-01-01T09:05:00Z",
    files: status === "complete" ? [{ type: "Patient", url: "ignored" }] : [],
  }]));
  await page.route("**/ui/bulk-export/active/*/card", (route) => route.fulfill({ status: 204 }));
  try {
    expect((await request.patch("/_user/settings", { data: { bulkExport: null } })).ok()).toBe(true);
    expect((await request.patch("/_user/settings", { data: { bulkExport: { jobs } } })).ok()).toBe(true);
    for (const viewport of [{ width: 1920, height: 1080 }, { width: 390, height: 844 }]) {
      await page.setViewportSize(viewport);
      await page.goto("/ui/bulk-export");
      await expect(page.locator(".job-card")).toHaveCount(states.length);
      for (const state of states) {
        const card = page.locator(`#job-${state}`);
        await expect(card.locator(".job-card__actions .btn:visible")).toHaveCount(
          state === "complete" || state === "failed" ? 2 : 1);
        expect(await card.locator(".job-card__actions").evaluate((group) => {
          const status = group.querySelector(".tag")!;
          const box = status.getBoundingClientRect();
          return Array.from(group.querySelectorAll(".btn"))
            .filter((action) => action.getBoundingClientRect().width > 0)
            .every((action) => {
              const rect = action.getBoundingClientRect();
              return Boolean(status.compareDocumentPosition(action) & Node.DOCUMENT_POSITION_FOLLOWING) &&
                (box.bottom <= rect.top + 1 ||
                  (box.top < rect.bottom && box.bottom > rect.top && box.right <= rect.left + 1));
            });
        })).toBe(true);
      }
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    }
  } finally {
    await request.patch("/_user/settings", { data: { bulkExport: null } });
    await request.patch("/_user/settings", { data: { bulkExport: previous } });
  }
});

// #1758: Delete lives in the card's overflow menu and asks in the shared
// dialog, so nothing floats over the next card.
test("export Delete sits in the overflow menu and confirms in the shared dialog", async ({ page, request }) => {
  const previous = (await (await request.get("/_user/settings")).json()).bulkExport ?? null;
  const jobs = Object.fromEntries(["menu-first", "menu-second"].map((id, index) => [id, {
    name: `Menu export ${id}`, status: "cancelled", scope: "system", remoteJob: "no-remote-job",
    startedAt: `2026-01-01T09:0${index}:00Z`, finishedAt: `2026-01-01T09:0${index}:30Z`,
  }]));
  await page.route("**/ui/bulk-export/active/*/card", (route) => route.fulfill({ status: 204 }));
  try {
    expect((await request.patch("/_user/settings", { data: { bulkExport: null } })).ok()).toBe(true);
    expect((await request.patch("/_user/settings", { data: { bulkExport: { jobs } } })).ok()).toBe(true);
    await page.goto("/ui/bulk-export");
    await expect(page.locator(".job-card")).toHaveCount(2);
    const first = page.locator("#job-menu-first");
    const second = page.locator("#job-menu-second");
    const disclosure = first.locator("details.job-card__delete");

    // Cancel keeps both cards; the disclosure never opens with JavaScript.
    await first.locator("details.menu > summary").click();
    await disclosure.locator("summary").click();
    await expect(confirmDialog(page)).toBeVisible();
    await expect(confirmDialog(page).locator(".confirm-dialog__message")).toContainText("Menu export menu-first");
    await expect(confirmDialog(page).locator("[data-confirm-ok]")).toHaveText("Delete export");
    await expect(disclosure).not.toHaveAttribute("open", /.*/);
    // Nothing of the first card covers the second card's actions.
    await expect(first.locator(".job-card__delete-confirm")).toBeHidden();
    const secondActions = await second.locator(".job-card__actions").boundingBox();
    expect(secondActions).not.toBeNull();
    await dismissConfirm(page);
    await expect(page.locator(".job-card")).toHaveCount(2);

    // The menu stays open after Cancel; confirming deletes only the first card.
    await expect(first.locator("details.menu")).toHaveAttribute("open", "");
    await disclosure.locator("summary").click();
    await acceptConfirm(page, /Menu export menu-first/);
    await expect(first).toHaveCount(0);
    await expect(second).toBeVisible();
    await expect(confirmDialog(page)).toHaveCount(0);
  } finally {
    await request.patch("/_user/settings", { data: { bulkExport: null } });
    await request.patch("/_user/settings", { data: { bulkExport: previous } });
  }
});

// #1758: Retry and Run again start a fresh export and add a new card; the
// original card is never rewritten.
test("Retry on a failed export and Run again on a complete one add new cards and keep the originals", async ({ page, request }) => {
  const previous = (await (await request.get("/_user/settings")).json()).bulkExport ?? null;
  const jobs = {
    "rerun-failed": {
      name: "Rerun failed export", status: "failed", scope: "system", remoteJob: "no-remote-job",
      error: "seeded failure", startedAt: "2026-01-01T09:00:00Z", finishedAt: "2026-01-01T09:01:00Z",
    },
    "rerun-complete": {
      name: "Rerun complete export", status: "complete", scope: "system", remoteJob: "no-remote-job",
      startedAt: "2026-01-01T09:02:00Z", finishedAt: "2026-01-01T09:03:00Z",
      files: [{ type: "Patient", url: "ignored" }],
    },
  };
  await page.route("**/ui/bulk-export/active/*/card", (route) => route.fulfill({ status: 204 }));
  try {
    expect((await request.patch("/_user/settings", { data: { bulkExport: null } })).ok()).toBe(true);
    expect((await request.patch("/_user/settings", { data: { bulkExport: { jobs } } })).ok()).toBe(true);
    await page.goto("/ui/bulk-export");
    await expect(page.locator(".job-card")).toHaveCount(2);

    const failed = page.locator("#job-rerun-failed");
    await expect(failed.getByRole("button", { name: "Run again" })).toHaveCount(0);
    await failed.getByRole("button", { name: "Retry" }).click();
    await expect(page.locator(".job-card")).toHaveCount(3);
    await expect(failed.locator(".tag--failed")).toBeVisible();
    await expect(failed).toContainText("seeded failure");

    const complete = page.locator("#job-rerun-complete");
    await complete.locator("details.menu > summary").click();
    await complete.getByRole("button", { name: "Run again" }).click();
    await expect(page.locator(".job-card")).toHaveCount(4);
    await expect(complete.locator(".tag--complete")).toBeVisible();
  } finally {
    await request.patch("/_user/settings", { data: { bulkExport: null } });
    await request.patch("/_user/settings", { data: { bulkExport: previous } });
  }
});
