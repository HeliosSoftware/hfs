import { test, expect } from "../pages/fixtures";

// Natural-language search (/ui/search). The translator is mounted because the
// e2e server sets a placeholder HFS_NL_SEARCH_API_KEY. We stub /$nl-search so
// the flow is deterministic and no real model is called: the key promise is
// that translation only *lands a query* — it never runs one.

test("the mode segment toggles between describe and build", async ({ search }) => {
  await search.goto();
  await expect(search.modeButton("nl")).toHaveAttribute("aria-pressed", "true");
  await search.modeButton("builder").click();
  await expect(search.modeButton("builder")).toHaveAttribute("aria-pressed", "true");
  await expect(search.modeButton("nl")).toHaveAttribute("aria-pressed", "false");
});

test("a supported translation lands a query in the editable strip, and never runs it", async ({
  page,
  search,
}) => {
  let ran = false;
  await page.route(/\/\$nl-search$/, (route) =>
    route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        supported: true,
        target: "Patient",
        query: "name=smith",
        explanation: "Searches for patients named Smith.",
        caveats: ["Name match is case-insensitive."],
      }),
    }),
  );
  // Running the *translated* query would be a GET carrying its params. The rail
  // hydrates counts with ?_summary=count on load, so match the query itself.
  page.on("request", (r) => {
    if (r.method() === "GET" && /name=smith/.test(r.url())) ran = true;
  });

  await search.goto();
  await search.nlText.fill("patients named smith");
  await search.nlSubmit.click();

  await expect(search.nlAnswer).toBeVisible();
  await expect(search.nlAnswer).toHaveClass(/nl-answer--ok/);
  await expect(search.nlAnswer).toContainText("Smith");
  await expect(search.nlAnswer).toContainText("case-insensitive");
  // The translated query lands in the builder's URL, ready to review and run.
  await expect(search.builder.url).toHaveValue(/Patient\?name=smith/);
  expect(ran, "translation must not execute the query").toBe(false);
});

test("an unsupported request is a plain refusal, not an error", async ({ page, search }) => {
  await page.route(/\/\$nl-search$/, (route) =>
    route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({ supported: false, reason: "That is not a search over this data." }),
    }),
  );
  await search.goto();
  await search.nlText.fill("write me a bubble sort");
  await search.nlSubmit.click();

  await expect(search.nlAnswer).toHaveClass(/nl-answer--unsupported/);
  await expect(search.nlAnswer).toContainText(/not a search/i);
});

test("an example chip fills the box and translates", async ({ page, search }) => {
  await page.route(/\/\$nl-search$/, (route) =>
    route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({ supported: true, target: "Observation", query: "", explanation: "ok" }),
    }),
  );
  await search.goto();
  await search.nlExamples.first().click();
  await expect(search.nlText).not.toHaveValue("");
  await expect(search.nlAnswer).toBeVisible();
});
