import { expect, test } from "../pages/fixtures";
import { Editor } from "../pages/editor";
import { addFromRoot, EDITOR_SCENARIOS, expectStableReveal, openEditorScenario } from "../pages/editor-scenarios";

test.use({ viewport: { width: 1280, height: 800 } });

for (const scenario of EDITOR_SCENARIOS) {
  test(`issue1721 ${scenario.name}: a physical pointer press preserves dirty-field Add through mouseup`, async ({ page, request }) => {
    const { ed, cleanup } = await openEditorScenario(page, request, scenario);
    const operations: string[] = [];
    // Do not intercept or hold the server: a fast blur response while the
    // mouse remains down is exactly the lost-activation regression.
    page.on("request", request => {
      if (request.url().endsWith("/ui/editor/render")) {
        operations.push(new URLSearchParams(request.postData() ?? "").get("op") ?? "");
      }
    });
    try {
      const input = ed.rowAt("identifier.0.value").locator("[data-set]");
      const dirty = `physical-${scenario.name}`;
      await input.fill(dirty);
      const button = ed.collectionAdd("identifier");
      await button.scrollIntoViewIfNeeded();
      const box = await button.boundingBox();
      expect(box).not.toBeNull();
      await page.mouse.move(box!.x + box!.width / 2, box!.y + box!.height / 2);
      await page.mouse.down();
      await page.waitForTimeout(250);
      await expect(input).toBeFocused();
      expect(operations).toEqual([]);
      await page.mouse.up();
      await expectStableReveal(ed, "identifier.40", false);
      const doc = await ed.currentDoc() as Record<string, any>;
      expect(doc.identifier).toHaveLength(41);
      expect(doc.identifier[0].value).toBe(dirty);
      expect(operations).toEqual(["set", "add"]);
      await ed.addUndo().focus();
      await page.keyboard.press("Enter");
      await expect(ed.rowAt("identifier.40")).toHaveCount(0);
      await expect(ed.collectionAdd("identifier")).toBeFocused();
    } finally { await page.mouse.up(); await cleanup(); }
  });
}

test("issue1721 a physical SearchParameter Add preserves an unblurred base value", async ({ page }) => {
  await page.goto("/ui/editor?type=SearchParameter", { waitUntil: "networkidle" });
  const ed = new Editor(page, page.locator("#editor-body"));
  await ed.openAddPanel();
  await ed.addItem("base").click();
  await expect(ed.form).toHaveAttribute("data-focus", "base.0");
  const input = ed.rowAt("base.0").locator("[data-set]");
  await input.fill("Patient");
  const box = await ed.collectionAdd("base").boundingBox();
  expect(box).not.toBeNull();
  await page.mouse.move(box!.x + box!.width / 2, box!.y + box!.height / 2);
  try {
    await page.mouse.down();
    await page.waitForTimeout(250);
    await expect(input).toBeFocused();
    await page.mouse.up();
    await expectStableReveal(ed, "base.1", true);
    expect((await ed.currentDoc()).base).toEqual(["Patient", ""]);
    await ed.addUndo().focus();
    await page.keyboard.press("Enter");
    await expect(ed.rowAt("base.1")).toHaveCount(0);
    await expect(ed.collectionAdd("base")).toBeFocused();
  } finally { await page.mouse.up(); }
});

for (const scenario of EDITOR_SCENARIOS) {
  test(`issue1721 ${scenario.name}: a new primitive closes the picker and stays focused in the visible tree`, async ({ page, request }) => {
    const { ed, cleanup } = await openEditorScenario(page, request, scenario);
    try {
      await expect(ed.rowAt(scenario.primitivePath)).toHaveCount(0);
      await addFromRoot(ed, scenario.primitive);
      await expect(ed.addPanel).not.toHaveAttribute("open");
      await expectStableReveal(ed, scenario.primitivePath, true);
      await expect(ed.addStatus).toContainText(`${scenario.primitive} added`);
      await expect(ed.addStatus).toHaveClass(/visually-hidden/);
      await expect(ed.addStatus).toHaveAttribute("role", "status");
      await expect(ed.addStatus).not.toHaveAttribute("hidden");
      await expect(ed.root.locator(".editor-add__added")).toHaveCount(0);
      await expect(ed.addUndo()).toHaveAttribute("data-remove", scenario.primitivePath);
      expect(await ed.addUndo().evaluate(button => button.closest("details") === null)).toBe(true);
      if (scenario.type === "Library") {
        await expect(ed.rowAt("content")).toHaveCount(0);
        expect((await ed.currentDoc()).content).toEqual([{ contentType: "application/sql", data: Buffer.from("SELECT 1 AS value").toString("base64") }]);
      }
      await ed.addUndo().focus();
      await page.keyboard.press("Enter");
      await expect(ed.rowAt(scenario.primitivePath)).toHaveCount(0);
      await expect(ed.addPanel.locator("summary").first()).toBeFocused();
      await expect(ed.addUndo()).toHaveCount(0);
    } finally { await cleanup(); }
  });

  test(`issue1720 issue1721 ${scenario.name}: complex groups append the final item and Undo returns to their Add action`, async ({ page, request }) => {
    const { ed, cleanup } = await openEditorScenario(page, request, scenario);
    try {
      await addFromRoot(ed, scenario.complex);
      const first = `${scenario.complex}.0`;
      await expect(ed.addPanel).not.toHaveAttribute("open");
      await expectStableReveal(ed, first, false);
      await expect(ed.rowAt(first)).toHaveAccessibleName(`${scenario.complex}[0] — ${first}`);
      const group = ed.rowAt(scenario.complex);
      await expect(group).toHaveAttribute("data-collection", "");
      await expect(group.locator(".editor-row__label")).toHaveText(scenario.complex);
      await expect(ed.rowAt(first).locator(".editor-row__label")).toHaveText("[0]");
      await expect(ed.collectionAdd(scenario.complex)).toHaveAttribute("data-add", "");
      await expect(ed.collectionAdd(scenario.complex)).toHaveAttribute("data-name", scenario.complex);

      // This is the native-scroll maximum: the added item is last in its
      // collection and no later fields pad the scroll range in these fixtures.
      await ed.collectionAdd(scenario.complex).click();
      const second = `${scenario.complex}.1`;
      await expectStableReveal(ed, second, false);
      await expect(ed.rowAt(second).locator(".editor-row__label")).toHaveText("[1]");
      expect((await ed.currentDoc())[scenario.complex]).toEqual([{}, {}]);
      await expect(ed.addUndo()).toHaveAttribute("data-remove", second);
      await expect(ed.addStatus).toContainText(`${scenario.complex} added`);
      await ed.addUndo().focus();
      await page.keyboard.press("Enter");
      await expect(ed.rowAt(second)).toHaveCount(0);
      expect((await ed.currentDoc())[scenario.complex]).toEqual([{}]);
      await expect(ed.collectionAdd(scenario.complex)).toBeFocused();
      await expect(ed.addUndo()).toHaveCount(0);
      await ed.rowAt(first).locator(`[data-remove='${first}']`).click();
      await expect(group).toHaveCount(0);
      expect(await ed.currentDoc()).not.toHaveProperty(scenario.complex);
    } finally { await cleanup(); }
  });
}

test("issue1720 base and target have distinct owners; direct append preserves values and reindexes exact controls", async ({ page }) => {
  await page.goto("/ui/editor?type=SearchParameter", { waitUntil: "networkidle" });
  const ed = new Editor(page, page.locator("#editor-body"));
  await ed.openAddPanel();
  await ed.addFilter().fill("base");
  await ed.addItem("base").click();
  await expect(ed.form).toHaveAttribute("data-focus", "base.0");
  await expect(ed.rowAt("base").locator(".editor-row__label")).toHaveText("base");
  await expect(ed.rowAt("base.0").locator("[data-set]")).toHaveAccessibleName("base[0] — base.0");
  await ed.rowAt("base.0").locator("[data-set]").fill("Patient");
  await ed.rowAt("base.0").locator("[data-set]").blur();
  await expect.poll(() => ed.currentDoc()).toMatchObject({ base: ["Patient"] });
  await expect(ed.addUndo()).toHaveCount(0);
  await ed.collectionAdd("base").click();
  await expect(ed.form).toHaveAttribute("data-focus", "base.1");
  expect((await ed.currentDoc()).base).toEqual(["Patient", ""]);
  await ed.openAddPanel();
  await ed.addFilter().fill("target");
  await ed.addItem("target").click();
  await expect(ed.form).toHaveAttribute("data-focus", "target.0");
  await expect(ed.rowAt("target").locator(".editor-row__label")).toHaveText("target");
  await expect(ed.rowAt("target.0").locator("[data-set]")).toHaveAccessibleName("target[0] — target.0");
  await ed.rowAt("base.0").locator("[data-remove='base.0']").click();
  await expect(ed.rowAt("base.1")).toHaveCount(0);
  await expect(ed.rowAt("base.0").locator("[data-set]")).toHaveValue("");
  await expect(ed.rowAt("target.0")).toHaveCount(1);
});

test("issue1721 an already visible primitive receives focus without moving the tree or page", async ({ page }) => {
  await page.goto("/ui/editor?type=Patient", { waitUntil: "networkidle" });
  const ed = new Editor(page, page.locator("#editor-body"));
  await ed.openAddPanel();
  await ed.addFilter().fill("birth");
  const scroll = await page.evaluate(() => window.scrollY);
  await ed.addItem("birthDate").click();
  await expectStableReveal(ed, "birthDate", true);
  expect(await ed.root.locator(".editor-tree").evaluate(node => node.scrollTop)).toBe(0);
  expect(await page.evaluate(() => window.scrollY)).toBe(scroll);
});

test("issue1721 an unrelated picker keeps its filter while the initiating picker closes", async ({ page }) => {
  await page.goto("/ui/editor?type=Patient", { waitUntil: "networkidle" });
  const ed = new Editor(page, page.locator("#editor-body"));
  await ed.applyJson({ resourceType: "Patient", name: [{}] });
  await expect(ed.rowAt("name.0")).toBeAttached();
  const nested = new Editor(page, ed.rowAt("name.0"));
  await ed.openAddPanel();
  await ed.addFilter().fill("birth");
  let release!: () => void;
  let started!: () => void;
  const held = new Promise<void>(resolve => { release = resolve; });
  const pending = new Promise<void>(resolve => { started = resolve; });
  await page.route("**/ui/editor/render", async route => { started(); await held; await route.continue(); });
  try {
    await ed.addItem("birthDate").click();
    await pending;
    // UI state is captured at response-time. A picker opened while the
    // request is pending must survive independently of the Add initiator.
    await nested.openAddPanel();
    await nested.addFilter().fill("family");
    release();
    await expect(ed.form).toHaveAttribute("data-focus", "birthDate");
    await expect(ed.addPanel).not.toHaveAttribute("open");
    await expect(nested.addPanel).toHaveAttribute("open", "");
    await expect(nested.addFilter()).toHaveValue("family");
    await expect(ed.rowAt("birthDate").locator("[data-set]")).toBeFocused();
  } finally { release(); await page.unroute("**/ui/editor/render"); }
});

test("issue1721 a pending blur/set disables stale Undo, and a successful refresh expires it", async ({ page }) => {
  await page.goto("/ui/editor?type=Patient", { waitUntil: "networkidle" });
  const ed = new Editor(page, page.locator("#editor-body"));
  await ed.openAddPanel();
  await ed.addItem("birthDate").click();
  await expect(ed.addUndo()).toHaveAttribute("data-remove", "birthDate");
  let release!: () => void;
  let started!: () => void;
  const held = new Promise<void>(resolve => { release = resolve; });
  const pending = new Promise<void>(resolve => { started = resolve; });
  let removes = 0;
  await page.route("**/ui/editor/render", async route => {
    const op = new URLSearchParams(route.request().postData() ?? "").get("op");
    if (op === "remove") removes++;
    if (op === "set") { started(); await held; }
    await route.continue();
  });
  try {
    const input = ed.rowAt("birthDate").locator("[data-set]");
    await input.fill("2024-05-17");
    await input.blur();
    await pending;
    await expect(ed.addUndo()).toBeDisabled();
    await expect(ed.addUndo()).toHaveAttribute("aria-busy", "true");
    await expect(ed.addUndo()).toHaveAttribute("title", /updating/i);
    // Real keyboard activation of a disabled button must not launch a stale
    // snapshot remove while the set response is held.
    await ed.addUndo().evaluate(node => (node as HTMLButtonElement).focus());
    await page.keyboard.press("Enter");
    expect(removes).toBe(0);
    release();
    await expect.poll(() => ed.currentDoc()).toMatchObject({ birthDate: "2024-05-17" });
    await expect(ed.addUndo()).toHaveCount(0);
    expect(removes).toBe(0);
  } finally { release(); await page.unroute("**/ui/editor/render"); }
});

test("issue1721 a failed creation leaves previous Undo available; raw JSON replacement clears it", async ({ page }) => {
  await page.goto("/ui/editor?type=Patient", { waitUntil: "networkidle" });
  const ed = new Editor(page, page.locator("#editor-body"));
  await ed.openAddPanel();
  await ed.addItem("birthDate").click();
  await expect(ed.addUndo()).toHaveAttribute("data-remove", "birthDate");
  await page.route("**/ui/editor/render", route => route.abort("failed"));
  await ed.openAddPanel();
  await ed.addItem("gender").click();
  await expect(ed.addUndo()).toBeEnabled();
  await expect(ed.addUndo()).toHaveAttribute("data-remove", "birthDate");
  expect(await ed.currentDoc()).not.toHaveProperty("gender");
  await page.unroute("**/ui/editor/render");
  await ed.applyJson({ resourceType: "Patient", name: [{ given: ["Restored"] }] });
  await expect(ed.rowAt("name.0.given.0")).toBeAttached();
  await expect(ed.addUndo()).toHaveCount(0);
  await expect(ed.addStatus).toBeEmpty();
});

for (const scenario of EDITOR_SCENARIOS) {
  test(`issue1721 ${scenario.name}: a held structural change blocks controls with stale array indexes`, async ({ page, request }) => {
    const { ed, cleanup } = await openEditorScenario(page, request, scenario);
    let release!: () => void;
    let started!: () => void;
    const held = new Promise<void>(resolve => { release = resolve; });
    const pending = new Promise<void>(resolve => { started = resolve; });
    const operations: string[] = [];
    await page.route("**/ui/editor/render", async route => {
      const data = new URLSearchParams(route.request().postData() ?? "");
      operations.push(`${data.get("op")}:${data.get("path")}`);
      if (data.get("op") === "remove") { started(); await held; }
      await route.continue();
    });
    try {
      await ed.rowAt("identifier.0").locator("[data-remove='identifier.0']").click();
      await pending;
      await expect(ed.root).toHaveAttribute("data-editor-structural-pending", "");
      await expect(ed.root).toHaveAttribute("aria-busy", "true");
      const staleRemove = ed.rowAt("identifier.1").locator("[data-remove='identifier.1']");
      const staleInput = ed.rowAt("identifier.1.value").locator("[data-set]");
      await expect(staleRemove).toBeDisabled();
      await expect(staleInput).toBeDisabled();
      await expect(ed.collectionAdd("identifier")).toBeDisabled();
      // Real input on the still-rendered second item cannot submit its old
      // index. Locator.click would wait until the disabled state ended.
      await staleRemove.scrollIntoViewIfNeeded();
      const box = await staleRemove.boundingBox();
      expect(box).not.toBeNull();
      await page.mouse.click(box!.x + box!.width / 2, box!.y + box!.height / 2);
      // Also guard synthetic events, even if a control bypasses native UI.
      await staleRemove.dispatchEvent("click");
      await staleInput.dispatchEvent("blur");
      await page.waitForTimeout(100);
      expect(operations).toEqual(["remove:identifier.0"]);
      release();
      await expect.poll(async () => (await ed.currentDoc() as Record<string, any>).identifier.slice(0, 2))
        .toMatchObject([{ value: "20001" }, { value: "20002" }]);
      await expect(ed.root).not.toHaveAttribute("data-editor-structural-pending");
      await expect(ed.rowAt("identifier.0.value").locator("[data-set]")).toBeEnabled();
      expect((await ed.currentDoc() as Record<string, any>).identifier).toHaveLength(39);
      expect(operations).toEqual(["remove:identifier.0"]);
    } finally { release(); await page.unroute("**/ui/editor/render"); await cleanup(); }
  });
}

test("issue1721 a failed structural request unlocks original controls for explicit retry", async ({ page, request }) => {
  const { ed, cleanup } = await openEditorScenario(page, request, EDITOR_SCENARIOS[0]);
  let release!: () => void;
  let started!: () => void;
  const held = new Promise<void>(resolve => { release = resolve; });
  const pending = new Promise<void>(resolve => { started = resolve; });
  await page.route("**/ui/editor/render", async route => { started(); await held; await route.abort("failed"); });
  try {
    await ed.rowAt("identifier.0").locator("[data-remove='identifier.0']").click();
    await pending;
    await expect(ed.rowAt("identifier.1.value").locator("[data-set]")).toBeDisabled();
    release();
    await expect(ed.root.locator("[data-editor-failure]")).toBeVisible();
    await expect(ed.root).not.toHaveAttribute("data-editor-structural-pending");
    await expect(ed.rowAt("identifier.1.value").locator("[data-set]")).toBeEnabled();
    expect((await ed.currentDoc() as Record<string, any>).identifier).toHaveLength(40);
    await page.unroute("**/ui/editor/render");
    await ed.rowAt("identifier.1").locator("[data-remove='identifier.1']").click();
    await expect.poll(async () => (await ed.currentDoc() as Record<string, any>).identifier.slice(0, 2))
      .toMatchObject([{ value: "20000" }, { value: "20002" }]);
    expect((await ed.currentDoc() as Record<string, any>).identifier).toHaveLength(39);
    await expect(ed.root.locator("[data-editor-failure]")).toBeHidden();
  } finally { release(); await page.unroute("**/ui/editor/render"); await cleanup(); }
});

// The browser's actual blur -> click sequence must retain the dirty value.
// Holding the set response proves Add waits, rather than winning by chance.
for (const scenario of EDITOR_SCENARIOS) {
  test(`issue1721 ${scenario.name}: immediate collection Add waits for the dirty field's set`, async ({ page, request }) => {
    const { ed, cleanup } = await openEditorScenario(page, request, scenario);
    let release!: () => void;
    let started!: () => void;
    const held = new Promise<void>(resolve => { release = resolve; });
    const pending = new Promise<void>(resolve => { started = resolve; });
    const addedDocs: Record<string, any>[] = [];
    await page.route("**/ui/editor/render", async route => {
      const data = new URLSearchParams(route.request().postData() ?? "");
      if (data.get("op") === "set") { started(); await held; }
      if (data.get("op") === "add") addedDocs.push(JSON.parse(data.get("doc")!));
      await route.continue();
    });
    try {
      const dirty = `typed-${scenario.name}`;
      await ed.rowAt("identifier.0.value").locator("[data-set]").fill(dirty);
      await ed.collectionAdd("identifier").click();
      await pending;
      await page.waitForTimeout(100);
      expect(addedDocs).toHaveLength(0);
      release();
      await expectStableReveal(ed, "identifier.40", false);
      const doc = await ed.currentDoc() as Record<string, any>;
      expect(doc.identifier).toHaveLength(41);
      expect(doc.identifier[0].value).toBe(dirty);
      expect(addedDocs).toHaveLength(1);
      expect(addedDocs[0].identifier[0].value).toBe(dirty);
      await ed.addUndo().click();
      await expect(ed.rowAt("identifier.40")).toHaveCount(0);
      await expect(ed.collectionAdd("identifier")).toBeFocused();
      expect((await ed.currentDoc() as Record<string, any>).identifier[0].value).toBe(dirty);
    } finally {
      release(); await page.unroute("**/ui/editor/render"); await cleanup();
    }
  });
}

for (const failure of ["network", "malformed"] as const) {
  test(`issue1721 a ${failure} prerequisite retains the dirty field and blocks queued Add until explicit retry`, async ({ page }) => {
    await page.goto("/ui/editor?type=SearchParameter", { waitUntil: "networkidle" });
    const ed = new Editor(page, page.locator("#editor-body"));
    await ed.openAddPanel();
    await ed.addItem("base").click();
    await expect(ed.form).toHaveAttribute("data-focus", "base.0");
    let release!: () => void;
    let started!: () => void;
    const held = new Promise<void>(resolve => { release = resolve; });
    const pending = new Promise<void>(resolve => { started = resolve; });
    let adds = 0;
    await page.route("**/ui/editor/render", async route => {
      const op = new URLSearchParams(route.request().postData() ?? "").get("op");
      if (op === "add") adds++;
      if (op === "set") {
        started(); await held;
        if (failure === "network") await route.abort("failed");
        else await route.fulfill({ status: 200, contentType: "text/html", body: "<p>Incomplete render</p>" });
      } else await route.continue();
    });
    try {
      const input = ed.rowAt("base.0").locator("[data-set]");
      await input.fill("Patient");
      await ed.collectionAdd("base").click();
      await pending;
      release();
      await expect(ed.root.locator("[data-editor-failure]")).toBeVisible();
      await expect(input).toHaveValue("Patient");
      await expect(input).toBeFocused();
      await expect(input).toHaveAttribute("aria-invalid", "true");
      await expect(ed.rowAt("base.1")).toHaveCount(0);
      expect(adds).toBe(0);
      // An actual later click retries the retained dirty input via blur,
      // then appends against that successful set response.
      await page.unroute("**/ui/editor/render");
      await ed.collectionAdd("base").click();
      await expectStableReveal(ed, "base.1", true);
      expect((await ed.currentDoc()).base).toEqual(["Patient", ""]);
      await expect(ed.root.locator("[data-editor-failure]")).toBeHidden();
      await expect(input).not.toHaveAttribute("aria-invalid", "true");
    } finally { release(); await page.unroute("**/ui/editor/render"); }
  });
}

for (const scenario of EDITOR_SCENARIOS.filter(scenario => scenario.type !== "Patient")) {
  for (const timing of ["timer", "in-flight"] as const) {
    test(`issue1721 ${scenario.name}: a stale raw ${timing} cannot replace a later Guided addition`, async ({ page, request }) => {
      const { ed, cleanup } = await openEditorScenario(page, request, scenario);
      let release!: () => void;
      let started!: () => void;
      const held = new Promise<void>(resolve => { release = resolve; });
      const pending = new Promise<void>(resolve => { started = resolve; });
      let rawRequests = 0;
      await page.route("**/ui/editor/render", async route => {
        const op = new URLSearchParams(route.request().postData() ?? "").get("op");
        if (!op) {
          rawRequests++;
          if (timing === "in-flight") { started(); await held; }
        }
        await route.continue();
      });
      try {
        // Prepare the actual Add opener before editing raw JSON so the
        // timer case clicks it comfortably inside the 600ms debounce.
        await ed.openAddPanel();
        await ed.addFilter().fill(scenario.primitive);
        const raw = ed.root.locator("textarea[name='json']");
        const cm = ed.root.locator(".cm-content");
        await cm.evaluate(dom => {
          const CM = (window as unknown as { HfsCodeMirror: any }).HfsCodeMirror;
          const view = CM.EditorView.findFromDOM(dom);
          if (!view) throw new Error("paired editor is not mounted");
          const doc = JSON.parse(view.state.doc.toString());
          doc.description = "Raw edit immediately preceding Guided Add";
          view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: JSON.stringify(doc, null, 2) } });
        });
        if (timing === "in-flight") await pending;
        await ed.addItem(scenario.primitive).click();
        await expectStableReveal(ed, scenario.primitivePath, true);
        if (timing === "in-flight") {
          await expect(ed.addUndo()).toBeDisabled();
          release();
        }
        // Exceed the real raw debounce, not just the 120ms caret timer.
        await page.waitForTimeout(750);
        await expectStableReveal(ed, scenario.primitivePath, true);
        await expect(ed.addUndo()).toBeEnabled();
        const guided = await ed.currentDoc();
        expect(JSON.parse(await raw.inputValue())).toEqual(guided);
        expect(guided.description).toBe("Raw edit immediately preceding Guided Add");
        expect(guided).toHaveProperty(scenario.primitive);
        expect(rawRequests).toBe(timing === "timer" ? 0 : 1);
      } finally {
        release(); await page.unroute("**/ui/editor/render"); await cleanup();
      }
    });
  }
}
