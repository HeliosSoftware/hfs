import { test, expect } from "../pages/fixtures";
import { createResource, waitSearchable } from "../pages/api";

// The Saved Queries workspace (/ui/queries): the shared query builder and the
// in-page results table — run a query, add builder rows, page through results,
// and see the datalist of parameters swap per type.
//
// saved-queries.js hides the whole builder form when the per-user settings
// store is unavailable (some backends don't provide one), so these skip
// themselves there — the same stance as the tenants suite.

test.describe("query builder", () => {
  test.beforeEach(async ({ queries }) => {
    await queries.goto();
    if (await queries.builder.form.isHidden().catch(() => true)) {
      test.skip(true, "no per-user settings store on this backend; the builder is hidden");
    }
  });

  test("running a query shows the results table with a total", async ({ queries, request }) => {
    await createResource(request, "Patient", { name: [{ family: "QueryA" }] });
    await createResource(request, "Patient", { name: [{ family: "QueryB" }] });

    await queries.goto();
    await queries.builder.run("Patient");
    await queries.results.waitShown();
    await expect(queries.results.rows.first()).toBeVisible();
    await expect(queries.results.meta).toContainText(/\d/);
  });

  test("the results table pages when a query spans more than one page", async ({
    queries,
    request,
  }) => {
    const devices = [];
    for (let i = 0; i < 3; i++) devices.push(await createResource(request, "Device", {}));
    // Indices refresh per resource type: wait on each device, not just the last.
    for (const id of devices) await waitSearchable(request, "Device", id);

    await queries.goto();
    await queries.builder.run("Device?_count=2");
    await queries.results.waitShown();
    await expect(queries.results.next).toBeVisible();

    const firstPage = await queries.results.rows.allInnerTexts();
    await queries.results.next.click();
    await expect
      .poll(async () => (await queries.results.rows.allInnerTexts()).join())
      .not.toBe(firstPage.join());
  });

  test("failed pagination preserves the visible page and can be retried", async ({
    queries,
  }) => {
    const pageOrigin = new URL(queries.page.url()).origin;
    const initialPath = "/Patient?_count=1&_sort=_id";
    const paginationOrigin = "http://127.0.0.1:1";
    const paginationPath =
      "/public/fhir/acme/_paging/opaque-token?_getpages=opaque%2Ftoken&_count=1";
    const paginationUrl = paginationOrigin + paginationPath;
    let initialRequests = 0;
    let paginationAttempts = 0;

    const firstPage = {
      resourceType: "Bundle",
      type: "searchset",
      total: 2,
      entry: [
        {
          fullUrl: `${pageOrigin}/Patient/patient-page-one`,
          resource: {
            resourceType: "Patient",
            id: "patient-page-one",
            name: [{ family: "First page" }],
          },
        },
      ],
      link: [
        { relation: "self", url: pageOrigin + initialPath },
        { relation: "next", url: paginationUrl },
      ],
    };
    const secondPage = {
      resourceType: "Bundle",
      type: "searchset",
      total: 2,
      entry: [
        {
          fullUrl: `${paginationOrigin}/public/fhir/acme/Patient/patient-page-two`,
          resource: {
            resourceType: "Patient",
            id: "patient-page-two",
            name: [{ family: "Second page" }],
          },
        },
      ],
      link: [{ relation: "previous", url: pageOrigin + initialPath }],
    };

    await queries.page.route("**/*", async (route) => {
      const request = route.request();
      const url = request.url();
      if (request.method() === "GET" && url === pageOrigin + initialPath) {
        initialRequests += 1;
        await route.fulfill({
          status: 200,
          contentType: "application/fhir+json",
          body: JSON.stringify(firstPage),
        });
        return;
      }
      if (url !== paginationUrl) {
        await route.continue();
        return;
      }
      if (request.method() === "OPTIONS") {
        await route.fulfill({
          status: 204,
          headers: {
            "Access-Control-Allow-Origin": pageOrigin,
            "Access-Control-Allow-Methods": "GET",
            "Access-Control-Allow-Headers": "Accept, X-Tenant-ID",
          },
        });
        return;
      }

      paginationAttempts += 1;
      const corsHeaders = {
        "Access-Control-Allow-Origin": pageOrigin,
        "Content-Type": "application/fhir+json",
      };
      if (paginationAttempts === 1) {
        await route.abort("failed");
      } else if (paginationAttempts === 2) {
        await route.fulfill({
          status: 502,
          headers: corsHeaders,
          body: JSON.stringify({
            resourceType: "OperationOutcome",
            issue: [{ diagnostics: "sensitive upstream response" }],
          }),
        });
      } else if (paginationAttempts === 3) {
        await route.fulfill({
          status: 200,
          headers: corsHeaders,
          body: "not-json",
        });
      } else {
        await route.fulfill({
          status: 200,
          headers: corsHeaders,
          body: JSON.stringify(secondPage),
        });
      }
    });

    await queries.page.evaluate(() => {
      const trackedWindow = window as typeof window & {
        __hfsDataChangedFailures?: number;
        __hfsFetchInputs?: string[];
      };
      const nativeFetch = window.fetch.bind(window);
      trackedWindow.__hfsDataChangedFailures = 0;
      trackedWindow.__hfsFetchInputs = [];
      window.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
        trackedWindow.__hfsFetchInputs?.push(
          typeof input === "string" ? input : input instanceof URL ? input.href : input.url,
        );
        return nativeFetch(input, init);
      }) as typeof window.fetch;
      document.addEventListener("hfs:data-changed", (event) => {
        const detail = (event as CustomEvent).detail;
        if (detail && detail.source === "query-results") {
          trackedWindow.__hfsDataChangedFailures =
            (trackedWindow.__hfsDataChangedFailures || 0) + 1;
        }
      });
    });

    await queries.builder.run(initialPath);
    await queries.results.waitShown();
    await expect(queries.results.next).toBeVisible();
    await expect(queries.results.error).toBeHidden();
    const visiblePage = await queries.results.visibleState();

    // A network failure leaves every visible result field and pager URL alone.
    await queries.results.next.click();
    await expect(queries.results.error).toBeVisible();
    await expect(queries.results.error).toContainText(paginationOrigin);
    await expect(queries.results.error).toContainText("HFS_BASE_URL");
    await expect(queries.results.error).not.toContainText("opaque");
    await expect.poll(() => paginationAttempts).toBe(1);
    expect(await queries.results.visibleState()).toEqual(visiblePage);
    await expect
      .poll(() =>
        queries.page.evaluate(
          () =>
            (window as typeof window & { __hfsDataChangedFailures?: number })
              .__hfsDataChangedFailures,
        ),
      )
      .toBe(1);
    expect(initialRequests).toBe(1);

    // The failure event does not recurse. A real data change repeats the last
    // successful relative path, not the failed absolute pagination URL.
    await queries.page.evaluate(() => {
      document.dispatchEvent(
        new CustomEvent("hfs:data-changed", { detail: { type: "Patient" } }),
      );
    });
    await expect.poll(() => initialRequests).toBe(2);
    await expect(queries.results.error).toBeHidden();
    expect(await queries.results.visibleState()).toEqual(visiblePage);

    // Non-2xx and malformed JSON failures are equally non-destructive. The
    // server response body and opaque query token never enter the alert.
    await queries.results.next.click();
    await expect
      .poll(() =>
        queries.page.evaluate(
          () =>
            (window as typeof window & { __hfsDataChangedFailures?: number })
              .__hfsDataChangedFailures,
        ),
      )
      .toBe(2);
    await expect(queries.results.error).not.toContainText("sensitive upstream response");
    expect(await queries.results.visibleState()).toEqual(visiblePage);

    await queries.results.next.click();
    await expect
      .poll(() =>
        queries.page.evaluate(
          () =>
            (window as typeof window & { __hfsDataChangedFailures?: number })
              .__hfsDataChangedFailures,
        ),
      )
      .toBe(3);
    expect(await queries.results.visibleState()).toEqual(visiblePage);

    // The fourth click retries the server-provided URL byte for byte. A valid
    // Bundle under a public path prefix replaces the page and clears the alert.
    await queries.results.next.click();
    await expect(queries.results.rows).toHaveCount(1);
    await expect(queries.results.rows.first()).toContainText("patient-page-two");
    await expect(queries.results.rows.first().locator("a.url")).toHaveAttribute(
      "href",
      `${paginationOrigin}/public/fhir/acme/Patient/patient-page-two`,
    );
    await expect(queries.results.openTab).toHaveAttribute("href", paginationUrl);
    await expect(queries.results.prev).toBeVisible();
    await expect(queries.results.error).toBeHidden();

    const fetchInputs = await queries.page.evaluate(
      () =>
        (window as typeof window & { __hfsFetchInputs?: string[] }).__hfsFetchInputs || [],
    );
    expect(fetchInputs.filter((input) => input === paginationUrl)).toHaveLength(4);
  });

  /// Chained search end to end (#406): the two chain directions meet on the
  /// patient in the middle — Practitioner <- Patient (generalPractitioner)
  /// <- Observation (subject) — and the combined query returns exactly it.
  test("a combined chain and _has query runs and returns the linked patient", async ({
    queries,
    request,
  }) => {
    // Unique per run: the suite may reuse a persistent dev server.
    const tag = Date.now().toString(36);
    const gp = await createResource(request, "Practitioner", {
      name: [{ family: `ChainSmith${tag}` }],
    });
    const patient = await createResource(request, "Patient", {
      name: [{ family: "ChainLinked" }],
      generalPractitioner: [{ reference: `Practitioner/${gp}` }],
    });
    const observation = await createResource(request, "Observation", {
      status: "final",
      code: { coding: [{ code: `chain-94-${tag}` }] },
      subject: { reference: `Patient/${patient}` },
    });
    // The chained query touches three indices; each refreshes on its own tick.
    await waitSearchable(request, "Practitioner", gp);
    await waitSearchable(request, "Patient", patient);
    await waitSearchable(request, "Observation", observation);

    await queries.goto();
    await queries.builder.run(
      `Patient?_has:Observation:patient:code=chain-94-${tag}&general-practitioner.name=ChainSmith${tag}`,
    );
    await queries.results.waitShown();
    await expect(queries.results.rows).toHaveCount(1);
    await expect(queries.results.rows.first()).toContainText(patient);
  });

  test("adding a condition row hydrates the builder", async ({ queries }) => {
    // The builder sections are hidden until there's a base query to parse.
    await queries.builder.setUrl("Patient");
    await queries.builder.addButton("condition").click();
    await expect(queries.builder.conditionRows).toHaveCount(1);
  });

  /* ---- chaining (#394) ------------------------------------------------- */

  test("a chained query hydrates into a forward-chain row and round-trips", async ({
    queries,
  }) => {
    await queries.builder.setUrl("Patient?general-practitioner.name=Smith");
    await expect(queries.builder.chainRows).toHaveCount(1);
    const row = queries.builder.chainRows.first();
    await expect(row.locator(".builder-row__chainref")).toHaveValue("general-practitioner");
    await expect(row.locator(".builder-row__cparam")).toHaveValue("name");
    await expect(row.locator(".builder-row__value")).toHaveValue("Smith");

    // Editing the value re-serializes the same chained key.
    await row.locator(".builder-row__value").fill("Jones");
    await expect(queries.builder.url).toHaveValue(
      "GET /Patient?general-practitioner.name=Jones",
    );
  });

  test("an explicitly typed chain keeps its :Type qualifier", async ({ queries }) => {
    await queries.builder.setUrl("Observation?subject:Patient.name:contains=ann");
    const row = queries.builder.chainRows.first();
    await expect(row.locator(".builder-row__chainref")).toHaveValue("subject");
    await expect(row.locator(".builder-row__cparam")).toHaveValue("name");
    await expect(row.locator(".builder-row__modifier")).toHaveValue("contains");
    // The registry feeds Patient into the target-type select.
    await expect
      .poll(async () => row.locator(".builder-row__ctype").inputValue())
      .toBe("Patient");

    await row.locator(".builder-row__value").fill("bob");
    await expect(queries.builder.url).toHaveValue(
      "GET /Observation?subject:Patient.name:contains=bob",
    );
  });

  test("drilling into a reference param converts the row to a chain", async ({
    queries,
  }) => {
    await queries.builder.setUrl("Patient?general-practitioner=123");
    const row = queries.builder.conditionRows.first();
    // The affordance appears once the registry metadata loads.
    const drill = queries.builder.drillButton(row);
    await expect(drill).toBeVisible();
    await drill.click();

    await expect(queries.builder.chainRows).toHaveCount(1);
    const chain = queries.builder.chainRows.first();
    await chain.locator(".builder-row__cparam").fill("name");
    await chain.locator(".builder-row__value").fill("Smith");
    await expect(queries.builder.url).toHaveValue(
      "GET /Patient?general-practitioner.name=Smith",
    );
    // The target-type select offers this param's registry targets.
    await expect
      .poll(async () => chain.locator(".builder-row__ctype option").count())
      .toBeGreaterThan(1);
  });

  test("drilling preserves a literal comma and every real OR alternative", async ({
    queries,
  }) => {
    await queries.builder.setUrl("Patient?general-practitioner=a%5C%2Cb,c");
    const row = queries.builder.conditionRows.first();
    const drill = queries.builder.drillButton(row);
    await expect(drill).toBeVisible();
    await drill.click();

    const chain = queries.builder.chainRows.first();
    await expect(chain.locator(".builder-row__value")).toHaveCount(2);
    await expect(chain.locator(".builder-row__value").nth(0)).toHaveValue("a,b");
    await expect(chain.locator(".builder-row__value").nth(1)).toHaveValue("c");
    await chain.locator(".builder-row__cparam").fill("name");
    await expect(queries.builder.url).toHaveValue(
      "GET /Patient?general-practitioner.name=a\\,b,c",
    );
  });

  test("a _has query hydrates into a reverse-chain row and round-trips", async ({
    queries,
  }) => {
    await queries.builder.setUrl("Patient?_has:Observation:patient:code=1234-5");
    await expect(queries.builder.hasRows).toHaveCount(1);
    const row = queries.builder.hasRows.first();
    await expect(row.locator(".builder-row__htype")).toHaveValue("Observation");
    await expect(row.locator(".builder-row__href")).toHaveValue("patient");
    await expect(row.locator(".builder-row__cparam")).toHaveValue("code");
    await expect(row.locator(".builder-row__value")).toHaveValue("1234-5");

    await row.locator(".builder-row__value").fill("8480-6");
    await expect(queries.builder.url).toHaveValue(
      "GET /Patient?_has:Observation:patient:code=8480-6",
    );
  });

  test("the links-here button builds a _has filter from scratch", async ({ queries }) => {
    await queries.builder.setUrl("Patient");
    await queries.builder.addButton("has").click();
    const row = queries.builder.hasRows.first();
    await row.locator(".builder-row__htype").fill("Observation");
    await row.locator(".builder-row__href").fill("patient");
    await row.locator(".builder-row__cparam").fill("code");
    await row.locator(".builder-row__value").fill("1234-5");
    await expect(queries.builder.url).toHaveValue(
      "GET /Patient?_has:Observation:patient:code=1234-5",
    );
  });

  test("a multi-level chain hydrates into hop segments and round-trips", async ({
    queries,
  }) => {
    await queries.builder.setUrl("Patient?general-practitioner.organization.name=Acme");
    await expect(queries.builder.chainRows).toHaveCount(1);
    const row = queries.builder.chainRows.first();
    const hops = row.locator(".builder-row__hopseg");
    await expect(hops).toHaveCount(2);
    await expect(hops.nth(0).locator(".builder-row__chainref")).toHaveValue(
      "general-practitioner",
    );
    await expect(hops.nth(1).locator(".builder-row__chainref")).toHaveValue("organization");
    await expect(row.locator(".builder-row__cparam")).toHaveValue("name");

    await row.locator(".builder-row__value").fill("Beta");
    await expect(queries.builder.url).toHaveValue(
      "GET /Patient?general-practitioner.organization.name=Beta",
    );
  });

  test("drilling deeper appends a hop when the leaf is a reference", async ({
    queries,
  }) => {
    await queries.builder.setUrl("Patient?general-practitioner.name=x");
    const row = queries.builder.chainRows.first();
    const leaf = row.locator(".builder-row__cparam");
    await leaf.fill("organization");
    // organization is a reference param on the target types, so the
    // drill-deeper affordance appears.
    const deeper = row.locator("[data-chain-deeper]");
    await expect(deeper).toBeVisible();
    await deeper.click();

    await expect(row.locator(".builder-row__hopseg")).toHaveCount(2);
    await leaf.fill("name");
    await row.locator(".builder-row__value").fill("Acme");
    await expect(queries.builder.url).toHaveValue(
      "GET /Patient?general-practitioner.organization.name=Acme",
    );
  });

  /* ---- OR values (#414) ------------------------------------------------ */

  test("comma OR values hydrate as stacked inputs and round-trip", async ({ queries }) => {
    await queries.builder.setUrl("Patient?name=Smith,Jones");
    const row = queries.builder.conditionRows.first();
    const values = row.locator(".builder-row__value");
    await expect(values).toHaveCount(2);
    await expect(values.nth(0)).toHaveValue("Smith");
    await expect(values.nth(1)).toHaveValue("Jones");

    await values.nth(1).fill("Garcia");
    await expect(queries.builder.url).toHaveValue("GET /Patient?name=Smith,Garcia");
  });

  test("escaped commas stay one visual value through narration, editing and Run", async ({
    queries,
    request,
  }) => {
    const tag = Date.now().toString(36);
    const literalName = `Comma,Literal-${tag}`;
    const escapedName = encodeURIComponent(literalName.replace(",", "\\,"));
    const literal = await createResource(request, "Patient", {
      name: [{ family: literalName }],
    });
    const comma = await createResource(request, "Patient", {
      name: [{ family: `Comma-${tag}` }],
    });
    const literalWord = await createResource(request, "Patient", {
      name: [{ family: `Literal-${tag}` }],
    });
    for (const id of [literal, comma, literalWord]) {
      await waitSearchable(request, "Patient", id);
    }

    await queries.goto();
    await queries.builder.setUrl(`Patient?name:exact=${escapedName}`);
    const row = queries.builder.conditionRows.first();
    const value = row.locator(".builder-row__value");
    await expect(value).toHaveCount(1);
    await expect(value).toHaveValue(literalName);
    await expect(queries.page.locator("#query-plain-text")).toContainText(
      `name is exactly “${literalName}”`,
    );
    await expect(queries.page.locator("#query-plain-text")).not.toContainText("” or “");

    const requestSent = queries.page.waitForRequest((candidate) => {
      const url = new URL(candidate.url());
      return url.pathname === "/Patient" && url.searchParams.has("name:exact");
    });
    await queries.builder.runButton.click();
    const sent = await requestSent;
    expect(new URL(sent.url()).searchParams.get("name:exact")).toBe(
      literalName.replace(",", "\\,"),
    );
    await queries.results.waitShown();
    await expect(queries.results.rows).toHaveCount(1);
    await expect(queries.results.rows.first()).toContainText(literal);

    await value.fill(`Comma,Edited-${tag}`);
    await expect(queries.builder.url).toHaveValue(
      `GET /Patient?name:exact=Comma\\,Edited-${tag}`,
    );
  });

  test("literal commas coexist with real OR values in conditions, chains and _has", async ({
    queries,
  }) => {
    await queries.builder.setUrl("Patient?name=a%5C%2Cb,c");
    let row = queries.builder.conditionRows.first();
    await expect(row.locator(".builder-row__value")).toHaveCount(2);
    await expect(row.locator(".builder-row__value").nth(0)).toHaveValue("a,b");
    await expect(row.locator(".builder-row__value").nth(1)).toHaveValue("c");
    await row.locator("[data-remove-or]").nth(1).click();
    await expect(queries.builder.url).toHaveValue("GET /Patient?name=a\\,b");

    await queries.builder.setUrl("Patient?general-practitioner.name=a%5C%2Cb,c");
    row = queries.builder.chainRows.first();
    await expect(row.locator(".builder-row__value")).toHaveCount(2);
    await expect(row.locator(".builder-row__value").nth(0)).toHaveValue("a,b");

    await queries.builder.setUrl("Patient?_has:Observation:patient:code=a%5C%2Cb,c");
    row = queries.builder.hasRows.first();
    await expect(row.locator(".builder-row__value")).toHaveCount(2);
    await expect(row.locator(".builder-row__value").nth(0)).toHaveValue("a,b");
  });

  test("malformed FHIR escapes keep the GET unchanged until corrected", async ({ queries }) => {
    await queries.builder.setUrl("Patient?name=a%5Cx");
    const row = queries.builder.conditionRows.first();
    await expect(queries.builder.error).toBeVisible();
    await expect(queries.builder.error).toContainText("invalid FHIR escape");

    await row.locator(".builder-row__key").fill("family");
    await expect(queries.builder.url).toHaveValue("Patient?name=a%5Cx");

    await row.locator(".builder-row__value").fill("a\\x");
    await expect(queries.builder.error).toBeHidden();
    await expect(queries.builder.url).toHaveValue("GET /Patient?family=a\\\\x");
  });

  test("the + or button stacks a value; the per-value × removes it", async ({
    queries,
  }) => {
    await queries.builder.setUrl("Patient?name=Smith");
    const row = queries.builder.conditionRows.first();
    await row.locator("[data-add-or]").click();
    await row.locator(".builder-row__value").nth(1).fill("Jones");
    await expect(queries.builder.url).toHaveValue("GET /Patient?name=Smith,Jones");

    await row.locator("[data-remove-or]").first().click();
    await expect(row.locator(".builder-row__value")).toHaveCount(1);
    await expect(queries.builder.url).toHaveValue("GET /Patient?name=Jones");
  });

  test("unchanged empty alternatives survive an unrelated visual edit", async ({ queries }) => {
    await queries.builder.setUrl("Patient?name=a,,b");
    const row = queries.builder.conditionRows.first();
    await expect(row.locator(".builder-row__value")).toHaveCount(3);
    await row.locator(".builder-row__key").fill("family");
    await expect(queries.builder.url).toHaveValue("GET /Patient?family=a,,b");
  });

  test("modifier values that resemble comparators remain literal", async ({ queries }) => {
    await queries.builder.setUrl("Patient?name:exact=ge1980,le1990");
    const row = queries.builder.conditionRows.first();
    const comparators = row.locator(".builder-row__comparator");
    const values = row.locator(".builder-row__value");

    await expect(row.locator(".builder-row__modifier")).toHaveValue("exact");
    await expect(comparators.nth(0)).toHaveValue("");
    await expect(comparators.nth(1)).toHaveValue("");
    await expect(values.nth(0)).toHaveValue("ge1980");
    await expect(values.nth(1)).toHaveValue("le1990");
    await expect(queries.page.locator("#query-plain-text")).toContainText(
      "name is exactly “ge1980” or “le1990”",
    );

    await values.nth(0).fill("ge1981");
    await expect(queries.builder.url).toHaveValue(
      "GET /Patient?name:exact=ge1981,le1990",
    );

    await row.locator(".builder-row__key").fill("birthdate");
    await expect(row).toHaveAttribute("data-mod-type", "date");
    await expect(row.locator(".builder-row__modifier")).toHaveValue("exact");
    await expect(comparators.nth(0)).toHaveValue("");
    await expect(comparators.nth(1)).toHaveValue("");
    await expect(values.nth(0)).toHaveValue("ge1981");
    await expect(values.nth(1)).toHaveValue("le1990");
    await expect(queries.builder.url).toHaveValue(
      "GET /Patient?birthdate:exact=ge1981,le1990",
    );
  });

  for (const activationPath of ["select", "chip"] as const) {
    test(`an unresolved hydrated string stays literal when :exact is activated by ${activationPath}`, async ({
      page,
      queries,
    }) => {
      let releaseParams!: () => void;
      const paramsGate = new Promise<void>((resolve) => {
        releaseParams = resolve;
      });
      await page.route("**/ui/queries/params?type=Patient", async (route) => {
        await paramsGate;
        await route.continue();
      });
      const patientRequests: string[] = [];
      page.on("request", (request) => {
        const url = new URL(request.url());
        if (url.pathname === "/Patient") patientRequests.push(url.search);
      });

      await queries.builder.setUrl("Patient?name=ge1980");
      const row = queries.builder.conditionRows.first();
      if (activationPath === "select") {
        await row.locator(".builder-row__modifier").selectOption("exact");
      } else {
        await row.locator("[data-toggle-mods]").click();
        await row.locator("[data-mod-chip=exact]").click();
      }

      await expect(row.locator(".builder-row__modifier")).toHaveValue("exact");
      await expect(row.locator(".builder-row__comparator")).toHaveValue("ge");
      await expect(row.locator(".builder-row__value")).toHaveValue("1980");
      await expect(queries.builder.url).toHaveValue("Patient?name=ge1980");
      await expect(queries.builder.runButton).toBeDisabled();
      await queries.builder.runButton.evaluate((button: HTMLButtonElement) => button.click());
      expect(patientRequests).toEqual([]);

      releaseParams();
      await expect(row).toHaveAttribute("data-mod-type", "string");
      await expect(row.locator(".builder-row__modifier")).toHaveValue("exact");
      await expect(row.locator(".builder-row__comparator")).toHaveValue("");
      await expect(row.locator(".builder-row__comparator")).toBeHidden();
      await expect(row.locator(".builder-row__value")).toHaveValue("ge1980");
      await expect(queries.builder.url).toHaveValue("GET /Patient?name:exact=ge1980");
      await expect(page.locator("#query-plain-text")).toContainText(
        "name is exactly “ge1980”",
      );
      await expect(queries.builder.runButton).toBeEnabled();
      expect(patientRequests).toEqual([]);
    });
  }

  for (const activationPath of ["select", "chip"] as const) {
    test(`delayed and resolved date hydration agree when :missing is activated by ${activationPath}`, async ({
      page,
      queries,
    }) => {
      let releaseParams!: () => void;
      const paramsGate = new Promise<void>((resolve) => {
        releaseParams = resolve;
      });
      const paramsRoute = "**/ui/queries/params?type=Patient";
      await page.route(paramsRoute, async (route) => {
        await paramsGate;
        await route.continue();
      });
      const patientRequests: string[] = [];
      page.on("request", (request) => {
        const url = new URL(request.url());
        if (url.pathname === "/Patient") patientRequests.push(url.search);
      });

      await queries.builder.setUrl("Patient?birthdate=ge1980-01-01");
      let row = queries.builder.conditionRows.first();
      if (activationPath === "select") {
        await row.locator(".builder-row__modifier").selectOption("missing");
      } else {
        await row.locator("[data-toggle-mods]").click();
        await row.locator("[data-mod-chip=missing]").click();
      }
      await expect(queries.builder.runButton).toBeDisabled();
      await expect(queries.builder.url).toHaveValue(
        "Patient?birthdate=ge1980-01-01",
      );
      await queries.builder.runButton.evaluate((button: HTMLButtonElement) => button.click());
      expect(patientRequests).toEqual([]);

      releaseParams();
      await expect(row).toHaveAttribute("data-mod-type", "date");
      await expect(queries.builder.runButton).toBeEnabled();
      const delayed = {
        value: await row.locator(".builder-row__value").inputValue(),
        comparator: await row.locator(".builder-row__comparator").inputValue(),
        modifier: await row.locator(".builder-row__modifier").inputValue(),
        url: await queries.builder.url.inputValue(),
        narration: await page.locator("#query-plain-text").innerText(),
      };

      await page.unroute(paramsRoute);
      await page.reload({ waitUntil: "networkidle" });
      await queries.builder.setUrl("Patient?birthdate=ge1980-01-01");
      row = queries.builder.conditionRows.first();
      await expect(row).toHaveAttribute("data-mod-type", "date");
      if (activationPath === "select") {
        await row.locator(".builder-row__modifier").selectOption("missing");
      } else {
        await row.locator("[data-toggle-mods]").click();
        await row.locator("[data-mod-chip=missing]").click();
      }
      const resolved = {
        value: await row.locator(".builder-row__value").inputValue(),
        comparator: await row.locator(".builder-row__comparator").inputValue(),
        modifier: await row.locator(".builder-row__modifier").inputValue(),
        url: await queries.builder.url.inputValue(),
        narration: await page.locator("#query-plain-text").innerText(),
      };

      expect(delayed).toEqual(resolved);
      expect(delayed).toMatchObject({
        value: "1980-01-01",
        comparator: "",
        modifier: "missing",
        url: "GET /Patient?birthdate:missing=1980-01-01",
      });
      expect(delayed.narration).toContain(
        "birthdate is present/absent “1980-01-01”",
      );
      await expect(queries.builder.runButton).toBeEnabled();
      expect(patientRequests).toEqual([]);
    });
  }

  const pendingComparatorSources = [
    {
      name: "ordered date",
      query: "Patient?birthdate=ge1980-01-01",
      modifier: "missing",
      resolvedType: "date",
      finalComparator: "le",
      finalValue: "1980-01-01",
      finalUrl: "GET /Patient?birthdate=le1980-01-01",
      narration: "birthdate is on or before “1980-01-01”",
      comparatorHidden: false,
    },
    {
      name: "non-ordered string",
      query: "Patient?name=ge1980",
      modifier: "exact",
      resolvedType: "string",
      finalComparator: "",
      finalValue: "le1980",
      finalUrl: "GET /Patient?name=le1980",
      narration: "name is “le1980”",
      comparatorHidden: true,
    },
  ] as const;

  for (const source of pendingComparatorSources) {
    for (const activationPath of ["select", "chip"] as const) {
      test(`a comparator chosen after pending ${activationPath} modifier waits for ${source.name} classification`, async ({
        context,
        page,
        queries,
      }) => {
        let releaseParams!: () => void;
        const paramsGate = new Promise<void>((resolve) => {
          releaseParams = resolve;
        });
        await page.route("**/ui/queries/params?type=Patient", async (route) => {
          await paramsGate;
          await route.continue();
        });
        await context.grantPermissions(["clipboard-read", "clipboard-write"]);
        const patientRequests: string[] = [];
        page.on("request", (request) => {
          const url = new URL(request.url());
          if (url.pathname === "/Patient") patientRequests.push(url.search);
        });

        await queries.builder.setUrl(source.query);
        const row = queries.builder.conditionRows.first();
        if (activationPath === "select") {
          await row.locator(".builder-row__modifier").selectOption(source.modifier);
        } else {
          await row.locator("[data-toggle-mods]").click();
          await row.locator(`[data-mod-chip=${source.modifier}]`).click();
        }
        await expect(queries.builder.runButton).toBeDisabled();

        await row.locator(".builder-row__comparator").selectOption("le");
        await expect(row.locator(".builder-row__modifier")).toHaveValue("");
        await expect(row.locator(".builder-row__comparator")).toHaveValue("le");
        await expect(queries.builder.url).toHaveValue(source.query);
        await queries.builder.copyButton.click();
        await expect
          .poll(async () => page.evaluate(() => navigator.clipboard.readText()))
          .toBe(source.query);
        await expect(queries.builder.runButton).toBeDisabled();
        await queries.builder.runButton.evaluate((button: HTMLButtonElement) => button.click());
        expect(patientRequests).toEqual([]);

        releaseParams();
        await expect(row).toHaveAttribute("data-mod-type", source.resolvedType);
        await expect(row.locator(".builder-row__modifier")).toHaveValue("");
        await expect(row.locator(".builder-row__comparator")).toHaveValue(
          source.finalComparator,
        );
        await expect(row.locator(".builder-row__value")).toHaveValue(
          source.finalValue,
        );
        await expect(queries.builder.url).toHaveValue(source.finalUrl);
        await expect(page.locator("#query-plain-text")).toContainText(
          source.narration,
        );
        if (source.comparatorHidden) {
          await expect(row.locator(".builder-row__comparator")).toBeHidden();
        } else {
          await expect(row.locator(".builder-row__comparator")).toBeVisible();
        }
        await expect(queries.builder.runButton).toBeEnabled();
        expect(patientRequests).toEqual([]);
      });
    }
  }

  for (const source of pendingComparatorSources) {
    test(`a direct delayed ${source.name} comparator edit waits for classification`, async ({
      context,
      page,
      queries,
    }) => {
      let releaseParams!: () => void;
      const paramsGate = new Promise<void>((resolve) => {
        releaseParams = resolve;
      });
      await page.route("**/ui/queries/params?type=Patient", async (route) => {
        await paramsGate;
        await route.continue();
      });
      await context.grantPermissions(["clipboard-read", "clipboard-write"]);
      const patientRequests: string[] = [];
      page.on("request", (request) => {
        const url = new URL(request.url());
        if (url.pathname === "/Patient") patientRequests.push(url.search);
      });

      await queries.builder.setUrl(source.query);
      const row = queries.builder.conditionRows.first();
      await row.locator(".builder-row__comparator").selectOption("le");

      await expect(queries.builder.url).toHaveValue(source.query);
      await queries.builder.copyButton.click();
      await expect
        .poll(async () => page.evaluate(() => navigator.clipboard.readText()))
        .toBe(source.query);
      await expect(queries.builder.runButton).toBeDisabled();
      await queries.builder.runButton.evaluate((button: HTMLButtonElement) => button.click());
      expect(patientRequests).toEqual([]);

      releaseParams();
      await expect(row).toHaveAttribute("data-mod-type", source.resolvedType);
      await expect(row.locator(".builder-row__comparator")).toHaveValue(
        source.finalComparator,
      );
      await expect(row.locator(".builder-row__value")).toHaveValue(
        source.finalValue,
      );
      await expect(queries.builder.url).toHaveValue(source.finalUrl);
      await expect(page.locator("#query-plain-text")).toContainText(
        source.narration,
      );
      await expect(queries.builder.runButton).toBeEnabled();
      expect(patientRequests).toEqual([]);
    });
  }

  test("a comparator on a newly added OR alternative waits for source classification", async ({
    context,
    page,
    queries,
  }) => {
    let releaseParams!: () => void;
    const paramsGate = new Promise<void>((resolve) => {
      releaseParams = resolve;
    });
    await page.route("**/ui/queries/params?type=Patient", async (route) => {
      await paramsGate;
      await route.continue();
    });
    await context.grantPermissions(["clipboard-read", "clipboard-write"]);
    const patientRequests: string[] = [];
    page.on("request", (request) => {
      const url = new URL(request.url());
      if (url.pathname === "/Patient") patientRequests.push(url.search);
    });

    await queries.builder.setUrl("Patient?name=alpha");
    const row = queries.builder.conditionRows.first();
    await row.locator("[data-add-or]").click();
    const values = row.locator(".builder-row__value");
    const comparators = row.locator(".builder-row__comparator");
    await values.nth(1).fill("1980");
    const stable = "GET /Patient?name=alpha,1980";
    await expect(queries.builder.url).toHaveValue(stable);

    await comparators.nth(1).selectOption("le");
    await expect(queries.builder.url).toHaveValue(stable);
    await queries.builder.copyButton.click();
    await expect
      .poll(async () => page.evaluate(() => navigator.clipboard.readText()))
      .toBe(stable);
    await expect(queries.builder.runButton).toBeDisabled();
    await queries.builder.runButton.evaluate((button: HTMLButtonElement) => button.click());
    expect(patientRequests).toEqual([]);

    releaseParams();
    await expect(row).toHaveAttribute("data-mod-type", "string");
    await expect(comparators.nth(0)).toHaveValue("");
    await expect(comparators.nth(1)).toHaveValue("");
    await expect(comparators.nth(0)).toBeHidden();
    await expect(comparators.nth(1)).toBeHidden();
    await expect(values.nth(0)).toHaveValue("alpha");
    await expect(values.nth(1)).toHaveValue("le1980");
    await expect(queries.builder.url).toHaveValue(
      "GET /Patient?name=alpha,le1980",
    );
    await expect(page.locator("#query-plain-text")).toContainText(
      "name is “alpha” or “le1980”",
    );
    await expect(queries.builder.runButton).toBeEnabled();
    expect(patientRequests).toEqual([]);
  });

  test("clearing the last unresolved comparator cancels classification", async ({
    context,
    page,
    queries,
  }) => {
    let releaseParams!: () => void;
    const paramsGate = new Promise<void>((resolve) => {
      releaseParams = resolve;
    });
    await page.route("**/ui/queries/params?type=Patient", async (route) => {
      await paramsGate;
      await route.continue();
    });
    await context.grantPermissions(["clipboard-read", "clipboard-write"]);
    const patientRequests: string[] = [];
    page.on("request", (request) => {
      const url = new URL(request.url());
      if (url.pathname === "/Patient") patientRequests.push(url.search);
    });

    const original = "Patient?name=ge1980";
    await queries.builder.setUrl(original);
    const row = queries.builder.conditionRows.first();
    const comparator = row.locator(".builder-row__comparator");
    await comparator.selectOption("le");
    await expect(queries.builder.url).toHaveValue(original);
    await queries.builder.copyButton.click();
    await expect
      .poll(async () => page.evaluate(() => navigator.clipboard.readText()))
      .toBe(original);
    await expect(queries.builder.runButton).toBeDisabled();

    await comparator.selectOption("");
    await expect(queries.builder.url).toHaveValue("GET /Patient?name=1980");
    await expect(queries.builder.runButton).toBeEnabled();
    expect(patientRequests).toEqual([]);

    releaseParams();
    await expect(row).toHaveAttribute("data-mod-type", "string");
    await expect(comparator).toHaveValue("");
    await expect(comparator).toBeHidden();
    await expect(row.locator(".builder-row__value")).toHaveValue("1980");
    await expect(queries.builder.url).toHaveValue("GET /Patient?name=1980");
    await expect(page.locator("#query-plain-text")).toContainText(
      "name is “1980”",
    );
    await expect(queries.builder.runButton).toBeEnabled();
    expect(patientRequests).toEqual([]);
  });

  test("pending comparator classification survives modifier toggles and a parameter transition", async ({
    page,
    queries,
  }) => {
    let releaseParams!: () => void;
    const paramsGate = new Promise<void>((resolve) => {
      releaseParams = resolve;
    });
    await page.route("**/ui/queries/params?type=Patient", async (route) => {
      await paramsGate;
      await route.continue();
    });
    const patientRequests: string[] = [];
    page.on("request", (request) => {
      const url = new URL(request.url());
      if (url.pathname === "/Patient") patientRequests.push(url.search);
    });

    const original = "Patient?name=ge1980";
    await queries.builder.setUrl(original);
    const row = queries.builder.conditionRows.first();
    const modifier = row.locator(".builder-row__modifier");
    await modifier.selectOption("exact");
    await row.locator(".builder-row__comparator").selectOption("le");
    await modifier.selectOption("exact");
    await modifier.selectOption("");
    await row.locator(".builder-row__key").fill("birthdate");

    await expect(queries.builder.url).toHaveValue(original);
    await expect(queries.builder.runButton).toBeDisabled();
    expect(patientRequests).toEqual([]);

    releaseParams();
    await expect(row).toHaveAttribute("data-mod-type", "date");
    await expect(modifier).toHaveValue("");
    await expect(row.locator(".builder-row__comparator")).toHaveValue("le");
    await expect(row.locator(".builder-row__value")).toHaveValue("1980");
    await expect(queries.builder.url).toHaveValue(
      "GET /Patient?birthdate=le1980",
    );
    await expect(queries.builder.runButton).toBeEnabled();
    expect(patientRequests).toEqual([]);
  });

  test("removing a row releases pending comparator classification", async ({
    page,
    queries,
  }) => {
    let releaseParams!: () => void;
    const paramsGate = new Promise<void>((resolve) => {
      releaseParams = resolve;
    });
    await page.route("**/ui/queries/params?type=Patient", async (route) => {
      await paramsGate;
      await route.continue();
    });
    const patientRequests: string[] = [];
    page.on("request", (request) => {
      const url = new URL(request.url());
      if (url.pathname === "/Patient") patientRequests.push(url.search);
    });

    await queries.builder.setUrl("Patient?name=ge1980");
    const row = queries.builder.conditionRows.first();
    await row.locator(".builder-row__modifier").selectOption("exact");
    await row.locator(".builder-row__comparator").selectOption("le");
    await expect(queries.builder.runButton).toBeDisabled();

    await row.locator("[data-remove-row]").click();
    await expect(queries.builder.conditionRows).toHaveCount(0);
    await expect(queries.builder.url).toHaveValue("GET /Patient");
    await expect(queries.builder.runButton).toBeEnabled();
    expect(patientRequests).toEqual([]);

    releaseParams();
    await expect(queries.builder.conditionRows).toHaveCount(0);
    await expect(queries.builder.url).toHaveValue("GET /Patient");
    await expect(queries.builder.runButton).toBeEnabled();
    expect(patientRequests).toEqual([]);
  });

  for (const activationPath of ["select", "chip"] as const) {
    test(`value edits stay transactional during pending ${activationPath} modifier activation`, async ({
      context,
      page,
      queries,
    }) => {
      let releaseParams!: () => void;
      const paramsGate = new Promise<void>((resolve) => {
        releaseParams = resolve;
      });
      await page.route("**/ui/queries/params?type=Patient", async (route) => {
        await paramsGate;
        await route.continue();
      });
      await context.grantPermissions(["clipboard-read", "clipboard-write"]);
      const patientRequests: string[] = [];
      page.on("request", (request) => {
        const url = new URL(request.url());
        if (url.pathname === "/Patient") patientRequests.push(url.search);
      });

      const original = "Patient?name=ge1980";
      await queries.builder.setUrl(original);
      const row = queries.builder.conditionRows.first();
      if (activationPath === "select") {
        await row.locator(".builder-row__modifier").selectOption("exact");
      } else {
        await row.locator("[data-toggle-mods]").click();
        await row.locator("[data-mod-chip=exact]").click();
      }
      await row.locator(".builder-row__value").fill("1981");

      await expect(queries.builder.url).toHaveValue(original);
      await queries.builder.copyButton.click();
      await expect
        .poll(async () => page.evaluate(() => navigator.clipboard.readText()))
        .toBe(original);
      await expect(queries.builder.runButton).toBeDisabled();
      await queries.builder.runButton.evaluate((button: HTMLButtonElement) => button.click());
      expect(patientRequests).toEqual([]);

      releaseParams();
      await expect(row).toHaveAttribute("data-mod-type", "string");
      await expect(row.locator(".builder-row__modifier")).toHaveValue("exact");
      await expect(row.locator(".builder-row__comparator")).toHaveValue("");
      await expect(row.locator(".builder-row__comparator")).toBeHidden();
      await expect(row.locator(".builder-row__value")).toHaveValue("ge1981");
      await expect(queries.builder.url).toHaveValue("GET /Patient?name:exact=ge1981");
      await expect(page.locator("#query-plain-text")).toContainText(
        "name is exactly “ge1981”",
      );
      await expect(queries.builder.runButton).toBeEnabled();
      expect(patientRequests).toEqual([]);
    });
  }

  const mixedComparatorOrders = [
    {
      name: "le then ge",
      query: "Patient?birthdate=le1979-12-31,ge1980-01-02",
      comparators: ["le", "ge"],
      values: ["1979-12-31", "1980-01-02"],
      narration:
        "birthdate is on or before “1979-12-31” or birthdate is on or after “1980-01-02”",
    },
    {
      name: "ge then le",
      query: "Patient?birthdate=ge1980-01-02,le1979-12-31",
      comparators: ["ge", "le"],
      values: ["1980-01-02", "1979-12-31"],
      narration:
        "birthdate is on or after “1980-01-02” or birthdate is on or before “1979-12-31”",
    },
  ];

  for (const scenario of mixedComparatorOrders) {
    test(`mixed date comparators hydrate independently: ${scenario.name}`, async ({ queries }) => {
      await queries.builder.setUrl(scenario.query);
      const row = queries.builder.conditionRows.first();
      const alternatives = row.locator(".builder-row__orvalue");
      const comparators = row.locator(".builder-row__comparator");
      const values = row.locator(".builder-row__value");

      await expect(alternatives).toHaveCount(2);
      await expect(comparators).toHaveCount(2);
      await expect(comparators.nth(0)).toHaveValue(scenario.comparators[0]);
      await expect(comparators.nth(1)).toHaveValue(scenario.comparators[1]);
      await expect(comparators.nth(0)).toBeVisible();
      await expect(comparators.nth(1)).toBeVisible();
      await expect(values.nth(0)).toHaveValue(scenario.values[0]);
      await expect(values.nth(1)).toHaveValue(scenario.values[1]);
      await values.nth(0).fill(scenario.values[0]);
      await expect(queries.builder.url).toHaveValue(`GET /${scenario.query}`);
      await expect(queries.page.locator("#query-plain-text")).toContainText(
        scenario.narration,
      );
    });
  }

  test("editing one date comparator leaves its sibling unchanged", async ({ queries }) => {
    await queries.builder.setUrl("Patient?birthdate=le1979-12-31,ge1980-01-02");
    const row = queries.builder.conditionRows.first();
    const comparators = row.locator(".builder-row__comparator");
    const values = row.locator(".builder-row__value");

    await comparators.nth(1).selectOption("gt");
    await expect(comparators.nth(0)).toHaveValue("le");
    await expect(comparators.nth(1)).toHaveValue("gt");
    await expect(queries.builder.url).toHaveValue(
      "GET /Patient?birthdate=le1979-12-31,gt1980-01-02",
    );
    await expect(queries.page.locator("#query-plain-text")).toContainText(
      "birthdate is on or before “1979-12-31” or birthdate is after “1980-01-02”",
    );

    await values.nth(0).fill("1978-12-31");
    await expect(queries.builder.url).toHaveValue(
      "GET /Patient?birthdate=le1978-12-31,gt1980-01-02",
    );
    await expect(comparators.nth(1)).toHaveValue("gt");
  });

  test("adding and removing date alternatives keeps comparators with their values", async ({
    queries,
  }) => {
    await queries.builder.setUrl("Patient?birthdate=le1979-12-31");
    const row = queries.builder.conditionRows.first();

    await row.locator("[data-add-or]").click();
    const alternatives = row.locator(".builder-row__orvalue");
    await expect(alternatives).toHaveCount(2);
    await expect(alternatives.nth(1).locator(".builder-row__comparator")).toHaveValue("");
    await alternatives.nth(1).locator(".builder-row__comparator").selectOption("ge");
    await alternatives.nth(1).locator(".builder-row__value").fill("1980-01-02");
    await expect(queries.builder.url).toHaveValue(
      "GET /Patient?birthdate=le1979-12-31,ge1980-01-02",
    );

    await alternatives.nth(0).locator("[data-remove-or]").click();
    await expect(alternatives).toHaveCount(1);
    await expect(alternatives.first().locator(".builder-row__comparator")).toHaveValue("ge");
    await expect(alternatives.first().locator(".builder-row__value")).toHaveValue(
      "1980-01-02",
    );
    await expect(queries.builder.url).toHaveValue("GET /Patient?birthdate=ge1980-01-02");
  });

  test("changing a date parameter clears every incompatible OR comparator before Run", async ({
    page,
    queries,
  }) => {
    const patientRequests: string[] = [];
    page.on("request", (request) => {
      const url = new URL(request.url());
      if (url.pathname === "/Patient") patientRequests.push(url.search);
    });

    await queries.builder.setUrl(
      "Patient?birthdate=ge1980-01-02,le1990-12-31",
    );
    const row = queries.builder.conditionRows.first();
    await expect(row).toHaveAttribute("data-mod-type", "date");

    await row.locator(".builder-row__key").fill("name");
    await expect(row).toHaveAttribute("data-mod-type", "string");
    await expect(row.locator(".builder-row__comparator").nth(0)).toHaveValue("");
    await expect(row.locator(".builder-row__comparator").nth(1)).toHaveValue("");
    await expect(row.locator(".builder-row__comparator").nth(0)).toBeHidden();
    await expect(row.locator(".builder-row__comparator").nth(1)).toBeHidden();
    await expect(row.locator(".builder-row__value").nth(0)).toHaveValue("1980-01-02");
    await expect(row.locator(".builder-row__value").nth(1)).toHaveValue("1990-12-31");
    await expect(queries.builder.url).toHaveValue(
      "GET /Patient?name=1980-01-02,1990-12-31",
    );
    await expect(page.locator("#query-plain-text")).toContainText(
      "name is “1980-01-02” or “1990-12-31”",
    );
    expect(patientRequests).toEqual([]);

    const sentRequest = page.waitForRequest((request) => {
      const url = new URL(request.url());
      return url.pathname === "/Patient" && url.searchParams.has("name");
    });
    await queries.builder.runButton.click();
    const sent = new URL((await sentRequest).url());
    expect(sent.searchParams.get("name")).toBe("1980-01-02,1990-12-31");
    expect(patientRequests).toHaveLength(1);
  });

  test("known string prefix-lookalikes hydrate literally while unknown parameters stay permissive", async ({
    page,
    queries,
  }) => {
    await queries.builder.setUrl("Patient?name=ge1980,le1990");
    let row = queries.builder.conditionRows.first();
    await expect(row).toHaveAttribute("data-mod-type", "string");
    await expect(row.locator(".builder-row__comparator").nth(0)).toHaveValue("");
    await expect(row.locator(".builder-row__comparator").nth(1)).toHaveValue("");
    await expect(row.locator(".builder-row__comparator").nth(0)).toBeHidden();
    await expect(row.locator(".builder-row__value").nth(0)).toHaveValue("ge1980");
    await expect(row.locator(".builder-row__value").nth(1)).toHaveValue("le1990");
    await expect(queries.builder.url).toHaveValue("Patient?name=ge1980,le1990");
    await expect(page.locator("#query-plain-text")).toContainText(
      "name is “ge1980” or “le1990”",
    );

    await queries.builder.setUrl("Patient?unregistered=ge1980");
    row = queries.builder.conditionRows.first();
    await expect(row).toHaveAttribute("data-mod-type", "");
    await expect(row.locator(".builder-row__comparator")).toHaveValue("ge");
    await expect(row.locator(".builder-row__comparator")).toBeVisible();
    await expect(row.locator(".builder-row__value")).toHaveValue("1980");
    await expect(queries.builder.url).toHaveValue("Patient?unregistered=ge1980");

    await queries.builder.setUrl("Patient?link:Patient.name=ge1980");
    row = queries.builder.chainRows.first();
    await expect(row).toHaveAttribute("data-mod-type", "string");
    await expect(row.locator(".builder-row__comparator")).toBeHidden();
    await expect(row.locator(".builder-row__value")).toHaveValue("ge1980");
    await expect(page.locator("#query-plain-text")).toContainText("name is “ge1980”");
    await expect(page.locator("#query-plain-text")).not.toContainText("on or after");

    await queries.builder.setUrl("Patient?_has:Observation:patient:code=ge1980");
    row = queries.builder.hasRows.first();
    await expect(row).toHaveAttribute("data-mod-type", "token");
    await expect(row.locator(".builder-row__comparator")).toBeHidden();
    await expect(row.locator(".builder-row__value")).toHaveValue("ge1980");
    await expect(page.locator("#query-plain-text")).toContainText("code is “ge1980”");
    await expect(page.locator("#query-plain-text")).not.toContainText("on or after");
  });

  test("forward and reverse chain leaf changes clear incompatible comparators", async ({
    queries,
  }) => {
    await queries.builder.setUrl("Patient?link:Patient.birthdate=ge1980-01-02");
    let row = queries.builder.chainRows.first();
    await expect(row).toHaveAttribute("data-mod-type", "date");
    await row.locator(".builder-row__cparam").fill("name");
    await expect(row).toHaveAttribute("data-mod-type", "string");
    await expect(row.locator(".builder-row__comparator")).toHaveValue("");
    await expect(row.locator(".builder-row__comparator")).toBeHidden();
    await expect(queries.builder.url).toHaveValue(
      "GET /Patient?link:Patient.name=1980-01-02",
    );

    await queries.builder.setUrl(
      "Patient?_has:Observation:patient:date=le2020-01-01",
    );
    row = queries.builder.hasRows.first();
    await expect(row).toHaveAttribute("data-mod-type", "date");
    await row.locator(".builder-row__cparam").fill("code");
    await expect(row).toHaveAttribute("data-mod-type", "token");
    await expect(row.locator(".builder-row__comparator")).toHaveValue("");
    await expect(row.locator(".builder-row__comparator")).toBeHidden();
    await expect(queries.builder.url).toHaveValue(
      "GET /Patient?_has:Observation:patient:code=2020-01-01",
    );
  });

  test("Run stays blocked while a parameter transition awaits metadata", async ({
    page,
    queries,
  }) => {
    let releaseParams!: () => void;
    let delayed = true;
    const paramsGate = new Promise<void>((resolve) => {
      releaseParams = resolve;
    });
    await page.route("**/ui/queries/params?type=Patient", async (route) => {
      if (delayed) await paramsGate;
      await route.continue();
    });
    const patientRequests: string[] = [];
    page.on("request", (request) => {
      const url = new URL(request.url());
      if (url.pathname === "/Patient") patientRequests.push(url.search);
    });

    await queries.builder.setUrl("Patient?birthdate=ge1980-01-02");
    const row = queries.builder.conditionRows.first();
    await row.locator(".builder-row__key").fill("name");
    await expect(queries.builder.runButton).toBeDisabled();
    await queries.builder.runButton.evaluate((button: HTMLButtonElement) => button.click());
    expect(patientRequests).toEqual([]);

    delayed = false;
    releaseParams();
    await expect(row).toHaveAttribute("data-mod-type", "string");
    await expect(queries.builder.url).toHaveValue("GET /Patient?name=1980-01-02");
    await expect(queries.builder.runButton).toBeEnabled();
    expect(patientRequests).toEqual([]);
  });

  test("a delayed known-string hydration stays literal across a direct parameter edit", async ({
    page,
    queries,
  }) => {
    let releaseParams!: () => void;
    const paramsGate = new Promise<void>((resolve) => {
      releaseParams = resolve;
    });
    await page.route("**/ui/queries/params?type=Patient", async (route) => {
      await paramsGate;
      await route.continue();
    });

    await queries.builder.setUrl("Patient?name=ge1980");
    const row = queries.builder.conditionRows.first();
    await row.locator(".builder-row__key").fill("family");
    await expect(queries.builder.runButton).toBeDisabled();

    releaseParams();
    await expect(row).toHaveAttribute("data-mod-type", "string");
    await expect(row.locator(".builder-row__comparator")).toHaveValue("");
    await expect(row.locator(".builder-row__comparator")).toBeHidden();
    await expect(row.locator(".builder-row__value")).toHaveValue("ge1980");
    await expect(queries.builder.url).toHaveValue("GET /Patient?family=ge1980");
    await expect(page.locator("#query-plain-text")).toContainText("family is “ge1980”");
    await expect(queries.builder.runButton).toBeEnabled();
  });

  test("delayed known-string hydration stays literal across chain leaf edits", async ({
    page,
    queries,
  }) => {
    let releasePatient!: () => void;
    const patientGate = new Promise<void>((resolve) => {
      releasePatient = resolve;
    });
    await page.route("**/ui/queries/params?type=Patient", async (route) => {
      await patientGate;
      await route.continue();
    });

    await queries.builder.setUrl("Patient?link:Patient.name=ge1980");
    let row = queries.builder.chainRows.first();
    await row.locator(".builder-row__cparam").fill("family");
    await expect(queries.builder.runButton).toBeDisabled();
    releasePatient();
    await expect(row).toHaveAttribute("data-mod-type", "string");
    await expect(row.locator(".builder-row__comparator")).toBeHidden();
    await expect(row.locator(".builder-row__value")).toHaveValue("ge1980");
    await expect(queries.builder.url).toHaveValue(
      "GET /Patient?link:Patient.family=ge1980",
    );

    await page.reload({ waitUntil: "networkidle" });
    let releaseObservation!: () => void;
    const observationGate = new Promise<void>((resolve) => {
      releaseObservation = resolve;
    });
    await page.route("**/ui/queries/params?type=Observation", async (route) => {
      await observationGate;
      await route.continue();
    });

    await queries.builder.setUrl("Patient?_has:Observation:patient:code=ge1980");
    row = queries.builder.hasRows.first();
    await row.locator(".builder-row__cparam").fill("category");
    await expect(queries.builder.runButton).toBeDisabled();
    releaseObservation();
    await expect(row).toHaveAttribute("data-mod-type", "token");
    await expect(row.locator(".builder-row__comparator")).toBeHidden();
    await expect(row.locator(".builder-row__value")).toHaveValue("ge1980");
    await expect(queries.builder.url).toHaveValue(
      "GET /Patient?_has:Observation:patient:category=ge1980",
    );
  });

  test("number and quantity parameters retain ordered comparator controls", async ({
    queries,
  }) => {
    await queries.builder.setUrl("RiskAssessment?probability=ge0.5");
    let row = queries.builder.conditionRows.first();
    await expect(row).toHaveAttribute("data-mod-type", "number");
    await expect(row.locator(".builder-row__comparator")).toHaveValue("ge");
    await expect(row.locator(".builder-row__comparator")).toBeVisible();
    await expect(row.locator(".builder-row__value")).toHaveValue("0.5");
    await row.locator(".builder-row__comparator").selectOption("gt");
    await expect(queries.builder.url).toHaveValue(
      "GET /RiskAssessment?probability=gt0.5",
    );

    await queries.builder.setUrl("Observation?value-quantity=le5");
    row = queries.builder.conditionRows.first();
    await expect(row).toHaveAttribute("data-mod-type", "quantity");
    await expect(row.locator(".builder-row__comparator")).toHaveValue("le");
    await expect(row.locator(".builder-row__comparator")).toBeVisible();
    await expect(row.locator(".builder-row__value")).toHaveValue("5");
    await row.locator(".builder-row__comparator").selectOption("ge");
    await expect(queries.builder.url).toHaveValue(
      "GET /Observation?value-quantity=ge5",
    );
  });

  test("date comparators stay available in forward and reverse chains", async ({
    queries,
  }) => {
    await queries.builder.setUrl("Patient?link:Patient.birthdate=1980-01-02");
    const chain = queries.builder.chainRows.first();
    const chainComparator = chain.locator(".builder-row__comparator");
    await expect(chain).toHaveAttribute("data-mod-type", "date");
    await expect(chainComparator).toBeVisible();
    await chainComparator.selectOption("ge");
    await expect(queries.builder.url).toHaveValue(
      "GET /Patient?link:Patient.birthdate=ge1980-01-02",
    );

    await queries.builder.setUrl("Patient?_has:Observation:patient:date=2020-01-01");
    const reverseChain = queries.builder.hasRows.first();
    const reverseComparator = reverseChain.locator(".builder-row__comparator");
    await expect(reverseChain).toHaveAttribute("data-mod-type", "date");
    await expect(reverseComparator).toBeVisible();
    await reverseComparator.selectOption("le");
    await expect(queries.builder.url).toHaveValue(
      "GET /Patient?_has:Observation:patient:date=le2020-01-01",
    );
  });

  /* ---- modifier panel (#415) ------------------------------------------ */

  test("the modifier select gates to the parameter type", async ({ queries }) => {
    await queries.builder.setUrl("Patient?name=x");
    const row = queries.builder.conditionRows.first();
    // string param: :contains offered, date prefixes are not
    await expect
      .poll(async () => row.locator(".builder-row__modifier option").allTextContents())
      .toContain(":contains");
    const opts = await row.locator(".builder-row__modifier option").allTextContents();
    expect(opts).not.toContain("ge");
    expect(opts).not.toContain(":in");
  });

  test("the MODIFY panel explains its chips", async ({ queries }) => {
    await queries.builder.setUrl("Patient?name=ann");
    const row = queries.builder.conditionRows.first();
    await expect(row).toHaveAttribute("data-mod-type", "string");
    await row.locator("[data-toggle-mods]").click();
    const panel = row.locator(".builder-row__modpanel");
    await expect(panel).toBeVisible();
    const chip = panel.locator("[data-mod-chip=contains]");
    await expect(chip).toContainText(":contains");
    await expect(chip).toContainText("anywhere");
  });

  test("choosing a comparator clears the active modifier chip", async ({ queries }) => {
    await queries.builder.setUrl("Patient?birthdate=1980-01-02");
    const row = queries.builder.conditionRows.first();
    await row.locator("[data-toggle-mods]").click();
    const missingChip = row.locator("[data-mod-chip=missing]");

    await missingChip.click();
    await expect(missingChip).toHaveAttribute("aria-pressed", "true");
    await row.locator(".builder-row__comparator").selectOption("ge");

    await expect(row.locator(".builder-row__modifier")).toHaveValue("");
    await expect(missingChip).toHaveAttribute("aria-pressed", "false");
    await expect(queries.builder.url).toHaveValue("GET /Patient?birthdate=ge1980-01-02");
  });

  const modifierLayoutScenarios = [
    {
      name: "desktop string with two OR values in Spanish",
      width: 1280,
      lang: "es",
      query: "Patient?name=ann,anne",
      modType: "string",
      requiredChip: "contains",
      interaction: {
        chip: "contains",
        url: "GET /Patient?name:contains=ann,anne",
      },
    },
    {
      name: "token just above the responsive breakpoint in German",
      width: 901,
      lang: "de",
      query: "Patient?gender=male",
      modType: "token",
      requiredChip: "of-type",
    },
    {
      name: "date alternatives with independent comparators",
      width: 901,
      lang: "en",
      query: "Patient?birthdate=le1979-12-31,ge1980-01-02",
      modType: "date",
      requiredChip: "missing",
    },
    {
      name: "unknown parameter at a narrow width in English",
      width: 760,
      lang: "en",
      query: "Patient?custom-param=value",
      requiredChip: "above",
      wraps: true,
    },
  ];

  for (const scenario of modifierLayoutScenarios) {
    test(`the MODIFY panel preserves layout for ${scenario.name}`, async ({
      page,
      queries,
    }) => {
      await page.setViewportSize({ width: scenario.width, height: 900 });
      await page.goto(`/ui/queries?lang=${scenario.lang}`, { waitUntil: "networkidle" });
      await expect(page.locator("html")).toHaveAttribute("lang", scenario.lang);
      await queries.builder.setUrl(scenario.query);

      const row = queries.builder.conditionRows.first();
      if ("modType" in scenario) {
        await expect(row).toHaveAttribute("data-mod-type", scenario.modType);
      }

      const primaryControls = row.locator(
        ".builder-row__key, .builder-row__modifier, .builder-row__values, " +
          ".builder-row__or, .builder-row__adv, [data-remove-row]",
      );
      const primaryBounds = () =>
        primaryControls.evaluateAll((elements) => {
          const rowRect = elements[0].closest(".builder-row")!.getBoundingClientRect();
          return elements.map((element) => {
            const rect = element.getBoundingClientRect();
            return [
              rect.x - rowRect.x,
              rect.y - rowRect.y,
              rect.width,
              rect.height,
            ].map(Math.round);
          });
        });

      const before = await primaryBounds();
      const toggle = row.locator("[data-toggle-mods]");
      await toggle.click();
      const panel = row.locator(".builder-row__modpanel");
      await expect(panel).toBeVisible();
      await expect(panel.locator(`[data-mod-chip='${scenario.requiredChip}']`)).toBeVisible();

      expect(await primaryBounds()).toEqual(before);
      const geometry = await row.evaluate((element) => {
        const rowRect = element.getBoundingClientRect();
        const relativeBox = (node: Element, parentRect = rowRect) => {
          const rect = node.getBoundingClientRect();
          return {
            x: rect.x - parentRect.x,
            y: rect.y - parentRect.y,
            width: rect.width,
            height: rect.height,
          };
        };
        const panel = element.querySelector(".builder-row__modpanel")!;
        const panelBox = relativeBox(panel);
        const panelClientRect = panel.getBoundingClientRect();
        const visibleLeaves = Array.from(
          element.querySelectorAll(
            ".builder-row__key, .builder-row__modifier, .builder-row__comparator, " +
              ".builder-row__value, " +
              ".builder-row__remove--or, .builder-row__or, .builder-row__adv, [data-remove-row]",
          ),
        ).filter((leaf) => {
          const style = getComputedStyle(leaf);
          return style.display !== "none" && style.visibility !== "hidden";
        });
        const leafBoxes = visibleLeaves.map((leaf) => relativeBox(leaf));
        const chips = Array.from(panel.querySelectorAll(".builder-row__modchip")).map((chip) =>
          relativeBox(chip, panelClientRect),
        );
        const overlaps = (
          first: ReturnType<typeof relativeBox>,
          second: ReturnType<typeof relativeBox>,
        ) =>
          first.x < second.x + second.width - 1 &&
          first.x + first.width > second.x + 1 &&
          first.y < second.y + second.height - 1 &&
          first.y + first.height > second.y + 1;
        const pairwiseClear = (boxes: ReturnType<typeof relativeBox>[]) =>
          boxes.every((box, index) => boxes.slice(index + 1).every((other) => !overlaps(box, other)));
        const within = (box: ReturnType<typeof relativeBox>, width: number, height: number) =>
          box.x >= -1 &&
          box.y >= -1 &&
          box.x + box.width <= width + 1 &&
          box.y + box.height <= height + 1;
        const valueWidths = Array.from(element.querySelectorAll(".builder-row__value")).map(
          (value) => value.getBoundingClientRect().width,
        );
        return {
          panelBelowControls:
            panelBox.y >= Math.max(...leafBoxes.map((box) => box.y + box.height)) - 1,
          panelFullWidth:
            Math.abs(panelBox.x) <= 1 && Math.abs(panelBox.width - rowRect.width) <= 1,
          noOverflow:
            element.scrollWidth <= element.clientWidth + 1 &&
            panel.scrollWidth <= panel.clientWidth + 1,
          controlsDoNotOverlap: pairwiseClear(leafBoxes),
          chipsDoNotOverlap: pairwiseClear(chips),
          controlsContained: leafBoxes.every((box) =>
            within(box, rowRect.width, rowRect.height),
          ),
          panelContained: within(panelBox, rowRect.width, rowRect.height),
          chipsContained: chips.every((box) =>
            within(box, panelClientRect.width, panelClientRect.height),
          ),
          minimumValueWidth: Math.min(...valueWidths),
          chipsWrap: chips.some((chip) => chip.y >= chips[0].y + chips[0].height - 1),
        };
      });

      expect(geometry).toMatchObject({
        panelBelowControls: true,
        panelFullWidth: true,
        noOverflow: true,
        controlsDoNotOverlap: true,
        chipsDoNotOverlap: true,
        controlsContained: true,
        panelContained: true,
        chipsContained: true,
      });
      expect(geometry.minimumValueWidth).toBeGreaterThan(120);
      if ("wraps" in scenario) expect(geometry.chipsWrap).toBe(true);

      if ("interaction" in scenario) {
        const chip = panel.locator(`[data-mod-chip='${scenario.interaction.chip}']`);
        await chip.click();
        await expect(chip).toHaveAttribute("aria-pressed", "true");
        await expect(row.locator(".builder-row__modifier")).toHaveValue(
          scenario.interaction.chip,
        );
        await expect(queries.builder.url).toHaveValue(scenario.interaction.url);
      }

      await toggle.click();
      await expect(panel).toBeHidden();
      expect(await primaryBounds()).toEqual(before);
    });
  }

  /* ---- results sort + typed columns (#416) ---------------------------- */

  test("typed default columns render and the sort control re-runs the query", async ({
    queries,
    request,
  }) => {
    await createResource(request, "Patient", {
      name: [{ family: "Sortable" }],
      gender: "female",
      birthDate: "1980-01-01",
    });
    await queries.goto();
    await queries.builder.run("Patient?name=Sortable");
    await queries.results.waitShown();

    // Patient renders its typed default columns without any _elements.
    const headers = queries.page.locator("#query-results-head th");
    await expect(headers).toContainText(["id", "name", "gender", "birthDate"]);

    const sort = queries.page.locator("#query-results-sort");
    await sort.selectOption("-_lastUpdated");
    await expect(queries.page.locator("#query-results-open")).toHaveAttribute(
      "href",
      "/Patient?name=Sortable&_sort=-_lastUpdated",
    );
  });

  /* ---- related data (#396) -------------------------------------------- */

  test("_include and _revinclude hydrate as structured related-data rows", async ({
    queries,
  }) => {
    await queries.builder.setUrl(
      "Patient?_include=Patient:general-practitioner&_revinclude:iterate=Observation:patient",
    );
    const rows = queries.page.locator("#builder-includes .builder-row--include");
    await expect(rows).toHaveCount(2);

    const inc = rows.nth(0);
    await expect(inc.locator(".builder-row__itype")).toHaveValue("Patient");
    await expect(inc.locator(".builder-row__iparam")).toHaveValue("general-practitioner");

    const rev = rows.nth(1);
    await expect(rev.locator(".builder-row__itype")).toHaveValue("Observation");
    await expect(rev.locator(".builder-row__iparam")).toHaveValue("patient");
    await expect(rev.locator("[data-toggle-iterate]")).toHaveAttribute("aria-pressed", "true");

    // Round-trip: toggling iterate off on the revinclude re-serializes.
    await rev.locator("[data-toggle-iterate]").click();
    await expect(queries.builder.url).toHaveValue(
      "GET /Patient?_include=Patient:general-practitioner&_revinclude=Observation:patient",
    );
  });

  test("the related-data buttons build includes from scratch", async ({ queries }) => {
    await queries.builder.setUrl("Patient");
    await queries.builder.addButton("include-rev").click();
    const row = queries.page.locator("#builder-includes .builder-row--include").first();
    await row.locator(".builder-row__itype").fill("Observation");
    await row.locator(".builder-row__iparam").fill("subject");
    await expect(queries.builder.url).toHaveValue(
      "GET /Patient?_revinclude=Observation:subject",
    );

    await queries.builder.addButton("include-fwd").click();
    const inc = queries.page.locator("#builder-includes .builder-row--include").nth(1);
    // The source type defaults to the base type.
    await expect(inc.locator(".builder-row__itype")).toHaveValue("Patient");
    await inc.locator(".builder-row__iparam").fill("general-practitioner");
    await expect(queries.builder.url).toHaveValue(
      "GET /Patient?_revinclude=Observation:subject&_include=Patient:general-practitioner",
    );
  });

  /* ---- in plain English (#395) ---------------------------------------- */

  test("the plain-English line narrates conditions, chains, _has and includes", async ({
    queries,
  }) => {
    await queries.builder.setUrl(
      "Patient?name:contains=Smith,Jones&birthdate=ge1980-01-01" +
        "&general-practitioner.name=Ann&_has:Observation:patient:code=1234-5" +
        "&_include:iterate=Patient:general-practitioner&_count=20",
    );
    const text = queries.page.locator("#query-plain-text");
    await expect(queries.page.locator("#query-plain")).toBeVisible();
    await expect(text).toContainText("Find Patient records");
    await expect(text).toContainText("name contains “Smith” or “Jones”");
    await expect(text).toContainText("birthdate is on or after “1980-01-01”");
    await expect(text).toContainText("general-practitioner’s name is “Ann”");
    await expect(text).toContainText("related Observation whose code is “1234-5”");
    await expect(text).toContainText("Also returning the general-practitioner of each Patient (repeatedly)");
    await expect(text).toContainText("Showing 20 per page");

    // The narration follows edits made through the rows.
    const row = queries.builder.conditionRows.first();
    await row.locator(".builder-row__value").first().fill("Lopez");
    await expect(text).toContainText("name contains “Lopez” or “Jones”");
  });

  test("picking a type swaps in that type's parameter datalist", async ({ queries }) => {
    await queries.railItem("Patient").click();
    // /ui/queries/params fills #param-options for the picked type.
    await expect.poll(async () => queries.builder.paramOptions.count()).toBeGreaterThan(0);
  });

  // Picking a type updates the URL (and back navigates) without a full
  // reload — the click handler is an enhancement over the rail's real
  // <a href> (#541).
  test("picking a rail type updates the URL and back navigates", async ({ queries, page }) => {
    await queries.railItem("Observation").click();
    await expect(page).toHaveURL(/\/ui\/queries\?type=Observation/);
    await expect(queries.railItem("Observation")).toHaveAttribute("aria-current", "true");

    await page.goBack();
    await expect(page).toHaveURL(/\/ui\/queries$/);
  });

  test("a run is recorded under the Recent disclosure", async ({ queries, request }) => {
    await createResource(request, "Patient", { name: [{ family: "Recent" }] });
    await queries.goto();
    await queries.builder.run("Patient?name=Recent");
    await queries.results.waitShown();

    await queries.builder.recentToggle.click();
    await expect(queries.builder.recentPanel).toContainText(/Patient/);
  });

  test("escaped commas survive Copy, Saved and Recent reloads", async ({
    context,
    page,
    queries,
  }) => {
    const tag = Date.now().toString(36);
    const name = `Escaped comma ${tag}`;
    const query = `Patient?name:exact=Copy%5C%2C${tag}`;
    const getQuery = `GET /${query}`;
    await context.grantPermissions(["clipboard-read", "clipboard-write"]);

    await queries.builder.setUrl(query);
    await queries.builder.copyButton.click();
    await expect
      .poll(async () => page.evaluate(() => navigator.clipboard.readText()))
      .toBe(query);

    await queries.builder.nameInput.fill(name);
    await queries.builder.saveButton.click();
    await expect(queries.savedList).toContainText(name);

    await page.reload({ waitUntil: "networkidle" });
    await queries.builder.recentToggle.click();
    await queries.builder.recentPanel.locator("[data-saved-load]", { hasText: name }).click();
    await expect(queries.builder.url).toHaveValue(getQuery);
    await expect(queries.builder.conditionRows.first().locator(".builder-row__value")).toHaveValue(
      `Copy,${tag}`,
    );

    const recentSaved = page.waitForResponse((response) => {
      const request = response.request();
      return (
        new URL(response.url()).pathname === "/_user/settings" &&
        request.method() === "PATCH" &&
        response.ok()
      );
    });
    const requestSent = page.waitForRequest((candidate) => {
      const url = new URL(candidate.url());
      return url.pathname === "/Patient" && url.searchParams.has("name:exact");
    });
    await queries.builder.runButton.click();
    const [, sent] = await Promise.all([
      recentSaved,
      requestSent,
      queries.results.waitShown(),
    ]);
    expect(new URL(sent.url()).searchParams.get("name:exact")).toBe(`Copy\\,${tag}`);
    await page.reload({ waitUntil: "networkidle" });
    await queries.builder.recentToggle.click();
    await queries.builder.recentPanel.getByRole("button", { name: getQuery, exact: true }).click();
    await expect(queries.builder.url).toHaveValue(getQuery);
    await expect(queries.builder.conditionRows.first().locator(".builder-row__value")).toHaveValue(
      `Copy,${tag}`,
    );
  });
});
