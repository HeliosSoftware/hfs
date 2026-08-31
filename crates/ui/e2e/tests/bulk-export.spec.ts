import { test, expect } from "../pages/fixtures";

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

test("Custom instant follows the Since preset and form serialization", async ({ bulkExport }) => {
  await bulkExport.goto();

  for (const preset of ["", "day", "week", "month"]) {
    await bulkExport.sincePreset.selectOption(preset);
    await expect(bulkExport.sinceCustom).toBeDisabled();
  }

  const instant = "2026-08-01T00:00:00Z";
  await bulkExport.sincePreset.selectOption("custom");
  await expect(bulkExport.sinceCustom).toBeEnabled();
  await bulkExport.sinceCustom.fill(instant);

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
});

test("keyboard narrowing submits exactly the two selected resource types", async ({
  page,
  bulkExport,
}) => {
  await bulkExport.goto();

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
  await bulkExport.form.locator('input[name="name"]').fill("temporary name");
  await bulkExport.scopeRadio("patient").check();
  await bulkExport.sincePreset.selectOption("custom");
  await bulkExport.sinceCustom.fill("2026-08-01T00:00:00Z");
  await bulkExport.clearButton.click();

  await expect(bulkExport.form.locator('input[name="name"]')).toHaveValue("");
  await expect(bulkExport.scopeRadio("system")).toBeChecked();
  await expect(bulkExport.sincePreset).toHaveValue("");
  await expect(bulkExport.sinceCustom).toHaveValue("");
  await expect(bulkExport.sinceCustom).toBeDisabled();
  await expect(bulkExport.allResources).toBeChecked();
  expect(
    await bulkExport.typeCheckboxes.evaluateAll((types) =>
      types.every((type) => (type as HTMLInputElement).checked && (type as HTMLInputElement).disabled),
    ),
  ).toBe(true);
});
