import { test, expect } from "../pages/fixtures";
import AxeBuilder from "@axe-core/playwright";
import { axeSummary } from "../pages/axe";

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
    actions.append(status);
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

    expect(Math.abs(firstResourceTop - allResourcesBottom - 14)).toBeLessThanOrEqual(0.5);
  }
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
  await bulkExport.goto();
  await bulkExport.nameInput.fill("Patient-only rejection");
  await bulkExport.scopeRadio("patient").check();
  await bulkExport.sincePreset.selectOption("custom");
  await bulkExport.sinceCustom.fill("2026-08-01T00:00:00Z");
  await bulkExport.form.evaluate((form) => {
    const patient = document.createElement("input");
    patient.type = "hidden";
    patient.name = "patient";
    patient.value = "Patient/not/valid";
    form.append(patient);
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
  await bulkExport.goto();
  await bulkExport.nameInput.fill(exportName);
  await bulkExport.scopeRadio("group").check();
  await bulkExport.form.locator('input[name="group_id"]').fill("preserved-group");
  await bulkExport.allResources.uncheck();
  await bulkExport.typeCheckbox("Patient").check();
  await bulkExport.typeCheckbox("Observation").check();
  await bulkExport.form.locator('input[name="elements"]').fill("id,meta");
  await bulkExport.form
    .locator('input[name="type_filter"]')
    .fill("Patient?active=true");
  await bulkExport.sincePreset.selectOption("custom");
  await bulkExport.sinceCustom.fill("2026-02-31T00:00:00Z");
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
  await expect(bulkExport.scopeRadio("group")).toBeChecked();
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
  await expect(bulkExport.clearLink).toHaveAttribute("href", "/ui/bulk-export/new");

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

test("Patient combobox supports keyboard selection, dedupe, removal, and scope serialization", async ({
  page,
  bulkExport,
}) => {
  const queries: string[] = [];
  await page.route("**/ui/bulk-export/patient-options", (route) => {
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
  await expect(bulkExport.patientListbox.getByRole("option").first()).toHaveAttribute(
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

test("Patient combobox closes on Tab and Clear removes selected patients", async ({
  page,
  bulkExport,
}) => {
  await page.route("**/ui/bulk-export/patient-options", (route) =>
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
  await bulkExport.clearButton.click();
  await expect(bulkExport.scopeRadio("system")).toBeChecked();
  await expect(bulkExport.selectedPatients).toHaveCount(0);
});

test("open Patient combobox has no automated accessibility violations", async ({
  page,
  bulkExport,
}) => {
  await page.route("**/ui/bulk-export/patient-options", (route) =>
    route.fulfill({ status: 200, contentType: "text/html", body: patientOptions }),
  );
  await bulkExport.goto();
  await bulkExport.scopeRadio("patient").check();
  await bulkExport.patientSearch.fill("an");
  await expect(bulkExport.patientListbox).toBeVisible();
  await bulkExport.patientSearch.press("ArrowDown");
  await bulkExport.patientSearch.press("Enter");

  const { violations } = await new AxeBuilder({ page }).analyze();
  expect(violations, axeSummary(violations)).toEqual([]);
});

test("Patient combobox shows empty and error messages and persists a runtime ID-only downgrade", async ({
  page,
  bulkExport,
}) => {
  await page.route("**/ui/bulk-export/patient-options", (route) => {
    const query = new URLSearchParams(route.request().postData() ?? "").get("q");
    if (query === "transport") {
      return route.fulfill({ status: 500, contentType: "text/plain", body: "server error" });
    }
    let body = patientMessage("No matching patients found.");
    if (query === "an") {
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
  await bulkExport.clearButton.click();
  await bulkExport.scopeRadio("patient").check();
  await expect(bulkExport.patientMessage).toBeHidden();

  await bulkExport.patientSearch.fill("fail");
  await expect(bulkExport.patientMessage.locator(".field__hint--error")).toBeVisible();
  await expect(bulkExport.patientMessage).toContainText("Suggestions could not be loaded");
  violations = (await new AxeBuilder({ page }).analyze()).violations;
  expect(violations, axeSummary(violations)).toEqual([]);

  await bulkExport.clearButton.click();
  await bulkExport.scopeRadio("patient").check();
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
  await expect(bulkExport.patientHint).toContainText("exact logical FHIR ID");
  await expect(bulkExport.patientSearch).toHaveAttribute("placeholder", "Search exact FHIR ID");
  await expect(bulkExport.patientMessage).not.toHaveText(await bulkExport.patientHint.innerText());
  await bulkExport.patientSearch.press("Escape");
  await expect(bulkExport.patientHint).toContainText("exact logical FHIR ID");
  await expect(bulkExport.patientSearch).toHaveAttribute("placeholder", "Search exact FHIR ID");
  await bulkExport.clearButton.click();
  await bulkExport.scopeRadio("patient").check();
  await expect(bulkExport.patientMessage).toBeHidden();
  await expect(bulkExport.patientCombobox).toHaveAttribute("data-combobox-mode", "alternate");
  await expect(bulkExport.patientHint).toContainText("exact logical FHIR ID");
  await expect(bulkExport.patientSearch).toHaveAttribute("placeholder", "Search exact FHIR ID");
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
  await bulkExport.form.evaluate((form) => {
    HTMLFormElement.prototype.submit = function () {};
    form.dispatchEvent(new SubmitEvent("submit", { bubbles: true, cancelable: true }));
  });

  await expect(bulkExport.startButton).toHaveAttribute("aria-busy", "true");
  await expect(bulkExport.startButton).toBeDisabled();
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

test("re-checking and Clear restore the All Resources state", async ({ bulkExport }) => {
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

  await expect(bulkExport.nameInput).toHaveValue("");
  await expect(bulkExport.nameHeading).toHaveText("Bulk Export");
  await expect(bulkExport.scopeRadio("system")).toBeChecked();
  await expect(bulkExport.sincePreset).toHaveValue("");
  await expect(bulkExport.sinceCustom).toHaveValue("");
  await expect(bulkExport.sinceCustom).toBeDisabled();
  await expect(bulkExport.nameError).toBeHidden();
  await expect(bulkExport.sinceCustomError).toBeHidden();
  await expect(bulkExport.allResources).toBeChecked();
  expect(
    await bulkExport.typeCheckboxes.evaluateAll((types) =>
      types.every((type) => (type as HTMLInputElement).checked && (type as HTMLInputElement).disabled),
    ),
  ).toBe(true);

  await bulkExport.nameInput.fill("   ");
  await bulkExport.sincePreset.selectOption("custom");
  await bulkExport.sinceCustom.fill("not-an-instant");
  await expect(bulkExport.nameError).toBeHidden();
  await expect(bulkExport.sinceCustomError).toBeHidden();
});
