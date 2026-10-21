import AxeBuilder from "@axe-core/playwright";
import { createResource, deleteResources, waitSearchable } from "../pages/api";
import { Editor } from "../pages/editor";
import { EDITOR_SCENARIOS, openEditorScenario } from "../pages/editor-scenarios";
import { fixedControl, fixedTrigger } from "../pages/fixed-select";
import { expect, test } from "../pages/fixtures";

const choices = {
  Patient: { name: "deceased", arm: "deceasedBoolean", primitive: true },
  ViewDefinition: { name: "versionAlgorithm", arm: "versionAlgorithmString", primitive: true },
  Library: { name: "subject", arm: "subjectCodeableConcept", primitive: false },
} as const;

for (const scenario of EDITOR_SCENARIOS) {
  test(`issue2080 ${scenario.name}: a themed choice updates JSON, focuses its node and remounts after Undo`, async ({ page, request }) => {
    const { ed, cleanup } = await openEditorScenario(page, request, scenario);
    const choice = choices[scenario.type];
    const selected: string[] = [];
    page.on("request", request => {
      if (!request.url().endsWith("/ui/editor/render")) return;
      const form = new URLSearchParams(request.postData() ?? "");
      if (form.get("op") === "choose") selected.push(form.get("arm") ?? "");
    });
    try {
      await ed.openAddPanel();
      await ed.addFilter().fill(choice.name);
      const native = ed.choiceType(choice.name);
      const old = await native.elementHandle();
      const trigger = fixedTrigger(native);
      await expect(native).toBeHidden();
      await expect(trigger).toHaveAccessibleName(new RegExp(`${choice.name}\\[x\\]`));
      await trigger.focus();
      await page.keyboard.press("ArrowDown");
      await expect(fixedControl(native).getByRole("listbox")).toBeVisible();
      // Moving the actual editor pane dismisses a popup whose anchor moved.
      const tree = ed.root.locator(".editor-tree");
      await tree.evaluate(node => { node.scrollTop = node.scrollHeight; });
      await expect(trigger).toHaveAttribute("aria-expanded", "false");
      await trigger.focus();
      await trigger.scrollIntoViewIfNeeded();
      await page.keyboard.press("ArrowDown");
      await page.keyboard.press("Escape");
      await expect(trigger).toHaveAttribute("aria-expanded", "false");
      await expect(ed.addPanel).toHaveAttribute("open");
      await page.keyboard.press("ArrowDown");
      await page.keyboard.press("Home");
      await page.keyboard.press("ArrowDown");
      await page.keyboard.press("Enter");
      await expect.poll(async () => Object.keys(await ed.currentDoc())).toContain(choice.arm);
      expect(selected).toEqual([choice.arm]);
      const created = choice.primitive ? ed.rowAt(choice.arm).locator("[data-set]") : ed.rowAt(choice.arm);
      await expect(created).toBeFocused();
      await expect(ed.addPanel).not.toHaveAttribute("open");
      const json = ed.root.locator("#editor-source, textarea[name=json]");
      expect(JSON.parse(await json.inputValue())).toHaveProperty(choice.arm);
      // Finishing the projection must not adopt the disconnected old SELECT.
      expect(await old!.evaluate(node => ({ connected: node.isConnected, mounted: node.hasAttribute("data-combobox-native") })))
        .toEqual({ connected: false, mounted: false });
      await old!.dispose();

      await ed.addUndo().click();
      await expect.poll(async () => Object.keys(await ed.currentDoc())).not.toContain(choice.arm);
      await ed.openAddPanel();
      await ed.addFilter().fill(choice.name);
      await expect(fixedTrigger(native)).toBeEnabled();
      await ed.chooseType(choice.name, choice.arm);
      await expect.poll(async () => Object.keys(await ed.currentDoc())).toContain(choice.arm);
      expect(selected).toEqual([choice.arm, choice.arm]);
      const selects = await ed.root.locator("section.editor-form select[data-combobox-select]").count();
      await expect(ed.root.locator("section.editor-form [data-combobox-generated]")).toHaveCount(selects);
    } finally { await cleanup(); }
  });
}

test("issue2080 RiskAssessment: Escape closes popup, Add, then modal; a pending choice locks its trigger", async ({ page, request }) => {
  const patient = await createResource(request, "Patient", { name: [{ family: `choice_risk_${Date.now()}` }] });
  const risk = await createResource(request, "RiskAssessment", { status: "final", subject: { reference: `Patient/${patient}` } });
  let release!: () => void;
  const held = new Promise<void>(resolve => { release = resolve; });
  try {
    await waitSearchable(request, "RiskAssessment", risk);
    await page.goto(`/ui/resources?url=${encodeURIComponent(`/RiskAssessment?_id=${risk}`)}`, { waitUntil: "networkidle" });
    const link = page.locator(`a.result-id[data-resource-id='${risk}']`);
    await link.click();
    const ed = new Editor(page, page.locator("#resource-editor-body"));
    await ed.openAddPanel();
    await ed.addFilter().fill("occurrence");
    const native = ed.choiceType("occurrence");
    await fixedTrigger(native).click();
    await page.keyboard.press("Escape");
    await expect(ed.addPanel).toHaveAttribute("open");
    await expect(page.locator("#resource-modal")).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(ed.addPanel).not.toHaveAttribute("open");
    await expect(page.locator("#resource-modal")).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.locator("#resource-modal")).toBeHidden();
    await link.click();
    await ed.openAddPanel();
    await ed.addFilter().fill("occurrence");

    await page.route("**/ui/editor/render", async route => {
      const form = new URLSearchParams(route.request().postData() ?? "");
      if (form.get("op") === "choose" && form.get("name") === "occurrence") await held;
      await route.continue();
    });
    await ed.chooseType("occurrence", "occurrencePeriod");
    await expect(native).toBeDisabled();
    await expect(fixedTrigger(native)).toBeDisabled();
    release();
    await expect.poll(async () => (await ed.currentDoc()).occurrencePeriod).toEqual({});
    await expect(ed.rowAt("occurrencePeriod")).toBeFocused();
  } finally {
    release();
    await page.unroute("**/ui/editor/render");
    await deleteResources(request, "RiskAssessment", [risk]);
    await deleteResources(request, "Patient", [patient]);
  }
});

for (const theme of ["light", "dark"] as const) {
  for (const host of ["Resources", "ViewDefinition"] as const) {
    test(`issue2080 ${host}: an open Guided popup stays accessible in ${theme}`, async ({ page, request, chrome }) => {
      await chrome.seedTheme(theme);
      const type = host === "Resources" ? "RiskAssessment" : "ViewDefinition";
      const id = await createResource(request, type, type === "RiskAssessment"
        ? { status: "final", subject: { reference: "Patient/themed-choice" } }
        : { name: `themed_choice_${Date.now()}`, status: "draft", resource: "Patient", select: [{ column: [{ name: "id", path: "getResourceKey()" }] }] });
      try {
        await waitSearchable(request, type, id);
        await page.goto(host === "Resources"
          ? `/ui/resources?url=${encodeURIComponent(`/RiskAssessment?_id=${id}`)}`
          : `/ui/sql/view-definitions?vd=${id}`, { waitUntil: "networkidle" });
        if (host === "Resources") await page.locator(`a.result-id[data-resource-id='${id}']`).click();
        const ed = new Editor(page, page.locator(host === "Resources" ? "#resource-editor-body" : "#vd-editor-grid"));
        await ed.openAddPanel();
        const name = host === "Resources" ? "occurrence" : "versionAlgorithm";
        await ed.addFilter().fill(name);
        const native = ed.choiceType(name);
        await fixedTrigger(native).click();
        const listbox = fixedControl(native).getByRole("listbox");
        await expect(listbox).toBeVisible();
        expect(await listbox.evaluate(node => {
          const rect = node.getBoundingClientRect();
          return rect.top >= 0 && rect.bottom <= window.innerHeight && rect.left >= 0 && rect.right <= window.innerWidth;
        })).toBe(true);
        const { violations } = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"]).analyze();
        expect(violations).toEqual([]);
      } finally { await deleteResources(request, type, [id]); }
    });
  }
}
