import { holdSearches, searchLifecycleTests } from "../pages/search-lifecycle";
import { test, expect } from "../pages/fixtures";
import AxeBuilder from "@axe-core/playwright";
import { axeSummary } from "../pages/axe";
import { createResource, waitSearchable, deleteResources } from "../pages/api";
import { seedSavedQuery } from "../pages/saved-search";

function catalogHtml(
  params: Array<{ code: string; type: string; targets?: string[] }>,
): string {
  const options = params.map(
    ({ code, type, targets = [] }) =>
      `<option value="${code}" data-type="${type}" data-targets="${targets.join(",")}"></option>`,
  );
  return `<datalist id="param-options">${options.join("")}</datalist>`;
}

// The shared visual builder on Search (/ui/search): the shared query builder and the
// in-page results table — run a query, add builder rows, page through results,
// and see the datalist of parameters swap per type.
//
// search-builder.js serves both active workspaces. Builder behavior does not
// depend on per-user settings; only Saved/Recent persistence requires them.

test.describe("query builder", () => {
  test.beforeEach(async ({ search }) => {
    await search.gotoBuilder();
  });

  test("running a query shows the results table with a total", async ({ search, request }) => {
    await createResource(request, "Patient", { name: [{ family: "QueryA" }] });
    await createResource(request, "Patient", { name: [{ family: "QueryB" }] });

    await search.gotoBuilder();
    await search.builder.run("Patient");
    await search.results.waitShown();
    await expect(search.results.rows.first()).toBeVisible();
    await expect(search.results.meta).toContainText(/\d/);
  });

  test("the results table pages when a query spans more than one page", async ({
    search,
    request,
  }) => {
    const devices = [];
    for (let i = 0; i < 3; i++) devices.push(await createResource(request, "Device", {}));
    // Indices refresh per resource type: wait on each device, not just the last.
    for (const id of devices) await waitSearchable(request, "Device", id);

    await search.gotoBuilder();
    await search.builder.run("Device?_count=2");
    await search.results.waitShown();
    await expect(search.results.next).toBeVisible();

    const firstPage = await search.results.rows.allInnerTexts();
    await search.results.next.click();
    await expect
      .poll(async () => (await search.results.rows.allInnerTexts()).join())
      .not.toBe(firstPage.join());
  });

  // The header reports the match count, not the page size: the page asks the
  // server for `_total=accurate` on the wire while the typed query stays as
  // typed. An explicit `_total=none` is respected, and the header then says
  // the count is partial while a next page exists (#1003).
  test("the results header shows the match count, not the page size", async ({
    search,
    request,
  }) => {
    const family = `Total${Date.now()}`;
    const ids = [];
    for (let i = 0; i < 3; i++) {
      ids.push(await createResource(request, "Patient", { name: [{ family }] }));
    }
    for (const id of ids) await waitSearchable(request, "Patient", id);

    await search.gotoBuilder();
    await search.builder.run(`Patient?family=${family}&_count=2`);
    await search.results.waitShown();
    await expect(search.results.rows).toHaveCount(2);
    await expect(search.results.meta).toHaveText(/^3 results/);
    await expect(search.results.next).toBeVisible();
    // The URL box keeps the exact text the user typed — `run()` fills it
    // verbatim and never re-normalizes with a "GET " prefix (that only
    // happens when loading a query from Recent) — confirming `withTotal`
    // only touches the wire request, not what is shown here.
    await expect(search.builder.url).toHaveValue(`Patient?family=${family}&_count=2`);

    await search.builder.run(`Patient?family=${family}&_count=2&_total=none`);
    await search.results.waitShown();
    await expect(search.results.rows).toHaveCount(2);
    await expect(search.results.meta).toHaveText(/^2\+ results/);
  });

  test("failed pagination preserves the visible page and can be retried", async ({
    search,
  }) => {
    const pageOrigin = new URL(search.page.url()).origin;
    const initialPath = "/Patient?_count=1&_sort=_id";
    const paginationOrigin = "http://127.0.0.1:1";
    const paginationPath =
      "/public/fhir/acme/_paging/opaque-token?_getpages=opaque%2Ftoken&_count=1";
    const paginationUrl = paginationOrigin + paginationPath;
    // #1003: every fetch the page makes asks for `_total=accurate` on the
    // wire (neither URL above already carries a `_total=`), so the routes
    // below must match what actually goes out, not the bare server links.
    const initialUrlWithTotal = pageOrigin + initialPath + "&_total=accurate";
    const paginationUrlWithTotal = paginationUrl + "&_total=accurate";
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

    await search.page.route("**/*", async (route) => {
      const request = route.request();
      const url = request.url();
      if (request.method() === "GET" && url === initialUrlWithTotal) {
        initialRequests += 1;
        await route.fulfill({
          status: 200,
          contentType: "application/fhir+json",
          body: JSON.stringify(firstPage),
        });
        return;
      }
      if (url !== paginationUrlWithTotal) {
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

    await search.page.evaluate(() => {
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

    await search.builder.run(initialPath);
    await search.results.waitShown();
    await expect(search.results.next).toBeVisible();
    await expect(search.results.error).toBeHidden();
    const visiblePage = await search.results.visibleState();

    // A network failure leaves every visible result field and pager URL alone.
    await search.results.next.click();
    await expect(search.results.error).toBeVisible();
    await expect(search.results.error).toContainText(paginationOrigin);
    await expect(search.results.error).toContainText("HFS_BASE_URL");
    await expect(search.results.error).not.toContainText("opaque");
    await expect.poll(() => paginationAttempts).toBe(1);
    expect(await search.results.visibleState()).toEqual(visiblePage);
    await expect
      .poll(() =>
        search.page.evaluate(
          () =>
            (window as typeof window & { __hfsDataChangedFailures?: number })
              .__hfsDataChangedFailures,
        ),
      )
      .toBe(1);
    expect(initialRequests).toBe(1);

    // The failure event does not recurse. A real data change repeats the last
    // successful relative path, not the failed absolute pagination URL.
    await search.page.evaluate(() => {
      document.dispatchEvent(
        new CustomEvent("hfs:data-changed", { detail: { type: "Patient" } }),
      );
    });
    await expect.poll(() => initialRequests).toBe(2);
    await expect(search.results.error).toBeHidden();
    expect(await search.results.visibleState()).toEqual(visiblePage);

    // Non-2xx and malformed JSON failures are equally non-destructive. The
    // server response body and opaque query token never enter the alert.
    await search.results.next.click();
    await expect
      .poll(() =>
        search.page.evaluate(
          () =>
            (window as typeof window & { __hfsDataChangedFailures?: number })
              .__hfsDataChangedFailures,
        ),
      )
      .toBe(2);
    await expect(search.results.error).not.toContainText("sensitive upstream response");
    expect(await search.results.visibleState()).toEqual(visiblePage);

    await search.results.next.click();
    await expect
      .poll(() =>
        search.page.evaluate(
          () =>
            (window as typeof window & { __hfsDataChangedFailures?: number })
              .__hfsDataChangedFailures,
        ),
      )
      .toBe(3);
    expect(await search.results.visibleState()).toEqual(visiblePage);

    // The fourth click retries the server-provided URL byte for byte. A valid
    // Bundle under a public path prefix replaces the page and clears the alert.
    await search.results.next.click();
    await expect(search.results.rows).toHaveCount(1);
    await expect(search.results.rows.first()).toContainText("patient-page-two");
    await expect(search.results.rows.first().locator("a.result-id")).toHaveAttribute(
      "href",
      `${paginationOrigin}/public/fhir/acme/Patient/patient-page-two`,
    );
    await expect(search.results.prev).toBeVisible();
    await expect(search.results.error).toBeHidden();

    const fetchInputs = await search.page.evaluate(
      () =>
        (window as typeof window & { __hfsFetchInputs?: string[] }).__hfsFetchInputs || [],
    );
    expect(fetchInputs.filter((input) => input === paginationUrlWithTotal)).toHaveLength(4);
  });

  // #1227: a same-origin error *response* (e.g. a search-less backend answering
  // 501) surfaces the server's own OperationOutcome diagnostic, not the generic
  // "check HFS_BASE_URL" hint, which is only right for a failed connection.
  test("a same-origin error response shows its OperationOutcome, not the base-url hint", async ({
    search,
  }) => {
    const initialPath = "/Patient?_count=1&_sort=_id";
    await search.page.route("**/Patient?*", async (route) => {
      await route.fulfill({
        status: 501,
        contentType: "application/fhir+json",
        body: JSON.stringify({
          resourceType: "OperationOutcome",
          issue: [
            {
              severity: "error",
              code: "not-supported",
              diagnostics:
                "Feature 'search' is not implemented on this backend",
            },
          ],
        }),
      });
    });

    await search.gotoBuilder();
    await search.builder.run(initialPath);
    await expect(search.results.error).toBeVisible();
    await expect(search.results.error).toContainText("is not implemented");
    await expect(search.results.error).not.toContainText("HFS_BASE_URL");
  });

  // #1106: row-navigation.js delegates the click from `document`, so a click
  // anywhere in the row opens the resource, same as clicking the id link.
  test("clicking a non-id cell opens the resource in a new tab", async ({
    search,
    context,
    request,
  }) => {
    const id = await createResource(request, "Patient", { name: [{ family: "QueriesRowClick" }] });
    await waitSearchable(request, "Patient", id);

    await search.gotoBuilder();
    await search.builder.run(`Patient?_id=${id}`);
    await search.results.waitShown();

    const cell = search.results.rows.first().locator("td:last-child");
    const [opened] = await Promise.all([context.waitForEvent("page"), cell.click()]);
    await opened.waitForLoadState();
    expect(opened.url()).toMatch(new RegExp(`/Patient/${id}$`));
    await opened.close();
  });

  /// Chained search end to end (#406): the two chain directions meet on the
  /// patient in the middle — Practitioner <- Patient (generalPractitioner)
  /// <- Observation (subject) — and the combined query returns exactly it.
  test("a combined chain and _has query runs and returns the linked patient", async ({
    search,
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

    await search.gotoBuilder();
    await search.builder.run(
      `Patient?_has:Observation:patient:code=chain-94-${tag}&general-practitioner.name=ChainSmith${tag}`,
    );
    await search.results.waitShown();
    await expect(search.results.rows).toHaveCount(1);
    await expect(search.results.rows.first()).toContainText(patient);
  });

  test("adding a condition row hydrates the builder", async ({ search }) => {
    // The builder sections are hidden until there's a base query to parse.
    await search.builder.setUrl("Patient");
    await search.builder.addButton("condition").click();
    await expect(search.builder.conditionRows).toHaveCount(1);
  });

  /* ---- chaining (#394) ------------------------------------------------- */

  test("a chained query hydrates into a forward-chain row and round-trips", async ({
    search,
  }) => {
    await search.builder.setUrl("Patient?general-practitioner.name=Smith");
    await expect(search.builder.chainRows).toHaveCount(1);
    const row = search.builder.chainRows.first();
    await expect(row.locator(".builder-row__chainref")).toHaveValue("general-practitioner");
    await expect(row.locator(".builder-row__cparam")).toHaveValue("name");
    await expect(row.locator(".builder-row__value")).toHaveValue("Smith");

    // Editing the value re-serializes the same chained key.
    await row.locator(".builder-row__value").fill("Jones");
    await expect(search.builder.url).toHaveValue(
      "GET /Patient?general-practitioner.name=Jones",
    );
  });

  test("an explicitly typed chain keeps its :Type qualifier", async ({ search }) => {
    await search.builder.setUrl("Observation?subject:Patient.name:contains=ann");
    const row = search.builder.chainRows.first();
    await expect(row.locator(".builder-row__chainref")).toHaveValue("subject");
    await expect(row.locator(".builder-row__cparam")).toHaveValue("name");
    await expect(row.locator(".builder-row__modifier")).toHaveValue("contains");
    // The registry feeds Patient into the target-type select.
    await expect
      .poll(async () => row.locator(".builder-row__ctype").inputValue())
      .toBe("Patient");

    await row.locator(".builder-row__value").fill("bob");
    await expect(search.builder.url).toHaveValue(
      "GET /Observation?subject:Patient.name:contains=bob",
    );
  });

  test("drilling into a reference param converts the row to a chain", async ({
    search,
  }) => {
    await search.builder.setUrl("Patient?general-practitioner=123");
    const row = search.builder.conditionRows.first();
    // The affordance appears once the registry metadata loads.
    const drill = search.builder.drillButton(row);
    await expect(drill).toBeVisible();
    await drill.click();

    await expect(search.builder.chainRows).toHaveCount(1);
    const chain = search.builder.chainRows.first();
    await chain.locator(".builder-row__cparam").fill("name");
    await chain.locator(".builder-row__value").fill("Smith");
    await expect(search.builder.url).toHaveValue(
      "GET /Patient?general-practitioner.name=Smith",
    );
    // The target-type select offers this param's registry targets.
    await expect
      .poll(async () => chain.locator(".builder-row__ctype option").count())
      .toBeGreaterThan(1);
  });

  test("drilling preserves a literal comma and every real OR alternative", async ({
    search,
  }) => {
    await search.builder.setUrl("Patient?general-practitioner=a%26b%5C%2Cc,d");
    const row = search.builder.conditionRows.first();
    const drill = search.builder.drillButton(row);
    await expect(drill).toBeVisible();
    await drill.click();

    const chain = search.builder.chainRows.first();
    await expect(chain.locator(".builder-row__value")).toHaveCount(2);
    await expect(chain.locator(".builder-row__value").nth(0)).toHaveValue("a&b,c");
    await expect(chain.locator(".builder-row__value").nth(1)).toHaveValue("d");
    await chain.locator(".builder-row__cparam").fill("name");
    await expect(search.builder.url).toHaveValue(
      "GET /Patient?general-practitioner.name=a%26b%5C%2Cc,d",
    );
  });

  test("a _has query hydrates into a reverse-chain row and round-trips", async ({
    search,
  }) => {
    await search.builder.setUrl("Patient?_has:Observation:patient:code=1234-5");
    await expect(search.builder.hasRows).toHaveCount(1);
    const row = search.builder.hasRows.first();
    await expect(row.locator(".builder-row__htype")).toHaveValue("Observation");
    await expect(row.locator(".builder-row__href")).toHaveValue("patient");
    await expect(row.locator(".builder-row__cparam")).toHaveValue("code");
    await expect(row.locator(".builder-row__value")).toHaveValue("1234-5");

    await row.locator(".builder-row__value").fill("8480-6");
    await expect(search.builder.url).toHaveValue(
      "GET /Patient?_has:Observation:patient:code=8480-6",
    );
  });

  test("the links-here button builds a _has filter from scratch", async ({ search }) => {
    await search.builder.setUrl("Patient");
    await search.builder.addButton("has").click();
    const row = search.builder.hasRows.first();
    await row.locator(".builder-row__htype").fill("Observation");
    await row.locator(".builder-row__href").fill("patient");
    await row.locator(".builder-row__cparam").fill("code");
    await row.locator(".builder-row__value").fill("1234-5");
    await expect(search.builder.url).toHaveValue(
      "GET /Patient?_has:Observation:patient:code=1234-5",
    );
  });

  test("a multi-level chain hydrates into hop segments and round-trips", async ({
    search,
  }) => {
    await search.builder.setUrl("Patient?general-practitioner.organization.name=Acme");
    await expect(search.builder.chainRows).toHaveCount(1);
    const row = search.builder.chainRows.first();
    const hops = row.locator(".builder-row__hopseg");
    await expect(hops).toHaveCount(2);
    await expect(hops.nth(0).locator(".builder-row__chainref")).toHaveValue(
      "general-practitioner",
    );
    await expect(hops.nth(1).locator(".builder-row__chainref")).toHaveValue("organization");
    await expect(row.locator(".builder-row__cparam")).toHaveValue("name");

    await row.locator(".builder-row__value").fill("Beta");
    await expect(search.builder.url).toHaveValue(
      "GET /Patient?general-practitioner.organization.name=Beta",
    );
  });

  test("drilling deeper appends a hop when the leaf is a reference", async ({
    search,
  }) => {
    await search.builder.setUrl("Patient?general-practitioner.name=x");
    const row = search.builder.chainRows.first();
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
    await expect(search.builder.url).toHaveValue(
      "GET /Patient?general-practitioner.organization.name=Acme",
    );
  });

  /* ---- OR values (#414) ------------------------------------------------ */

  test("comma OR values hydrate as stacked inputs and round-trip", async ({ search }) => {
    await search.builder.setUrl("Patient?name=Smith,Jones");
    const row = search.builder.conditionRows.first();
    const values = row.locator(".builder-row__value");
    await expect(values).toHaveCount(2);
    await expect(values.nth(0)).toHaveValue("Smith");
    await expect(values.nth(1)).toHaveValue("Jones");

    await values.nth(1).fill("Garcia");
    await expect(search.builder.url).toHaveValue("GET /Patient?name=Smith,Garcia");
  });

  test("escaped commas stay one visual value through narration, editing and Run", async ({
    search,
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

    await search.gotoBuilder();
    await search.builder.setUrl(`Patient?name:exact=${escapedName}`);
    const row = search.builder.conditionRows.first();
    const value = row.locator(".builder-row__value");
    await expect(value).toHaveCount(1);
    await expect(value).toHaveValue(literalName);
    await expect(search.page.locator("#query-plain-text")).toContainText(
      `name is exactly “${literalName}”`,
    );
    await expect(search.page.locator("#query-plain-text")).not.toContainText("” or “");

    const requestSent = search.page.waitForRequest((candidate) => {
      const url = new URL(candidate.url());
      return url.pathname === "/Patient" && url.searchParams.has("name:exact");
    });
    await search.builder.runButton.click();
    const sent = await requestSent;
    expect(new URL(sent.url()).searchParams.get("name:exact")).toBe(
      literalName.replace(",", "\\,"),
    );
    await search.results.waitShown();
    await expect(search.results.rows).toHaveCount(1);
    await expect(search.results.rows.first()).toContainText(literal);

    await value.fill(`Comma,Edited-${tag}`);
    await expect(search.builder.url).toHaveValue(
      `GET /Patient?name:exact=Comma%5C%2CEdited-${tag}`,
    );
  });

  test("literal commas coexist with real OR values in conditions, chains and _has", async ({
    search,
  }) => {
    await search.builder.setUrl("Patient?name=a%5C%2Cb,c");
    let row = search.builder.conditionRows.first();
    await expect(row.locator(".builder-row__value")).toHaveCount(2);
    await expect(row.locator(".builder-row__value").nth(0)).toHaveValue("a,b");
    await expect(row.locator(".builder-row__value").nth(1)).toHaveValue("c");
    await row.locator("[data-remove-or]").nth(1).click();
    await expect(search.builder.url).toHaveValue("GET /Patient?name=a%5C%2Cb");

    await search.builder.setUrl("Patient?general-practitioner.name=a%5C%2Cb,c");
    row = search.builder.chainRows.first();
    await expect(row.locator(".builder-row__value")).toHaveCount(2);
    await expect(row.locator(".builder-row__value").nth(0)).toHaveValue("a,b");

    await search.builder.setUrl("Patient?_has:Observation:patient:code=a%5C%2Cb,c");
    row = search.builder.hasRows.first();
    await expect(row.locator(".builder-row__value")).toHaveCount(2);
    await expect(row.locator(".builder-row__value").nth(0)).toHaveValue("a,b");
    await row.locator(".builder-row__value").nth(0).fill("a&b,c");
    await expect(search.builder.url).toHaveValue(
      "GET /Patient?_has:Observation:patient:code=a%26b%5C%2Cc,c",
    );
  });

  test("malformed FHIR escapes keep the GET unchanged until corrected", async ({ search }) => {
    await search.builder.setUrl("Patient?name=a%5Cx");
    const row = search.builder.conditionRows.first();
    await expect(search.builder.error).toBeVisible();
    await expect(search.builder.error).toContainText("invalid FHIR escape");

    await row.locator(".builder-row__key").fill("family");
    await expect(search.builder.url).toHaveValue("Patient?name=a%5Cx");

    await row.locator(".builder-row__value").fill("a\\x");
    await expect(search.builder.error).toBeHidden();
    await expect(search.builder.url).toHaveValue("GET /Patient?family=a%5C%5Cx");
  });

  test("operator cleanup keeps malformed escapes blocked until correction", async ({
    page,
    search,
  }) => {
    const searches: string[] = [];
    page.on("request", (request) => {
      const url = new URL(request.url());
      if (request.method() === "GET" && url.pathname === "/Patient") {
        searches.push(url.search);
      }
    });

    await search.builder.setUrl("Patient?name:contains=a%5Cx");
    const row = search.builder.conditionRows.first();
    await row.locator(".builder-row__key").fill("gender");
    await expect(row).toHaveAttribute("data-mod-type", "token");
    await expect(row.locator(".builder-row__modifier")).toHaveValue("");
    await expect(search.builder.error).toBeVisible();
    await expect(search.builder.runButton).toBeDisabled();
    await expect(search.builder.copyButton).toBeDisabled();
    await search.builder.url.focus();
    await page.keyboard.press("Enter");
    expect(searches).toEqual([]);

    await row.locator(".builder-row__value").fill("a\\x");
    await expect(search.builder.error).toBeHidden();
    await expect(search.builder.url).toHaveValue("GET /Patient?gender=a%5C%5Cx");
    await expect(search.builder.runButton).toBeEnabled();
    await expect(search.builder.copyButton).toBeEnabled();
    expect(searches).toEqual([]);
  });

  test("the + or button stacks a value; the per-value × removes it", async ({
    search,
  }) => {
    await search.builder.setUrl("Patient?name=Smith");
    const row = search.builder.conditionRows.first();
    await row.locator("[data-add-or]").click();
    await row.locator(".builder-row__value").nth(1).fill("Jones");
    await expect(search.builder.url).toHaveValue("GET /Patient?name=Smith,Jones");

    await row.locator("[data-remove-or]").first().click();
    await expect(row.locator(".builder-row__value")).toHaveCount(1);
    await expect(search.builder.url).toHaveValue("GET /Patient?name=Jones");
  });

  test("unchanged empty alternatives survive an unrelated visual edit", async ({ search }) => {
    await search.builder.setUrl("Patient?name=a,,b");
    const row = search.builder.conditionRows.first();
    await expect(row.locator(".builder-row__value")).toHaveCount(3);
    await row.locator(".builder-row__key").fill("family");
    await expect(search.builder.url).toHaveValue("GET /Patient?family=a,,b");
  });

  test("modifier values that resemble comparators remain literal", async ({ search }) => {
    await search.builder.setUrl("Patient?name:exact=ge1980,le1990");
    const row = search.builder.conditionRows.first();
    const comparators = row.locator(".builder-row__comparator");
    const values = row.locator(".builder-row__value");

    await expect(row.locator(".builder-row__modifier")).toHaveValue("exact");
    await expect(comparators.nth(0)).toHaveValue("");
    await expect(comparators.nth(1)).toHaveValue("");
    await expect(values.nth(0)).toHaveValue("ge1980");
    await expect(values.nth(1)).toHaveValue("le1990");
    await expect(search.page.locator("#query-plain-text")).toContainText(
      "name is exactly “ge1980” or “le1990”",
    );

    await values.nth(0).fill("ge1981");
    await expect(search.builder.url).toHaveValue(
      "GET /Patient?name:exact=ge1981,le1990",
    );

    /* Moving the same values onto a date parameter drops `:exact`, which no
     * ordered type supports (#627); the prefixes they were hiding behind it
     * become real comparators in the same pass. */
    await row.locator(".builder-row__key").fill("birthdate");
    await expect(row).toHaveAttribute("data-mod-type", "date");
    await expect(row.locator(".builder-row__modifier")).toHaveValue("");
    await expect(comparators.nth(0)).toHaveValue("ge");
    await expect(comparators.nth(1)).toHaveValue("le");
    await expect(values.nth(0)).toHaveValue("1981");
    await expect(values.nth(1)).toHaveValue("1990");
    await expect(search.builder.url).toHaveValue(
      "GET /Patient?birthdate=ge1981,le1990",
    );
  });

  for (const activationPath of ["select", "chip"] as const) {
    test(`an unresolved hydrated string stays literal when :exact is activated by ${activationPath}`, async ({
      page,
      search,
    }) => {
      let releaseParams!: () => void;
      const paramsGate = new Promise<void>((resolve) => {
        releaseParams = resolve;
      });
      await page.route("**/ui/resources/params?type=Patient", async (route) => {
        await paramsGate;
        await route.continue();
      });
      const patientRequests: string[] = [];
      page.on("request", (request) => {
        const url = new URL(request.url());
        if (url.pathname === "/Patient") patientRequests.push(url.search);
      });

      await search.builder.setUrl("Patient?name=ge1980");
      const row = search.builder.conditionRows.first();
      if (activationPath === "select") {
        await row.locator(".builder-row__modifier").selectOption("exact");
      } else {
        await row.locator("[data-toggle-mods]").click();
        await row.locator("[data-mod-chip=exact]").click();
      }

      await expect(row.locator(".builder-row__modifier")).toHaveValue("exact");
      await expect(row.locator(".builder-row__comparator")).toHaveValue("ge");
      await expect(row.locator(".builder-row__value")).toHaveValue("1980");
      await expect(search.builder.url).toHaveValue("Patient?name=ge1980");
      await expect(search.builder.runButton).toBeDisabled();
      await search.builder.runButton.evaluate((button: HTMLButtonElement) => button.click());
      expect(patientRequests).toEqual([]);

      releaseParams();
      await expect(row).toHaveAttribute("data-mod-type", "string");
      await expect(row.locator(".builder-row__modifier")).toHaveValue("exact");
      await expect(row.locator(".builder-row__comparator")).toHaveValue("");
      await expect(row.locator(".builder-row__comparator")).toBeHidden();
      await expect(row.locator(".builder-row__value")).toHaveValue("ge1980");
      await expect(search.builder.url).toHaveValue("GET /Patient?name:exact=ge1980");
      await expect(page.locator("#query-plain-text")).toContainText(
        "name is exactly “ge1980”",
      );
      await expect(search.builder.runButton).toBeEnabled();
      expect(patientRequests).toEqual([]);
    });
  }

  for (const activationPath of ["select", "chip"] as const) {
    test(`delayed and resolved date hydration agree when :missing is activated by ${activationPath}`, async ({
      page,
      search,
    }) => {
      let releaseParams!: () => void;
      const paramsGate = new Promise<void>((resolve) => {
        releaseParams = resolve;
      });
      const paramsRoute = "**/ui/resources/params?type=Patient";
      await page.route(paramsRoute, async (route) => {
        await paramsGate;
        await route.continue();
      });
      const patientRequests: string[] = [];
      page.on("request", (request) => {
        const url = new URL(request.url());
        if (url.pathname === "/Patient") patientRequests.push(url.search);
      });

      await search.builder.setUrl("Patient?birthdate=ge1980-01-01");
      let row = search.builder.conditionRows.first();
      if (activationPath === "select") {
        await row.locator(".builder-row__modifier").selectOption("missing");
      } else {
        await row.locator("[data-toggle-mods]").click();
        await row.locator("[data-mod-chip=missing]").click();
      }
      await expect(search.builder.runButton).toBeDisabled();
      await expect(search.builder.url).toHaveValue(
        "Patient?birthdate=ge1980-01-01",
      );
      await search.builder.runButton.evaluate((button: HTMLButtonElement) => button.click());
      expect(patientRequests).toEqual([]);

      releaseParams();
      await expect(row).toHaveAttribute("data-mod-type", "date");
      await expect(search.builder.runButton).toBeEnabled();
      const delayed = {
        value: await row.locator(".builder-row__value").inputValue(),
        comparator: await row.locator(".builder-row__comparator").inputValue(),
        modifier: await row.locator(".builder-row__modifier").inputValue(),
        url: await search.builder.url.inputValue(),
        narration: await page.locator("#query-plain-text").innerText(),
      };

      await page.unroute(paramsRoute);
      await page.reload({ waitUntil: "networkidle" });
      await search.showBuilder();
      await search.builder.setUrl("Patient?birthdate=ge1980-01-01");
      row = search.builder.conditionRows.first();
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
        url: await search.builder.url.inputValue(),
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
      await expect(search.builder.runButton).toBeEnabled();
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
        search,
      }) => {
        let releaseParams!: () => void;
        const paramsGate = new Promise<void>((resolve) => {
          releaseParams = resolve;
        });
        await page.route("**/ui/resources/params?type=Patient", async (route) => {
          await paramsGate;
          await route.continue();
        });
        await context.grantPermissions(["clipboard-read", "clipboard-write"]);
        const patientRequests: string[] = [];
        page.on("request", (request) => {
          const url = new URL(request.url());
          if (url.pathname === "/Patient") patientRequests.push(url.search);
        });

        await search.builder.setUrl(source.query);
        const row = search.builder.conditionRows.first();
        if (activationPath === "select") {
          await row.locator(".builder-row__modifier").selectOption(source.modifier);
        } else {
          await row.locator("[data-toggle-mods]").click();
          await row.locator(`[data-mod-chip=${source.modifier}]`).click();
        }
        await expect(search.builder.runButton).toBeDisabled();

        await row.locator(".builder-row__comparator").selectOption("le");
        await expect(row.locator(".builder-row__modifier")).toHaveValue("");
        await expect(row.locator(".builder-row__comparator")).toHaveValue("le");
        await expect(search.builder.url).toHaveValue(source.query);
        await search.builder.copyButton.click();
        await expect
          .poll(async () => page.evaluate(() => navigator.clipboard.readText()))
          .toBe(source.query);
        await expect(search.builder.runButton).toBeDisabled();
        await search.builder.runButton.evaluate((button: HTMLButtonElement) => button.click());
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
        await expect(search.builder.url).toHaveValue(source.finalUrl);
        await expect(page.locator("#query-plain-text")).toContainText(
          source.narration,
        );
        if (source.comparatorHidden) {
          await expect(row.locator(".builder-row__comparator")).toBeHidden();
        } else {
          await expect(row.locator(".builder-row__comparator")).toBeVisible();
        }
        await expect(search.builder.runButton).toBeEnabled();
        expect(patientRequests).toEqual([]);
      });
    }
  }

  for (const source of pendingComparatorSources) {
    test(`a direct delayed ${source.name} comparator edit waits for classification`, async ({
      context,
      page,
      search,
    }) => {
      let releaseParams!: () => void;
      const paramsGate = new Promise<void>((resolve) => {
        releaseParams = resolve;
      });
      await page.route("**/ui/resources/params?type=Patient", async (route) => {
        await paramsGate;
        await route.continue();
      });
      await context.grantPermissions(["clipboard-read", "clipboard-write"]);
      const patientRequests: string[] = [];
      page.on("request", (request) => {
        const url = new URL(request.url());
        if (url.pathname === "/Patient") patientRequests.push(url.search);
      });

      await search.builder.setUrl(source.query);
      const row = search.builder.conditionRows.first();
      await row.locator(".builder-row__comparator").selectOption("le");

      await expect(search.builder.url).toHaveValue(source.query);
      await search.builder.copyButton.click();
      await expect
        .poll(async () => page.evaluate(() => navigator.clipboard.readText()))
        .toBe(source.query);
      await expect(search.builder.runButton).toBeDisabled();
      await search.builder.runButton.evaluate((button: HTMLButtonElement) => button.click());
      expect(patientRequests).toEqual([]);

      releaseParams();
      await expect(row).toHaveAttribute("data-mod-type", source.resolvedType);
      await expect(row.locator(".builder-row__comparator")).toHaveValue(
        source.finalComparator,
      );
      await expect(row.locator(".builder-row__value")).toHaveValue(
        source.finalValue,
      );
      await expect(search.builder.url).toHaveValue(source.finalUrl);
      await expect(page.locator("#query-plain-text")).toContainText(
        source.narration,
      );
      await expect(search.builder.runButton).toBeEnabled();
      expect(patientRequests).toEqual([]);
    });
  }

  test("a comparator on a newly added OR alternative waits for source classification", async ({
    context,
    page,
    search,
  }) => {
    let releaseParams!: () => void;
    const paramsGate = new Promise<void>((resolve) => {
      releaseParams = resolve;
    });
    await page.route("**/ui/resources/params?type=Patient", async (route) => {
      await paramsGate;
      await route.continue();
    });
    await context.grantPermissions(["clipboard-read", "clipboard-write"]);
    const patientRequests: string[] = [];
    page.on("request", (request) => {
      const url = new URL(request.url());
      if (url.pathname === "/Patient") patientRequests.push(url.search);
    });

    await search.builder.setUrl("Patient?name=alpha");
    const row = search.builder.conditionRows.first();
    await row.locator("[data-add-or]").click();
    const values = row.locator(".builder-row__value");
    const comparators = row.locator(".builder-row__comparator");
    await values.nth(1).fill("1980");
    const stable = "GET /Patient?name=alpha,1980";
    await expect(search.builder.url).toHaveValue(stable);

    await comparators.nth(1).selectOption("le");
    await expect(search.builder.url).toHaveValue(stable);
    await search.builder.copyButton.click();
    await expect
      .poll(async () => page.evaluate(() => navigator.clipboard.readText()))
      .toBe(stable);
    await expect(search.builder.runButton).toBeDisabled();
    await search.builder.runButton.evaluate((button: HTMLButtonElement) => button.click());
    expect(patientRequests).toEqual([]);

    releaseParams();
    await expect(row).toHaveAttribute("data-mod-type", "string");
    await expect(comparators.nth(0)).toHaveValue("");
    await expect(comparators.nth(1)).toHaveValue("");
    await expect(comparators.nth(0)).toBeHidden();
    await expect(comparators.nth(1)).toBeHidden();
    await expect(values.nth(0)).toHaveValue("alpha");
    await expect(values.nth(1)).toHaveValue("le1980");
    await expect(search.builder.url).toHaveValue(
      "GET /Patient?name=alpha,le1980",
    );
    await expect(page.locator("#query-plain-text")).toContainText(
      "name is “alpha” or “le1980”",
    );
    await expect(search.builder.runButton).toBeEnabled();
    expect(patientRequests).toEqual([]);
  });

  test("clearing the last unresolved comparator cancels classification", async ({
    context,
    page,
    search,
  }) => {
    let releaseParams!: () => void;
    const paramsGate = new Promise<void>((resolve) => {
      releaseParams = resolve;
    });
    await page.route("**/ui/resources/params?type=Patient", async (route) => {
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
    await search.builder.setUrl(original);
    const row = search.builder.conditionRows.first();
    const comparator = row.locator(".builder-row__comparator");
    await comparator.selectOption("le");
    await expect(search.builder.url).toHaveValue(original);
    await search.builder.copyButton.click();
    await expect
      .poll(async () => page.evaluate(() => navigator.clipboard.readText()))
      .toBe(original);
    await expect(search.builder.runButton).toBeDisabled();

    await comparator.selectOption("");
    await expect(search.builder.url).toHaveValue("GET /Patient?name=1980");
    await expect(search.builder.runButton).toBeEnabled();
    expect(patientRequests).toEqual([]);

    releaseParams();
    await expect(row).toHaveAttribute("data-mod-type", "string");
    await expect(comparator).toHaveValue("");
    await expect(comparator).toBeHidden();
    await expect(row.locator(".builder-row__value")).toHaveValue("1980");
    await expect(search.builder.url).toHaveValue("GET /Patient?name=1980");
    await expect(page.locator("#query-plain-text")).toContainText(
      "name is “1980”",
    );
    await expect(search.builder.runButton).toBeEnabled();
    expect(patientRequests).toEqual([]);
  });

  test("pending comparator classification survives modifier toggles and a parameter transition", async ({
    page,
    search,
  }) => {
    let releaseParams!: () => void;
    const paramsGate = new Promise<void>((resolve) => {
      releaseParams = resolve;
    });
    await page.route("**/ui/resources/params?type=Patient", async (route) => {
      await paramsGate;
      await route.continue();
    });
    const patientRequests: string[] = [];
    page.on("request", (request) => {
      const url = new URL(request.url());
      if (url.pathname === "/Patient") patientRequests.push(url.search);
    });

    const original = "Patient?name=ge1980";
    await search.builder.setUrl(original);
    const row = search.builder.conditionRows.first();
    const modifier = row.locator(".builder-row__modifier");
    await modifier.selectOption("exact");
    await row.locator(".builder-row__comparator").selectOption("le");
    await modifier.selectOption("exact");
    await modifier.selectOption("");
    await row.locator(".builder-row__key").fill("birthdate");

    await expect(search.builder.url).toHaveValue(original);
    await expect(search.builder.runButton).toBeDisabled();
    expect(patientRequests).toEqual([]);

    releaseParams();
    await expect(row).toHaveAttribute("data-mod-type", "date");
    await expect(modifier).toHaveValue("");
    await expect(row.locator(".builder-row__comparator")).toHaveValue("le");
    await expect(row.locator(".builder-row__value")).toHaveValue("1980");
    await expect(search.builder.url).toHaveValue(
      "GET /Patient?birthdate=le1980",
    );
    await expect(search.builder.runButton).toBeEnabled();
    expect(patientRequests).toEqual([]);
  });

  test("removing a row releases pending comparator classification", async ({
    page,
    search,
  }) => {
    let releaseParams!: () => void;
    const paramsGate = new Promise<void>((resolve) => {
      releaseParams = resolve;
    });
    await page.route("**/ui/resources/params?type=Patient", async (route) => {
      await paramsGate;
      await route.continue();
    });
    const patientRequests: string[] = [];
    page.on("request", (request) => {
      const url = new URL(request.url());
      if (url.pathname === "/Patient") patientRequests.push(url.search);
    });

    await search.builder.setUrl("Patient?name=ge1980");
    const row = search.builder.conditionRows.first();
    await row.locator(".builder-row__modifier").selectOption("exact");
    await row.locator(".builder-row__comparator").selectOption("le");
    await expect(search.builder.runButton).toBeDisabled();

    await row.locator("[data-remove-row]").click();
    await expect(search.builder.conditionRows).toHaveCount(0);
    await expect(search.builder.url).toHaveValue("GET /Patient");
    await expect(search.builder.runButton).toBeEnabled();
    expect(patientRequests).toEqual([]);

    releaseParams();
    await expect(search.builder.conditionRows).toHaveCount(0);
    await expect(search.builder.url).toHaveValue("GET /Patient");
    await expect(search.builder.runButton).toBeEnabled();
    expect(patientRequests).toEqual([]);
  });

  for (const activationPath of ["select", "chip"] as const) {
    test(`value edits stay transactional during pending ${activationPath} modifier activation`, async ({
      context,
      page,
      search,
    }) => {
      let releaseParams!: () => void;
      const paramsGate = new Promise<void>((resolve) => {
        releaseParams = resolve;
      });
      await page.route("**/ui/resources/params?type=Patient", async (route) => {
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
      await search.builder.setUrl(original);
      const row = search.builder.conditionRows.first();
      if (activationPath === "select") {
        await row.locator(".builder-row__modifier").selectOption("exact");
      } else {
        await row.locator("[data-toggle-mods]").click();
        await row.locator("[data-mod-chip=exact]").click();
      }
      await row.locator(".builder-row__value").fill("1981");

      await expect(search.builder.url).toHaveValue(original);
      await search.builder.copyButton.click();
      await expect
        .poll(async () => page.evaluate(() => navigator.clipboard.readText()))
        .toBe(original);
      await expect(search.builder.runButton).toBeDisabled();
      await search.builder.runButton.evaluate((button: HTMLButtonElement) => button.click());
      expect(patientRequests).toEqual([]);

      releaseParams();
      await expect(row).toHaveAttribute("data-mod-type", "string");
      await expect(row.locator(".builder-row__modifier")).toHaveValue("exact");
      await expect(row.locator(".builder-row__comparator")).toHaveValue("");
      await expect(row.locator(".builder-row__comparator")).toBeHidden();
      await expect(row.locator(".builder-row__value")).toHaveValue("ge1981");
      await expect(search.builder.url).toHaveValue("GET /Patient?name:exact=ge1981");
      await expect(page.locator("#query-plain-text")).toContainText(
        "name is exactly “ge1981”",
      );
      await expect(search.builder.runButton).toBeEnabled();
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
    test(`mixed date comparators hydrate independently: ${scenario.name}`, async ({ search }) => {
      await search.builder.setUrl(scenario.query);
      const row = search.builder.conditionRows.first();
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
      await expect(search.builder.url).toHaveValue(`GET /${scenario.query}`);
      await expect(search.page.locator("#query-plain-text")).toContainText(
        scenario.narration,
      );
    });
  }

  test("editing one date comparator leaves its sibling unchanged", async ({ search }) => {
    await search.builder.setUrl("Patient?birthdate=le1979-12-31,ge1980-01-02");
    const row = search.builder.conditionRows.first();
    const comparators = row.locator(".builder-row__comparator");
    const values = row.locator(".builder-row__value");

    await comparators.nth(1).selectOption("gt");
    await expect(comparators.nth(0)).toHaveValue("le");
    await expect(comparators.nth(1)).toHaveValue("gt");
    await expect(search.builder.url).toHaveValue(
      "GET /Patient?birthdate=le1979-12-31,gt1980-01-02",
    );
    await expect(search.page.locator("#query-plain-text")).toContainText(
      "birthdate is on or before “1979-12-31” or birthdate is after “1980-01-02”",
    );

    await values.nth(0).fill("1978-12-31");
    await expect(search.builder.url).toHaveValue(
      "GET /Patient?birthdate=le1978-12-31,gt1980-01-02",
    );
    await expect(comparators.nth(1)).toHaveValue("gt");
  });

  test("adding and removing date alternatives keeps comparators with their values", async ({
    search,
  }) => {
    await search.builder.setUrl("Patient?birthdate=le1979-12-31");
    const row = search.builder.conditionRows.first();

    await row.locator("[data-add-or]").click();
    const alternatives = row.locator(".builder-row__orvalue");
    await expect(alternatives).toHaveCount(2);
    await expect(alternatives.nth(1).locator(".builder-row__comparator")).toHaveValue("");
    await alternatives.nth(1).locator(".builder-row__comparator").selectOption("ge");
    await alternatives.nth(1).locator(".builder-row__value").fill("1980-01-02");
    await expect(search.builder.url).toHaveValue(
      "GET /Patient?birthdate=le1979-12-31,ge1980-01-02",
    );

    await alternatives.nth(0).locator("[data-remove-or]").click();
    await expect(alternatives).toHaveCount(1);
    await expect(alternatives.first().locator(".builder-row__comparator")).toHaveValue("ge");
    await expect(alternatives.first().locator(".builder-row__value")).toHaveValue(
      "1980-01-02",
    );
    await expect(search.builder.url).toHaveValue("GET /Patient?birthdate=ge1980-01-02");
  });

  test("changing a date parameter clears every incompatible OR comparator before Run", async ({
    page,
    search,
  }) => {
    const patientRequests: string[] = [];
    page.on("request", (request) => {
      const url = new URL(request.url());
      if (url.pathname === "/Patient") patientRequests.push(url.search);
    });

    await search.builder.setUrl(
      "Patient?birthdate=ge1980-01-02,le1990-12-31",
    );
    const row = search.builder.conditionRows.first();
    await expect(row).toHaveAttribute("data-mod-type", "date");

    await row.locator(".builder-row__key").fill("name");
    await expect(row).toHaveAttribute("data-mod-type", "string");
    await expect(row.locator(".builder-row__comparator").nth(0)).toHaveValue("");
    await expect(row.locator(".builder-row__comparator").nth(1)).toHaveValue("");
    await expect(row.locator(".builder-row__comparator").nth(0)).toBeHidden();
    await expect(row.locator(".builder-row__comparator").nth(1)).toBeHidden();
    await expect(row.locator(".builder-row__value").nth(0)).toHaveValue("1980-01-02");
    await expect(row.locator(".builder-row__value").nth(1)).toHaveValue("1990-12-31");
    await expect(search.builder.url).toHaveValue(
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
    await search.builder.runButton.click();
    const sent = new URL((await sentRequest).url());
    expect(sent.searchParams.get("name")).toBe("1980-01-02,1990-12-31");
    expect(patientRequests).toHaveLength(1);
  });

  test("known string prefix-lookalikes hydrate literally while unknown parameters stay permissive", async ({
    page,
    search,
  }) => {
    await search.builder.setUrl("Patient?name=ge1980,le1990");
    let row = search.builder.conditionRows.first();
    await expect(row).toHaveAttribute("data-mod-type", "string");
    await expect(row.locator(".builder-row__comparator").nth(0)).toHaveValue("");
    await expect(row.locator(".builder-row__comparator").nth(1)).toHaveValue("");
    await expect(row.locator(".builder-row__comparator").nth(0)).toBeHidden();
    await expect(row.locator(".builder-row__value").nth(0)).toHaveValue("ge1980");
    await expect(row.locator(".builder-row__value").nth(1)).toHaveValue("le1990");
    await expect(search.builder.url).toHaveValue("Patient?name=ge1980,le1990");
    await expect(page.locator("#query-plain-text")).toContainText(
      "name is “ge1980” or “le1990”",
    );

    await search.builder.setUrl("Patient?unregistered=ge1980");
    row = search.builder.conditionRows.first();
    await expect(row).toHaveAttribute("data-mod-type", "");
    await expect(row.locator(".builder-row__comparator")).toHaveValue("ge");
    await expect(row.locator(".builder-row__comparator")).toBeVisible();
    await expect(row.locator(".builder-row__value")).toHaveValue("1980");
    await expect(search.builder.url).toHaveValue("Patient?unregistered=ge1980");

    await search.builder.setUrl("Patient?link:Patient.name=ge1980");
    row = search.builder.chainRows.first();
    await expect(row).toHaveAttribute("data-mod-type", "string");
    await expect(row.locator(".builder-row__comparator")).toBeHidden();
    await expect(row.locator(".builder-row__value")).toHaveValue("ge1980");
    await expect(page.locator("#query-plain-text")).toContainText("name is “ge1980”");
    await expect(page.locator("#query-plain-text")).not.toContainText("on or after");

    await search.builder.setUrl("Patient?_has:Observation:patient:code=ge1980");
    row = search.builder.hasRows.first();
    await expect(row).toHaveAttribute("data-mod-type", "token");
    await expect(row.locator(".builder-row__comparator")).toBeHidden();
    await expect(row.locator(".builder-row__value")).toHaveValue("ge1980");
    await expect(page.locator("#query-plain-text")).toContainText("code is “ge1980”");
    await expect(page.locator("#query-plain-text")).not.toContainText("on or after");
  });

  test("forward and reverse chain leaf changes clear incompatible comparators", async ({
    search,
  }) => {
    await search.builder.setUrl("Patient?link:Patient.birthdate=ge1980-01-02");
    let row = search.builder.chainRows.first();
    await expect(row).toHaveAttribute("data-mod-type", "date");
    await row.locator(".builder-row__cparam").fill("name");
    await expect(row).toHaveAttribute("data-mod-type", "string");
    await expect(row.locator(".builder-row__comparator")).toHaveValue("");
    await expect(row.locator(".builder-row__comparator")).toBeHidden();
    await expect(search.builder.url).toHaveValue(
      "GET /Patient?link:Patient.name=1980-01-02",
    );

    await search.builder.setUrl(
      "Patient?_has:Observation:patient:date=le2020-01-01",
    );
    row = search.builder.hasRows.first();
    await expect(row).toHaveAttribute("data-mod-type", "date");
    await row.locator(".builder-row__cparam").fill("code");
    await expect(row).toHaveAttribute("data-mod-type", "token");
    await expect(row.locator(".builder-row__comparator")).toHaveValue("");
    await expect(row.locator(".builder-row__comparator")).toBeHidden();
    await expect(search.builder.url).toHaveValue(
      "GET /Patient?_has:Observation:patient:code=2020-01-01",
    );
  });

  test("Run stays blocked while a parameter transition awaits metadata", async ({
    page,
    search,
  }) => {
    let releaseParams!: () => void;
    let delayed = true;
    const paramsGate = new Promise<void>((resolve) => {
      releaseParams = resolve;
    });
    await page.route("**/ui/resources/params?type=Patient", async (route) => {
      if (delayed) await paramsGate;
      await route.continue();
    });
    const patientRequests: string[] = [];
    page.on("request", (request) => {
      const url = new URL(request.url());
      if (url.pathname === "/Patient") patientRequests.push(url.search);
    });

    await search.builder.setUrl("Patient?birthdate=ge1980-01-02");
    const row = search.builder.conditionRows.first();
    await row.locator(".builder-row__key").fill("name");
    await expect(search.builder.runButton).toBeDisabled();
    await search.builder.runButton.evaluate((button: HTMLButtonElement) => button.click());
    expect(patientRequests).toEqual([]);

    delayed = false;
    releaseParams();
    await expect(row).toHaveAttribute("data-mod-type", "string");
    await expect(search.builder.url).toHaveValue("GET /Patient?name=1980-01-02");
    await expect(search.builder.runButton).toBeEnabled();
    expect(patientRequests).toEqual([]);
  });

  test("a delayed known-string hydration stays literal across a direct parameter edit", async ({
    page,
    search,
  }) => {
    let releaseParams!: () => void;
    const paramsGate = new Promise<void>((resolve) => {
      releaseParams = resolve;
    });
    await page.route("**/ui/resources/params?type=Patient", async (route) => {
      await paramsGate;
      await route.continue();
    });

    await search.builder.setUrl("Patient?name=ge1980");
    const row = search.builder.conditionRows.first();
    await row.locator(".builder-row__key").fill("family");
    await expect(search.builder.runButton).toBeDisabled();

    releaseParams();
    await expect(row).toHaveAttribute("data-mod-type", "string");
    await expect(row.locator(".builder-row__comparator")).toHaveValue("");
    await expect(row.locator(".builder-row__comparator")).toBeHidden();
    await expect(row.locator(".builder-row__value")).toHaveValue("ge1980");
    await expect(search.builder.url).toHaveValue("GET /Patient?family=ge1980");
    await expect(page.locator("#query-plain-text")).toContainText("family is “ge1980”");
    await expect(search.builder.runButton).toBeEnabled();
  });

  test("delayed known-string hydration stays literal across chain leaf edits", async ({
    page,
    search,
  }) => {
    let releasePatient!: () => void;
    const patientGate = new Promise<void>((resolve) => {
      releasePatient = resolve;
    });
    await page.route("**/ui/resources/params?type=Patient", async (route) => {
      await patientGate;
      await route.continue();
    });

    await search.builder.setUrl("Patient?link:Patient.name=ge1980");
    let row = search.builder.chainRows.first();
    await row.locator(".builder-row__cparam").fill("family");
    await expect(search.builder.runButton).toBeDisabled();
    releasePatient();
    await expect(row).toHaveAttribute("data-mod-type", "string");
    await expect(row.locator(".builder-row__comparator")).toBeHidden();
    await expect(row.locator(".builder-row__value")).toHaveValue("ge1980");
    await expect(search.builder.url).toHaveValue(
      "GET /Patient?link:Patient.family=ge1980",
    );

    await page.reload({ waitUntil: "networkidle" });
    await search.showBuilder();
    let releaseObservation!: () => void;
    const observationGate = new Promise<void>((resolve) => {
      releaseObservation = resolve;
    });
    await page.route("**/ui/resources/params?type=Observation", async (route) => {
      await observationGate;
      await route.continue();
    });

    await search.builder.setUrl("Patient?_has:Observation:patient:code=ge1980");
    row = search.builder.hasRows.first();
    await row.locator(".builder-row__cparam").fill("category");
    await expect(search.builder.runButton).toBeDisabled();
    releaseObservation();
    await expect(row).toHaveAttribute("data-mod-type", "token");
    await expect(row.locator(".builder-row__comparator")).toBeHidden();
    await expect(row.locator(".builder-row__value")).toHaveValue("ge1980");
    await expect(search.builder.url).toHaveValue(
      "GET /Patient?_has:Observation:patient:category=ge1980",
    );
  });

  test("number and quantity parameters retain ordered comparator controls", async ({
    search,
  }) => {
    await search.builder.setUrl("RiskAssessment?probability=ge0.5");
    let row = search.builder.conditionRows.first();
    await expect(row).toHaveAttribute("data-mod-type", "number");
    await expect(row.locator(".builder-row__comparator")).toHaveValue("ge");
    await expect(row.locator(".builder-row__comparator")).toBeVisible();
    await expect(row.locator(".builder-row__value")).toHaveValue("0.5");
    await row.locator(".builder-row__comparator").selectOption("gt");
    await expect(search.builder.url).toHaveValue(
      "GET /RiskAssessment?probability=gt0.5",
    );

    await search.builder.setUrl("Observation?value-quantity=le5");
    row = search.builder.conditionRows.first();
    await expect(row).toHaveAttribute("data-mod-type", "quantity");
    await expect(row.locator(".builder-row__comparator")).toHaveValue("le");
    await expect(row.locator(".builder-row__comparator")).toBeVisible();
    await expect(row.locator(".builder-row__value")).toHaveValue("5");
    await row.locator(".builder-row__comparator").selectOption("ge");
    await expect(search.builder.url).toHaveValue(
      "GET /Observation?value-quantity=ge5",
    );
  });

  test("date comparators stay available in forward and reverse chains", async ({
    search,
  }) => {
    await search.builder.setUrl("Patient?link:Patient.birthdate=1980-01-02");
    const chain = search.builder.chainRows.first();
    const chainComparator = chain.locator(".builder-row__comparator");
    await expect(chain).toHaveAttribute("data-mod-type", "date");
    await expect(chainComparator).toBeVisible();
    await chainComparator.selectOption("ge");
    await expect(search.builder.url).toHaveValue(
      "GET /Patient?link:Patient.birthdate=ge1980-01-02",
    );

    await search.builder.setUrl("Patient?_has:Observation:patient:date=2020-01-01");
    const reverseChain = search.builder.hasRows.first();
    const reverseComparator = reverseChain.locator(".builder-row__comparator");
    await expect(reverseChain).toHaveAttribute("data-mod-type", "date");
    await expect(reverseComparator).toBeVisible();
    await reverseComparator.selectOption("le");
    await expect(search.builder.url).toHaveValue(
      "GET /Patient?_has:Observation:patient:date=le2020-01-01",
    );
  });

  /* ---- modifier panel (#415) ------------------------------------------ */

  test("the modifier select gates to the parameter type", async ({ search }) => {
    await search.builder.setUrl("Patient?name=x");
    const row = search.builder.conditionRows.first();
    // string param: :contains offered, date prefixes are not
    await expect
      .poll(async () => row.locator(".builder-row__modifier option").allTextContents())
      .toContain(":contains");
    const opts = await row.locator(".builder-row__modifier option").allTextContents();
    expect(opts).not.toContain("ge");
    expect(opts).not.toContain(":in");
  });

  test("changing a parameter removes incompatible modifiers before the query can run", async ({
    page,
    search,
  }) => {
    const searches: string[] = [];
    page.on("request", (request) => {
      const url = new URL(request.url());
      if (request.method() === "GET" && url.pathname === "/Patient") {
        searches.push(url.search);
      }
    });

    await search.builder.setUrl("Patient?name:contains=OrAlpha556");
    const row = search.builder.conditionRows.first();
    await expect(row).toHaveAttribute("data-mod-type", "string");
    await row.locator(".builder-row__key").fill("gender");

    await expect(row).toHaveAttribute("data-mod-type", "token");
    await expect(row.locator(".builder-row__modifier")).toHaveValue("");
    await expect(search.builder.url).toHaveValue("GET /Patient?gender=OrAlpha556");
    await expect(page.locator("#query-plain-text")).toContainText(
      "gender is “OrAlpha556”",
    );
    expect(searches).toEqual([]);

    const sent = page.waitForRequest((request) => {
      const url = new URL(request.url());
      return request.method() === "GET" && url.pathname === "/Patient";
    });
    await search.builder.runButton.click();
    // #1003: withTotal appends `_total=accurate` on the wire (the URL box
    // above stays the user's literal text).
    expect(new URL((await sent).url()).search).toBe("?gender=OrAlpha556&_total=accurate");
  });

  test("known parameter types expose the complete modifier matrix", async ({ page, search }) => {
    const scenarios = [
      {
        key: "string-param",
        type: "string",
        modifiers: ["exact", "contains", "text", "missing"],
      },
      {
        key: "token-param",
        type: "token",
        modifiers: ["text", "not", "above", "below", "in", "not-in", "of-type", "missing"],
      },
      {
        key: "reference-param",
        type: "reference",
        modifiers: ["contains", "text", "above", "below", "identifier", "missing"],
      },
      {
        key: "uri-param",
        type: "uri",
        modifiers: ["contains", "above", "below", "missing"],
      },
      { key: "date-param", type: "date", modifiers: ["missing"] },
      { key: "number-param", type: "number", modifiers: ["missing"] },
      { key: "quantity-param", type: "quantity", modifiers: ["missing"] },
      { key: "composite-param", type: "composite", modifiers: [] },
      { key: "special-param", type: "special", modifiers: [] },
    ];
    await page.route("**/ui/resources/params?type=Matrix", async (route) => {
      await route.fulfill({
        contentType: "text/html",
        body: catalogHtml(
          scenarios.map(({ key, type }) => ({ code: key, type })),
        ),
      });
    });

    for (const scenario of scenarios) {
      await search.builder.setUrl(`Matrix?${scenario.key}=x`);
      const row = search.builder.conditionRows.first();
      await expect(row).toHaveAttribute("data-mod-type", scenario.type);
      const values = await row.locator(".builder-row__modifier option").evaluateAll((options) =>
        options.slice(1).map((option) => (option as HTMLOptionElement).value),
      );
      expect(values).toEqual(scenario.modifiers);
    }
  });

  test("compatibility-only and ambiguous modifiers are preserved only where valid", async ({
    page,
    search,
  }) => {
    const catalogs: Record<
      string,
      Array<{ code: string; type: string; targets?: string[] }>
    > = {
      Matrix: [
        { code: "string-param", type: "string" },
        { code: "token-param", type: "token" },
        { code: "reference-param", type: "reference" },
        { code: "subject", type: "reference", targets: ["Left", "Right"] },
      ],
      Left: [{ code: "leaf", type: "string" }],
      Right: [{ code: "leaf", type: "token" }],
    };
    await page.route("**/ui/resources/params?type=*", async (route) => {
      const type = new URL(route.request().url()).searchParams.get("type") || "";
      await route.fulfill({
        contentType: "text/html",
        body: catalogHtml(catalogs[type] || []),
      });
    });

    const transitions = [
      { modifier: "text-advanced", from: "token-param", to: "string-param" },
      { modifier: "code-text", from: "reference-param", to: "string-param" },
      { modifier: "Patient", from: "reference-param", to: "token-param" },
    ];
    for (const transition of transitions) {
      await search.builder.setUrl(
        `Matrix?${transition.from}:${transition.modifier}=x`,
      );
      const row = search.builder.conditionRows.first();
      await expect(row.locator(".builder-row__modifier")).toHaveValue(
        transition.modifier,
      );
      await row.locator(".builder-row__key").fill(transition.to);
      await expect(row.locator(".builder-row__modifier")).toHaveValue("");
      await expect(search.builder.url).toHaveValue(
        `GET /Matrix?${transition.to}=x`,
      );
    }

    await search.builder.setUrl("Matrix?subject.leaf:opaque=x");
    const chain = search.builder.chainRows.first();
    await expect(chain).toHaveAttribute("data-compat-state", "unknown");
    await expect(chain.locator(".builder-row__modifier")).toHaveValue("opaque");
    await expect(search.builder.url).toHaveValue("Matrix?subject.leaf:opaque=x");
  });

  test("removing a modifier rehydrates every ordered prefix exactly once", async ({
    page,
    search,
  }) => {
    await page.route("**/ui/resources/params?type=Matrix", async (route) => {
      await route.fulfill({
        contentType: "text/html",
        body: catalogHtml([
          { code: "text-param", type: "string" },
          { code: "ordered-param", type: "date" },
        ]),
      });
    });

    for (const prefix of ["eq", "ne", "gt", "ge", "lt", "le", "sa", "eb", "ap"]) {
      await search.builder.setUrl(
        `Matrix?text-param:contains=${prefix}1980`,
      );
      const row = search.builder.conditionRows.first();
      await row.locator(".builder-row__key").fill("ordered-param");
      await expect(row.locator(".builder-row__modifier")).toHaveValue("");
      await expect(row.locator(".builder-row__comparator")).toHaveValue(prefix);
      await expect(row.locator(".builder-row__value")).toHaveValue("1980");
      await expect(search.builder.url).toHaveValue(
        `GET /Matrix?ordered-param=${prefix}1980`,
      );
      if (prefix === "eq") {
        await row.locator(".builder-row__comparator").selectOption("le");
        await expect(search.builder.url).toHaveValue(
          "GET /Matrix?ordered-param=le1980",
        );
      }
    }
  });

  test("compatible modifiers survive while incompatible value prefixes are removed", async ({
    search,
  }) => {
    await search.builder.setUrl("Patient?name:missing=true");
    let row = search.builder.conditionRows.first();
    await expect(row).toHaveAttribute("data-mod-type", "string");
    await row.locator(".builder-row__key").fill("gender");
    await expect(row.locator(".builder-row__modifier")).toHaveValue("missing");
    await expect(search.builder.url).toHaveValue("GET /Patient?gender:missing=true");

    await search.builder.setUrl("Patient?birthdate=ge1980-01-01");
    row = search.builder.conditionRows.first();
    await expect(row.locator(".builder-row__comparator")).toHaveValue("ge");
    await row.locator(".builder-row__key").fill("gender");
    await expect(row.locator(".builder-row__comparator")).toHaveValue("");
    await expect(row.locator(".builder-row__comparator")).toBeHidden();
    await expect(search.builder.url).toHaveValue("GET /Patient?gender=1980-01-01");
  });

  test("forward and reverse chain leaves reconcile their operators", async ({ search }) => {
    await search.builder.setUrl("Observation?subject:Patient.name:contains=ge1980");
    let row = search.builder.chainRows.first();
    await expect(row).toHaveAttribute("data-mod-type", "string");
    await row.locator(".builder-row__cparam").fill("birthdate");
    await expect(row).toHaveAttribute("data-mod-type", "date");
    await expect(row.locator(".builder-row__modifier")).toHaveValue("");
    await expect(row.locator(".builder-row__comparator")).toHaveValue("ge");
    await expect(row.locator(".builder-row__value")).toHaveValue("1980");
    await expect(search.builder.url).toHaveValue(
      "GET /Observation?subject:Patient.birthdate=ge1980",
    );

    await search.builder.setUrl("Patient?_has:Observation:patient:code:not=ge1980");
    row = search.builder.hasRows.first();
    await expect(row).toHaveAttribute("data-mod-type", "token");
    await row.locator(".builder-row__cparam").fill("date");
    await expect(row).toHaveAttribute("data-mod-type", "date");
    await expect(row.locator(".builder-row__modifier")).toHaveValue("");
    await expect(row.locator(".builder-row__comparator")).toHaveValue("ge");
    await expect(row.locator(".builder-row__value")).toHaveValue("1980");
    await expect(search.builder.url).toHaveValue(
      "GET /Patient?_has:Observation:patient:date=ge1980",
    );
  });

  test("pending catalog metadata blocks every builder consumer and coalesces one URL update", async ({
    page,
    search,
  }) => {
    let releaseCatalog: () => void = () => {};
    const catalogReleased = new Promise<void>((resolve) => {
      releaseCatalog = resolve;
    });
    await page.route("**/ui/resources/params?type=Patient", async (route) => {
      await catalogReleased;
      await route.continue();
    });
    const searches: string[] = [];
    page.on("request", (request) => {
      const url = new URL(request.url());
      if (request.method() === "GET" && url.pathname === "/Patient") {
        searches.push(url.search);
      }
    });

    await search.builder.setUrl("Patient?name:contains=OrAlpha556");
    const row = search.builder.conditionRows.first();
    await row.locator(".builder-row__key").fill("gender");
    await expect(row).toHaveAttribute("data-compat-state", "pending");
    await expect(search.builder.runButton).toBeDisabled();
    await expect(search.builder.copyButton).toBeDisabled();
    await search.builder.url.focus();
    await page.keyboard.press("Enter");
    expect(searches).toEqual([]);

    releaseCatalog();
    await expect(row).toHaveAttribute("data-compat-state", "known");
    await expect(search.builder.url).toHaveValue("GET /Patient?gender=OrAlpha556");
    await expect(search.builder.runButton).toBeEnabled();
    expect(searches).toEqual([]);

    const sent = page.waitForRequest((request) => {
      const url = new URL(request.url());
      return request.method() === "GET" && url.pathname === "/Patient";
    });
    await search.builder.runButton.click();
    // #1003: withTotal appends `_total=accurate` on the wire (the URL box
    // above stays the user's literal text).
    expect(new URL((await sent).url()).search).toBe("?gender=OrAlpha556&_total=accurate");
  });

  test("removing the last pending row immediately releases builder consumers", async ({
    page,
    search,
  }) => {
    let releaseCatalog: () => void = () => {};
    const catalogReleased = new Promise<void>((resolve) => {
      releaseCatalog = resolve;
    });
    await page.route("**/ui/resources/params?type=Patient", async (route) => {
      await catalogReleased;
      await route.continue();
    });

    await search.builder.setUrl("Patient?name=x");
    const row = search.builder.conditionRows.first();
    await expect(row).toHaveAttribute("data-compat-state", "pending");
    await row.locator("[data-remove-row]").click();
    await expect(search.builder.conditionRows).toHaveCount(0);
    await expect(search.builder.url).toHaveValue("GET /Patient");
    await expect(search.builder.runButton).toBeEnabled();
    await expect(search.builder.copyButton).toBeEnabled();

    releaseCatalog();
    await expect(search.builder.runButton).toBeEnabled();
    await expect(search.builder.url).toHaveValue("GET /Patient");
  });

  test("unknown parameters and catalog failures remain permissive", async ({ page, search }) => {
    await page.route("**/ui/resources/params?type=Patient", async (route) => {
      await route.fulfill({ status: 503, body: "catalog unavailable" });
    });

    await search.builder.setUrl("Patient?future-param:opaque=x");
    const row = search.builder.conditionRows.first();
    await expect(row).toHaveAttribute("data-compat-state", "unknown");
    await expect(row.locator(".builder-row__modifier")).toHaveValue("opaque");
    await expect(search.builder.url).toHaveValue("Patient?future-param:opaque=x");
    await expect(search.builder.runButton).toBeEnabled();
  });

  test("no-op catalog reconciliation preserves the exact loaded URL", async ({ search }) => {
    const exact = "GET /Patient?name:exact=Copy%5C%2C627";
    await search.builder.setUrl(exact);
    const row = search.builder.conditionRows.first();

    await expect(row).toHaveAttribute("data-compat-state", "known");
    await expect(row.locator(".builder-row__modifier")).toHaveValue("exact");
    await expect(search.builder.runButton).toBeEnabled();
    await expect(search.builder.url).toHaveValue(exact);
  });

  test("editing a pending deep link cancels its deferred auto-run", async ({ page, search }) => {
    let releaseCatalog: () => void = () => {};
    const catalogReleased = new Promise<void>((resolve) => {
      releaseCatalog = resolve;
    });
    await page.route("**/ui/resources/params?type=Patient", async (route) => {
      await catalogReleased;
      await route.continue();
    });
    const searches: string[] = [];
    page.on("request", (request) => {
      const url = new URL(request.url());
      if (request.method() === "GET" && url.pathname === "/Patient") {
        searches.push(url.search);
      }
    });

    const deepLink = encodeURIComponent("/Patient?name:contains=before");
    await page.goto(`/ui/search?url=${deepLink}`);
    await search.showBuilder();
    const row = search.builder.conditionRows.first();
    await expect(row).toHaveAttribute("data-compat-state", "pending");
    await row.locator(".builder-row__value").fill("after");
    releaseCatalog();

    await expect(row).toHaveAttribute("data-compat-state", "known");
    await expect(search.builder.url).toHaveValue("GET /Patient?name:contains=after");
    await expect(search.builder.runButton).toBeEnabled();
    expect(searches).toEqual([]);
  });

  test("the MODIFY panel explains its chips", async ({ search }) => {
    await search.builder.setUrl("Patient?name=ann");
    const row = search.builder.conditionRows.first();
    await expect(row).toHaveAttribute("data-mod-type", "string");
    await row.locator("[data-toggle-mods]").click();
    const panel = row.locator(".builder-row__modpanel");
    await expect(panel).toBeVisible();
    const chip = panel.locator("[data-mod-chip=contains]");
    await expect(chip).toContainText(":contains");
    await expect(chip).toContainText("anywhere");
  });

  test("choosing a comparator clears the active modifier chip", async ({ search }) => {
    await search.builder.setUrl("Patient?birthdate=1980-01-02");
    const row = search.builder.conditionRows.first();
    await row.locator("[data-toggle-mods]").click();
    const missingChip = row.locator("[data-mod-chip=missing]");

    await missingChip.click();
    await expect(missingChip).toHaveAttribute("aria-pressed", "true");
    await row.locator(".builder-row__comparator").selectOption("ge");

    await expect(row.locator(".builder-row__modifier")).toHaveValue("");
    await expect(missingChip).toHaveAttribute("aria-pressed", "false");
    await expect(search.builder.url).toHaveValue("GET /Patient?birthdate=ge1980-01-02");
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
      search,
    }) => {
      await page.setViewportSize({ width: scenario.width, height: 900 });
      await page.goto(`/ui/search?lang=${scenario.lang}`, { waitUntil: "networkidle" });
      await search.showBuilder();
      await expect(page.locator("html")).toHaveAttribute("lang", scenario.lang);
      await search.builder.setUrl(scenario.query);

      const row = search.builder.conditionRows.first();
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
        await expect(search.builder.url).toHaveValue(scenario.interaction.url);
      }

      await toggle.click();
      await expect(panel).toBeHidden();
      expect(await primaryBounds()).toEqual(before);
    });
  }

  /* ---- results sort + typed columns (#416) ---------------------------- */

  test("typed default columns render and the sort control re-runs the query", async ({
    search,
    request,
  }) => {
    await createResource(request, "Patient", {
      name: [{ family: "Sortable" }],
      gender: "female",
      birthDate: "1980-01-01",
    });
    await search.gotoBuilder();
    await search.builder.run("Patient?name=Sortable");
    await search.results.waitShown();

    // Without _elements, columns come from the attributes the server returned (#1105).
    const headers = search.page.locator("#query-results-head th");
    await expect(headers).toContainText(["id", "name", "gender", "birthDate"]);

    const sort = search.page.locator("#query-results-sort");
    await sort.selectOption("-_lastUpdated");
    // Picking a sort rewrites the visible query and re-runs (#958).
    await expect(search.builder.url).toHaveValue(
      "GET /Patient?name=Sortable&_sort=-_lastUpdated",
    );
  });

  /* ---- related data (#396) -------------------------------------------- */

  test("_include and _revinclude hydrate as structured related-data rows", async ({
    search,
  }) => {
    await search.builder.setUrl(
      "Patient?_include=Patient:general-practitioner&_revinclude:iterate=Observation:patient",
    );
    const rows = search.page.locator("#builder-includes .builder-row--include");
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
    await expect(search.builder.url).toHaveValue(
      "GET /Patient?_include=Patient:general-practitioner&_revinclude=Observation:patient",
    );
  });

  test("the related-data buttons build includes from scratch", async ({ search }) => {
    await search.builder.setUrl("Patient");
    await search.builder.addButton("include-rev").click();
    const row = search.page.locator("#builder-includes .builder-row--include").first();
    await row.locator(".builder-row__itype").fill("Observation");
    await row.locator(".builder-row__iparam").fill("subject");
    await expect(search.builder.url).toHaveValue(
      "GET /Patient?_revinclude=Observation:subject",
    );

    await search.builder.addButton("include-fwd").click();
    const inc = search.page.locator("#builder-includes .builder-row--include").nth(1);
    // The source type defaults to the base type.
    await expect(inc.locator(".builder-row__itype")).toHaveValue("Patient");
    await inc.locator(".builder-row__iparam").fill("general-practitioner");
    await expect(search.builder.url).toHaveValue(
      "GET /Patient?_revinclude=Observation:subject&_include=Patient:general-practitioner",
    );
  });

  /* ---- in plain English (#395) ---------------------------------------- */

  test("the plain-English line narrates conditions, chains, _has and includes", async ({
    search,
  }) => {
    await search.builder.setUrl(
      "Patient?name:contains=Smith,Jones&birthdate=ge1980-01-01" +
        "&general-practitioner.name=Ann&_has:Observation:patient:code=1234-5" +
        "&_include:iterate=Patient:general-practitioner&_count=20",
    );
    const text = search.page.locator("#query-plain-text");
    await expect(search.page.locator("#query-plain")).toBeVisible();
    await expect(text).toContainText("Find Patient records");
    await expect(text).toContainText("name contains “Smith” or “Jones”");
    await expect(text).toContainText("birthdate is on or after “1980-01-01”");
    await expect(text).toContainText("general-practitioner’s name is “Ann”");
    await expect(text).toContainText("related Observation whose code is “1234-5”");
    await expect(text).toContainText("Also returning the general-practitioner of each Patient (repeatedly)");
    await expect(text).toContainText("Showing 20 per page");

    // The narration follows edits made through the rows.
    const row = search.builder.conditionRows.first();
    await row.locator(".builder-row__value").first().fill("Lopez");
    await expect(text).toContainText("name contains “Lopez” or “Jones”");
  });

  test("picking a type swaps in that type's parameter datalist", async ({ search }) => {
    await search.railItem("Patient").click();
    // /ui/resources/params fills #param-options for the picked type.
    await expect.poll(async () => search.builder.paramOptions.count()).toBeGreaterThan(0);
  });

  // Picking a type updates the URL (and back navigates) without a full
  // reload — the click handler is an enhancement over the rail's real
  // <a href> (#541).
  test("picking a rail type updates the URL and back navigates", async ({ search, page }) => {
    await search.railItem("Observation").click();
    await expect(page).toHaveURL(/\/ui\/search\?type=Observation/);
    await expect(search.railItem("Observation")).toHaveAttribute("aria-current", "true");

    await page.goBack();
    await expect(page).toHaveURL(/\/ui\/search$/);
  });

  test("a run is recorded under the Recent disclosure", async ({ search, request }) => {
    const settings = await request.get("/_user/settings");
    test.skip(settings.status() === 501, "no per-user settings store on this backend");
    expect(settings.ok()).toBe(true);
    await createResource(request, "Patient", { name: [{ family: "Recent" }] });
    await search.gotoBuilder();
    await search.builder.run("Patient?name=Recent");
    await search.results.waitShown();

    await search.builder.recentToggle.click();
    await expect(search.builder.recentPanel).toContainText(/Patient/);
  });

  test("URL encoding: visual ampersands survive Run, Copy, Saved and Recent", async ({
    context,
    page,
    search,
    request,
  }) => {
    const tag = Date.now().toString(36);
    const literal = "A&A HEALTHCARE LLC";
    const savedName = `Ampersand query ${tag}`;
    const expected = "GET /Location?name=A%26A%20HEALTHCARE%20LLC";
    const ids: string[] = [];
    let savedFixture: Awaited<ReturnType<typeof seedSavedQuery>> = null;
    await context.grantPermissions(["clipboard-read", "clipboard-write"]);
    try {
      for (let i = 0; i < 2; i++) {
        ids.push(await createResource(request, "Location", { name: literal }));
      }
      const decoy = await createResource(request, "Location", { name: "A ONLY DECOY" });
      ids.push(decoy);
      for (const id of ids) await waitSearchable(request, "Location", id);

      await search.builder.setUrl("Location");
      await search.builder.addButton("condition").click();
      const row = search.builder.conditionRows.first();
      await row.locator(".builder-row__key").fill("name");
      await row.locator(".builder-row__value").fill(literal);
      await expect(search.builder.url).toHaveValue(expected);

      const requestSent = search.page.waitForRequest((candidate) => {
        const url = new URL(candidate.url());
        return url.pathname === "/Location" && url.searchParams.has("name");
      });
      await search.builder.runButton.click();
      const sentUrl = new URL((await requestSent).url());
      expect(sentUrl.searchParams.getAll("name")).toEqual([literal]);
      expect([...sentUrl.searchParams.keys()].sort()).toEqual(["_total", "name"]);
      await search.results.waitShown();
      await expect(search.results.rows).toHaveCount(2);
      await expect(
        search.results.rows.locator(`[data-resource-id="${ids[0]}"]`),
      ).toHaveCount(1);
      await expect(
        search.results.rows.locator(`[data-resource-id="${ids[1]}"]`),
      ).toHaveCount(1);
      await expect(
        search.results.rows.locator(`[data-resource-id="${decoy}"]`),
      ).toHaveCount(0);

      await search.builder.copyButton.click();
      await expect
        .poll(async () => page.evaluate(() => navigator.clipboard.readText()))
        .toBe(expected);

      savedFixture = await seedSavedQuery(request, "Location", savedName, "name=A%26A%20HEALTHCARE%20LLC");
      test.skip(!savedFixture, "no per-user settings store on this backend");

      await page.reload({ waitUntil: "networkidle" });
      await search.showBuilder();
      await search.builder.recentToggle.click();
      await search.builder.recentPanel.locator("[data-saved-load]", { hasText: savedName }).click();
      await expect(search.builder.url).toHaveValue(expected);
      const values = search.builder.conditionRows.first().locator(".builder-row__value");
      await expect(values).toHaveCount(1);
      await expect(values).toHaveValue(literal);

      const persisted = page.waitForResponse((response) => {
        const req = response.request();
        return (
          new URL(response.url()).pathname === "/_user/settings" &&
          req.method() === "PATCH" &&
          response.ok()
        );
      });
      const resent = page.waitForRequest((candidate) => {
        const url = new URL(candidate.url());
        return url.pathname === "/Location" && url.searchParams.has("name");
      });
      await search.builder.runButton.click();
      const [, reloaded] = await Promise.all([
        persisted,
        resent,
        search.results.waitShown(),
      ]);
      expect(new URL(reloaded.url()).searchParams.get("name")).toBe(literal);
      await page.reload({ waitUntil: "networkidle" });
      await search.showBuilder();
      await search.builder.recentToggle.click();
      await search.builder.recentPanel.getByRole("button", { name: expected, exact: true }).click();
      await expect(search.builder.url).toHaveValue(expected);
      await expect(
        search.builder.conditionRows.first().locator(".builder-row__value"),
      ).toHaveValue(literal);
    } finally {
      await savedFixture?.remove();
      await deleteResources(request, "Location", ids);
    }
  });

  test("URL encoding: untouched hydration and cross-row edits", async ({
    context,
    page,
    search,
  }) => {
    const hydrated = "Location?name=A%26A+HEALTHCARE+LLC&_count=2";
    const literal = "A&A HEALTHCARE LLC";
    await context.grantPermissions(["clipboard-read", "clipboard-write"]);

    await search.builder.setUrl(hydrated);
    await expect(search.builder.url).toHaveValue(hydrated);
    await search.builder.copyButton.click();
    await expect
      .poll(async () => page.evaluate(() => navigator.clipboard.readText()))
      .toBe(hydrated);

    const untouchedRequest = page.waitForRequest((request) => {
      const url = new URL(request.url());
      return url.pathname === "/Location" && url.searchParams.has("name");
    });
    await search.builder.runButton.click();
    const untouchedUrl = new URL((await untouchedRequest).url());
    expect(untouchedUrl.search).toBe(
      "?name=A%26A+HEALTHCARE+LLC&_count=2&_total=accurate",
    );
    expect(untouchedUrl.searchParams.getAll("name")).toEqual([literal]);
    await expect(search.builder.url).toHaveValue(hydrated);
    await expect
      .poll(async () => page.evaluate(() => navigator.clipboard.readText()))
      .toBe(hydrated);

    const countRow = page.locator("#builder-controls .builder-row");
    await expect(countRow).toHaveCount(1);
    await expect(countRow.locator(".builder-row__key")).toHaveValue("_count");
    await countRow.locator(".builder-row__value").fill("3");
    const canonical = "GET /Location?name=A%26A%20HEALTHCARE%20LLC&_count=3";
    await expect(search.builder.url).toHaveValue(canonical);

    const editedRequest = page.waitForRequest((request) => {
      const url = new URL(request.url());
      return url.pathname === "/Location" && url.searchParams.get("_count") === "3";
    });
    await search.builder.runButton.click();
    const editedUrl = new URL((await editedRequest).url());
    expect(editedUrl.searchParams.getAll("name")).toEqual([literal]);
    expect(editedUrl.searchParams.getAll("_count")).toEqual(["3"]);
    expect(editedUrl.searchParams.getAll("_total")).toEqual(["accurate"]);

    const sort = page.locator("#query-results-sort");
    await expect(sort).toBeEnabled();
    const sortedRequest = page.waitForRequest((request) => {
      const url = new URL(request.url());
      return url.pathname === "/Location" && url.searchParams.has("_sort");
    });
    await sort.selectOption("_lastUpdated");
    const sortedUrl = new URL((await sortedRequest).url());
    expect(sortedUrl.searchParams.getAll("name")).toEqual([literal]);
    expect(sortedUrl.searchParams.get("_sort")).toBe("_lastUpdated");
    await expect(search.builder.url).toHaveValue(
      canonical + "&_sort=_lastUpdated",
    );
  });

  test("URL encoding: typed values preserve FHIR syntax", async ({ search }) => {
    const cases = [
      {
        type: "Patient",
        key: "identifier",
        initial: null,
        literal: "http://example.org/a?x=1&y=2|A+B",
        wire: "http%3A%2F%2Fexample.org%2Fa%3Fx%3D1%26y%3D2%7CA%2BB",
        comparator: "",
      },
      {
        type: "Patient",
        key: "general-practitioner",
        initial: null,
        literal: "https://example.org/Practitioner/123",
        wire: "https%3A%2F%2Fexample.org%2FPractitioner%2F123",
        comparator: "",
      },
      {
        type: "Observation",
        key: "date",
        initial: "Observation?date=ge2026-09-21",
        literal: "2026-09-22T12:00:00+04:00",
        wire: "ge2026-09-22T12%3A00%3A00%2B04%3A00",
        comparator: "ge",
      },
    ];

    for (const sample of cases) {
      if (sample.initial) {
        await search.builder.setUrl(sample.initial);
      } else {
        await search.builder.setUrl(sample.type);
        await search.builder.addButton("condition").click();
        await search.builder.conditionRows
          .first()
          .locator(".builder-row__key")
          .fill(sample.key);
      }

      let row = search.builder.conditionRows.first();
      await expect(row.locator(".builder-row__comparator")).toHaveValue(
        sample.comparator,
      );
      await row.locator(".builder-row__value").fill(sample.literal);
      const expected = `GET /${sample.type}?${sample.key}=${sample.wire}`;
      await expect(search.builder.url).toHaveValue(expected);

      await search.builder.setUrl(expected);
      await expect(search.builder.conditionRows).toHaveCount(1);
      row = search.builder.conditionRows.first();
      await expect(row.locator(".builder-row__value")).toHaveValue(sample.literal);
      await expect(row.locator(".builder-row__comparator")).toHaveValue(
        sample.comparator,
      );
    }
  });

  test("URL encoding: reserved characters round-trip", async ({ page, search }) => {
    const cases = [
      {
        literal: "A&B + 50% #=? Muñoz",
        encoded: "A%26B%20%2B%2050%25%20%23%3D%3F%20Mu%C3%B1oz",
      },
      { literal: "A%26B", encoded: "A%2526B" },
    ];

    for (const sample of cases) {
      await search.builder.setUrl("Location");
      await search.builder.addButton("condition").click();
      const row = search.builder.conditionRows.first();
      await row.locator(".builder-row__key").fill("name");
      await row.locator(".builder-row__value").fill(sample.literal);
      const expected = `GET /Location?name=${sample.encoded}`;
      await expect(search.builder.url).toHaveValue(expected);

      const requestSent = page.waitForRequest((request) => {
        const url = new URL(request.url());
        return url.pathname === "/Location" && url.searchParams.has("name");
      });
      await search.builder.runButton.click();
      const sentUrl = new URL((await requestSent).url());
      expect(sentUrl.search).toBe(`?name=${sample.encoded}&_total=accurate`);
      expect(sentUrl.searchParams.getAll("name")).toEqual([sample.literal]);
      expect([...sentUrl.searchParams.keys()].sort()).toEqual(["_total", "name"]);
      expect(sentUrl.hash).toBe("");

      await search.builder.setUrl(expected);
      await expect(search.builder.conditionRows).toHaveCount(1);
      await expect(
        search.builder.conditionRows.first().locator(".builder-row__value"),
      ).toHaveValue(sample.literal);
    }
  });

  test("escaped commas survive Copy, Saved and Recent reloads", async ({
    context,
    page,
    search,
    request,
  }) => {
    const tag = Date.now().toString(36);
    const name = `Escaped comma ${tag}`;
    const query = `Patient?name:exact=Copy%5C%2C${tag}`;
    const getQuery = `GET /${query}`;
    await context.grantPermissions(["clipboard-read", "clipboard-write"]);

    await search.builder.setUrl(query);
    await search.builder.copyButton.click();
    await expect
      .poll(async () => page.evaluate(() => navigator.clipboard.readText()))
      .toBe(query);

    const savedFixture = await seedSavedQuery(request, "Patient", name, `name:exact=Copy%5C%2C${tag}`);
    test.skip(!savedFixture, "no per-user settings store on this backend");
    try {

      await page.reload({ waitUntil: "networkidle" });
      await search.showBuilder();
      await search.builder.recentToggle.click();
      await search.builder.recentPanel.locator("[data-saved-load]", { hasText: name }).click();
      await expect(search.builder.url).toHaveValue(getQuery);
      await expect(search.builder.conditionRows.first().locator(".builder-row__value")).toHaveValue(
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
      await search.builder.runButton.click();
      const [, sent] = await Promise.all([
        recentSaved,
        requestSent,
        search.results.waitShown(),
      ]);
      expect(new URL(sent.url()).searchParams.get("name:exact")).toBe(`Copy\\,${tag}`);
      await page.reload({ waitUntil: "networkidle" });
      await search.showBuilder();
      await search.builder.recentToggle.click();
      await search.builder.recentPanel.getByRole("button", { name: getQuery, exact: true }).click();
      await expect(search.builder.url).toHaveValue(getQuery);
      await expect(search.builder.conditionRows.first().locator(".builder-row__value")).toHaveValue(
        `Copy,${tag}`,
      );
    } finally {
      await savedFixture?.remove();
    }
  });
});

// The condition parameter and the first chain segment are a typeahead
// (typeahead.js) fed by the per-type catalog (#1643).
test.describe("query builder parameter typeahead", () => {
  const PATIENT_CATALOG = catalogHtml([
    { code: "head-circumference", type: "quantity" },
    { code: "address", type: "string" },
    { code: "address-city", type: "string" },
    { code: "birthdate", type: "date" },
    { code: "death-date", type: "date" },
    { code: "_lastUpdated", type: "date" },
    { code: "name", type: "string" },
    { code: "general-practitioner", type: "reference", targets: ["Organization", "Practitioner"] },
  ]);

  test.beforeEach(async ({ page, search }) => {
    await page.route("**/ui/resources/params?type=Patient", async (route) => {
      await route.fulfill({ contentType: "text/html", body: PATIENT_CATALOG });
    });
    await search.gotoBuilder();
    await search.builder.setUrl("Patient");
    await search.builder.addButton("condition").click();
    await expect(search.builder.conditionRows).toHaveCount(1);
  });

  const key = (search: { builder: { conditionRows: import("@playwright/test").Locator } }) =>
    search.builder.conditionRows.first().locator(".builder-row__key");

  test("typeahead: focusing the parameter opens a combobox listing the whole catalog with type pills", async ({
    search,
  }) => {
    const input = key(search);
    // Add leaves the field focused with the list closed; a click opens it.
    await expect(input).toHaveAttribute("aria-expanded", "false");
    await input.click();
    await expect(input).toHaveAttribute("role", "combobox");
    await expect(input).toHaveAttribute("aria-expanded", "true");
    await expect(input).not.toHaveAttribute("list", /.*/);
    await expect(search.builder.typeaheadVisibleListbox).toBeVisible();
    await expect(search.builder.typeaheadOptionValues).toHaveText([
      "head-circumference",
      "address",
      "address-city",
      "birthdate",
      "death-date",
      "_lastUpdated",
      "name",
      "general-practitioner",
    ]);
    await expect(search.builder.typeaheadOptions.first().locator(".typeahead__hint")).toHaveText(
      "quantity",
    );
    await expect(search.builder.typeaheadOptions.nth(1).locator(".typeahead__hint")).toHaveText(
      "string",
    );
    await expect(search.builder.typeaheadOptions.nth(3).locator(".typeahead__hint")).toHaveText(
      "date",
    );
  });

  test("typeahead: two real clicks on Add condition add two rows and pick nothing", async ({
    search,
  }) => {
    const before = await search.builder.url.inputValue();
    await search.builder.addButton("condition").click();
    await expect(search.builder.conditionRows).toHaveCount(2);
    await expect(search.builder.typeaheadVisibleListbox).toHaveCount(0);
    for (const row of await search.builder.conditionRows.all()) {
      await expect(row.locator(".builder-row__key")).toHaveValue("");
    }
    await expect(search.builder.url).toHaveValue(before);
  });

  test("typeahead: after Add, a click on the focused parameter opens the list", async ({
    search,
  }) => {
    const input = key(search);
    await expect(input).toBeFocused();
    await expect(search.builder.typeaheadVisibleListbox).toHaveCount(0);
    await input.click();
    await expect(search.builder.typeaheadVisibleListbox).toBeVisible();
  });

  test("typeahead: a field focused before the catalog arrives refreshes when it loads", async ({
    page,
    search,
  }) => {
    let release: () => void = () => {};
    const gate = new Promise<void>((resolve) => (release = resolve));
    await page.route("**/ui/resources/params?type=Observation", async (route) => {
      await gate;
      await route.fulfill({
        contentType: "text/html",
        body: catalogHtml([{ code: "code", type: "token" }]),
      });
    });
    await search.builder.setUrl("Observation");
    if ((await search.builder.conditionRows.count()) === 0) {
      await search.builder.addButton("condition").click();
    }
    const input = key(search);
    await input.click();
    await input.fill("c");
    await expect(search.builder.typeaheadVisibleListbox).toContainText("No matching parameters");
    release();
    await expect(search.builder.typeaheadOptionValues).toHaveText(["code"]);
  });

  test("typeahead: filtering matches any part of the name or the type", async ({ search }) => {
    const input = key(search);
    const values = search.builder.typeaheadOptionValues;
    await input.focus();
    await input.fill("ad");
    // Prefix matches first; "head-circumference" only contains "ad" in the
    // middle and comes first in the catalog, yet ranks last.
    await expect(values).toHaveText(["address", "address-city", "head-circumference"]);
    await input.fill("city");
    await expect(values).toHaveText(["address-city"]);
    await input.fill("BIRTH");
    await expect(values).toHaveText(["birthdate"]);
    await input.fill("lastupdated");
    await expect(values).toHaveText(["_lastUpdated"]);
    await input.fill("date");
    await expect(values).toHaveText(["birthdate", "death-date", "_lastUpdated"]);
  });

  test("typeahead: ArrowDown then Enter chooses the active option", async ({ search }) => {
    const input = key(search);
    await input.focus();
    await input.fill("birth");
    await input.press("ArrowDown");
    await expect(input).toHaveAttribute("aria-activedescendant", /typeahead-list-\d+-opt-0/);
    await input.press("Enter");
    await expect(input).toHaveValue("birthdate");
    await expect(search.builder.typeaheadVisibleListbox).toHaveCount(0);
    await expect(input).toHaveAttribute("aria-expanded", "false");
    await expect(input).not.toHaveAttribute("aria-activedescendant", /.*/);
    await expect(search.builder.url).toHaveValue(/birthdate=/);
  });

  test("typeahead: Escape closes keeping the text, and a click chooses without losing focus", async ({
    page,
    search,
  }) => {
    const input = key(search);
    await input.focus();
    await input.fill("nam");
    await input.press("Escape");
    await expect(search.builder.typeaheadVisibleListbox).toHaveCount(0);
    await expect(input).toHaveAttribute("aria-expanded", "false");
    await expect(input).toHaveValue("nam");

    await input.fill("na");
    await search.builder.typeaheadOptions.filter({ hasText: "name" }).first().click();
    await expect(input).toHaveValue("name");
    await expect(search.builder.typeaheadVisibleListbox).toHaveCount(0);
    await expect(input).toHaveAttribute("aria-expanded", "false");
    await expect(input).toBeFocused();
    await expect(search.builder.url).toHaveValue(/name=/);
    expect(await page.evaluate(() => document.activeElement?.className)).toContain(
      "builder-row__key",
    );
  });

  test("typeahead: an unmatched query shows the empty text and no options", async ({ search }) => {
    const input = key(search);
    await input.focus();
    await input.fill("zzz");
    await expect(search.builder.typeaheadVisibleListbox).toContainText("No matching parameters");
    await expect(search.builder.typeaheadOptions).toHaveCount(0);
  });

  test("typeahead: choosing a reference parameter reveals the drill-in button", async ({
    search,
  }) => {
    const row = search.builder.conditionRows.first();
    const input = key(search);
    await expect(search.builder.drillButton(row)).toBeHidden();
    await input.focus();
    await input.fill("general");
    await input.press("ArrowDown");
    await input.press("Enter");
    await expect(input).toHaveValue("general-practitioner");
    await expect(search.builder.drillButton(row)).toBeVisible();
  });

  test("typeahead: the first chain segment is a typeahead and later segments keep the datalist", async ({
    search,
  }) => {
    await search.builder.setUrl("Patient?general-practitioner.organization.name=Smith");
    await expect(search.builder.chainRows).toHaveCount(1);
    const refs = search.builder.chainRows.first().locator(".builder-row__chainref");
    await expect(refs).toHaveCount(2);
    await expect(refs.nth(0)).toHaveAttribute("role", "combobox");
    await expect(refs.nth(0)).not.toHaveAttribute("list", /.*/);
    await expect(refs.nth(1)).not.toHaveAttribute("role", /.*/);
    await expect(refs.nth(1)).toHaveAttribute("list", /.+/);
    await refs.nth(0).focus();
    await refs.nth(0).fill("general");
    await expect(search.builder.typeaheadOptionValues).toHaveText(["general-practitioner"]);
  });

  test("typeahead: replacing a long value keeps the list open", async ({ page, search }) => {
    await search.builder.setUrl("Patient");
    await search.builder.addButton("condition").click();
    const input = search.builder.conditionRows.first().locator(".builder-row__key");
    await input.focus();
    await input.fill("general-practitioner-general-practitioner-general-practitioner");
    await input.fill("general");
    await page.waitForTimeout(300);
    await expect(search.builder.typeaheadOptionValues).toHaveText(["general-practitioner"]);
  });

  test("typeahead: rebuilding the rows leaves no orphan listboxes", async ({ search }) => {
    await search.builder.setUrl("Patient?name=a&birthdate=ge1980&general-practitioner.name=x");
    await expect(search.builder.conditionRows).toHaveCount(3);
    await search.builder.setUrl("Patient?name=b");
    await expect(search.builder.conditionRows).toHaveCount(1);
    await expect(search.builder.typeaheadListboxes).toHaveCount(
      await search.builder.typeaheadComboboxes.count(),
    );
    await expect(search.builder.typeaheadComboboxes).toHaveCount(1);
    await search.builder.conditionRows.first().locator("[data-remove-row]").click();
    await expect(search.builder.typeaheadListboxes).toHaveCount(0);
  });

  test("typeahead: the listbox stays inside a narrow viewport", async ({ page, search }) => {
    await page.setViewportSize({ width: 360, height: 700 });
    await page.evaluate(
      () => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))),
    );
    const input = key(search);
    await input.scrollIntoViewIfNeeded();
    await input.focus();
    await input.pressSequentially("a");
    // Measure in the same task that opens the list: later layout scrolls
    // close it by design.
    const box = await page.evaluate(() => {
      const input = document.querySelector<HTMLInputElement>(
        "#builder-conditions .builder-row__key",
      )!;
      input.dispatchEvent(new Event("input", { bubbles: true }));
      const listbox = document.querySelector<HTMLElement>("body > .typeahead__listbox")!;
      const rect = listbox.getBoundingClientRect();
      return { hidden: listbox.hidden, left: rect.left, right: rect.right };
    });
    expect(box.hidden).toBe(false);
    expect(box.left).toBeGreaterThanOrEqual(0);
    expect(box.right).toBeLessThanOrEqual(360);
  });
});

// A condition whose parameter is not in the catalog of the resource type is
// flagged, never blocked (#1643).
test.describe("query builder unknown parameter", () => {
  const PATIENT = catalogHtml([
    { code: "name", type: "string" },
    { code: "birthdate", type: "date" },
    { code: "general-practitioner", type: "reference", targets: ["Practitioner"] },
  ]);
  const OBSERVATION = catalogHtml([
    { code: "code", type: "token" },
    { code: "subject", type: "reference", targets: ["Patient"] },
  ]);
  const MESSAGE = "Not a search parameter for Patient. Pick one from the list.";

  test.beforeEach(async ({ page, search }) => {
    await page.route("**/ui/resources/params?type=Patient", async (route) => {
      await route.fulfill({ contentType: "text/html", body: PATIENT });
    });
    await page.route("**/ui/resources/params?type=Observation", async (route) => {
      await route.fulfill({ contentType: "text/html", body: OBSERVATION });
    });
    await search.gotoBuilder();
  });

  async function typeUnknown(search: any, text: string) {
    await search.builder.setUrl("Patient");
    await search.builder.addButton("condition").click();
    const input = search.builder.conditionRows.first().locator(".builder-row__key");
    await input.fill(text);
    await input.blur();
    return input;
  }

  test("unknown parameter: leaving the field flags it with a visible message", async ({
    search,
  }) => {
    const input = await typeUnknown(search, "asdasd");
    await expect(input).toHaveAttribute("aria-invalid", "true");
    const id = await input.getAttribute("aria-describedby");
    expect(id).toBeTruthy();
    const message = search.builder.page.locator(`#${id}`);
    await expect(message).toBeVisible();
    await expect(message).toHaveText(MESSAGE);
  });

  test("unknown parameter: nothing is blocked and the sentence names it", async ({ search }) => {
    await typeUnknown(search, "asdasd");
    await expect(search.builder.url).toHaveValue(/asdasd=/);
    await expect(search.builder.runButton).toBeEnabled();
    await expect(search.builder.plainUnknown).toBeVisible();
    await expect(search.builder.plainUnknown).toContainText(
      '"asdasd" is not a search parameter for Patient',
    );
  });

  test("unknown parameter: choosing a listed parameter clears the flag", async ({ search }) => {
    const input = await typeUnknown(search, "asdasd");
    await expect(input).toHaveAttribute("aria-invalid", "true");
    await input.click();
    await input.fill("nam");
    await search.builder.typeaheadOptions.filter({ hasText: "name" }).first().click();
    await expect(input).not.toHaveAttribute("aria-invalid", /.*/);
    await expect(search.builder.rowError(search.builder.conditionRows.first())).toHaveCount(0);
    await expect(search.builder.plainUnknown).toBeHidden();
  });

  test("unknown parameter: editing a flagged field clears the mark until it is confirmed", async ({
    search,
  }) => {
    const input = await typeUnknown(search, "asdasd");
    await expect(input).toHaveAttribute("aria-invalid", "true");
    await input.focus();
    await input.pressSequentially("x");
    await expect(input).not.toHaveAttribute("aria-invalid", /.*/);
    await expect(search.builder.flaggedInputs).toHaveCount(0);
    await input.blur();
    await expect(input).toHaveAttribute("aria-invalid", "true");
  });

  test("unknown parameter: reverting an edit to the same value marks the row again", async ({
    search,
  }) => {
    const input = await typeUnknown(search, "asdasd");
    await expect(input).toHaveAttribute("aria-invalid", "true");
    await input.focus();
    await input.pressSequentially("x");
    await expect(search.builder.flaggedInputs).toHaveCount(0);
    await input.press("Backspace");
    await input.blur();
    await expect(input).toHaveAttribute("aria-invalid", "true");
    await expect(search.builder.flaggedInputs).toHaveCount(1);
  });

  test("unknown parameter: the sentence follows the flagged rows, not the keystrokes", async ({
    search,
  }) => {
    await search.builder.setUrl("Patient");
    await search.builder.addButton("condition").click();
    const input = search.builder.conditionRows.first().locator(".builder-row__key");
    await input.focus();
    await input.pressSequentially("nam");
    await expect(search.builder.plainUnknown).toBeHidden();
    await input.fill("asdasd");
    await expect(search.builder.plainUnknown).toBeHidden();
    await input.blur();
    await expect(search.builder.plainUnknown).toContainText(
      '"asdasd" is not a search parameter for Patient',
    );
    await input.focus();
    await input.pressSequentially("x");
    await expect(search.builder.plainUnknown).toBeHidden();
  });

  test("unknown parameter: a modifier typed in the key field is judged by its base name", async ({
    search,
  }) => {
    const input = await typeUnknown(search, "name:exact");
    await expect(search.builder.flaggedInputs).toHaveCount(0);
    await expect(input).not.toHaveAttribute("aria-invalid", /.*/);
  });

  test("unknown parameter: standard FHIR parameters outside the catalog are never flagged", async ({
    search,
  }) => {
    await search.builder.setUrl("Patient?_list=abc&_filter=x");
    await expect(search.builder.url).toHaveValue(/_list=abc/);
    await expect(search.builder.flaggedInputs).toHaveCount(0);
    await expect(search.builder.plainUnknown).toBeHidden();

    await search.builder.setUrl("Patient?_bogus=1");
    await expect(search.builder.flaggedInputs).toHaveCount(1);
  });

  test("unknown parameter: a modifier on a known parameter is not flagged", async ({ search }) => {
    await search.builder.setUrl("Patient?name:exact=smith");
    await expect(search.builder.conditionRows).toHaveCount(1);
    await expect(search.builder.plainText).toContainText("Patient");
    await expect(search.builder.flaggedInputs).toHaveCount(0);
    await expect(search.builder.plainUnknown).toBeHidden();
  });

  test("unknown parameter: only the first chain hop is judged", async ({ search }) => {
    await search.builder.setUrl("Patient?general-practitioner.name=x");
    await expect(search.builder.chainRows).toHaveCount(1);
    await expect(search.builder.flaggedInputs).toHaveCount(0);

    await search.builder.setUrl("Patient?bogus.name=x");
    const row = search.builder.chainRows.first();
    const first = row.locator(".builder-row__hopseg .builder-row__chainref").first();
    await expect(first).toHaveAttribute("aria-invalid", "true");
    await expect(search.builder.rowError(row)).toHaveText(MESSAGE);
  });

  test("unknown parameter: a _has row is never flagged", async ({ search }) => {
    await search.builder.setUrl("Patient?_has:Observation:patient:code=1234");
    await expect(search.builder.hasRows).toHaveCount(1);
    await expect(search.builder.flaggedInputs).toHaveCount(0);
    await expect(search.builder.plainUnknown).toBeHidden();
  });

  test("unknown parameter: a resource type change re-judges the rows", async ({ search }) => {
    await search.builder.setUrl("Patient?birthdate=1990");
    await expect(search.builder.conditionRows).toHaveCount(1);
    await expect(search.builder.flaggedInputs).toHaveCount(0);

    await search.builder.setUrl("Observation?birthdate=1990");
    const row = search.builder.conditionRows.first();
    await expect(row.locator(".builder-row__key")).toHaveAttribute("aria-invalid", "true");
    await expect(search.builder.rowError(row)).toHaveText(
      "Not a search parameter for Observation. Pick one from the list.",
    );
  });

  test("unknown parameter: a failed catalog flags nothing", async ({ page, search }) => {
    await page.unroute("**/ui/resources/params?type=Patient");
    await page.route("**/ui/resources/params?type=Patient", async (route) => {
      await route.fulfill({ status: 500, body: "boom" });
    });
    const answered = page.waitForResponse("**/ui/resources/params?type=Patient");
    await search.builder.setUrl("Patient?asdasd=x");
    await answered;
    await expect(search.builder.plainText).toContainText("Patient");
    await expect(search.builder.conditionRows).toHaveCount(1);
    await expect(search.builder.flaggedInputs).toHaveCount(0);
    await expect(search.builder.plainUnknown).toBeHidden();
  });

  for (const theme of ["light", "dark"] as const) {
    test(`unknown parameter: a flagged row passes axe — ${theme}`, async ({
      page,
      chrome,
      search,
    }) => {
      await chrome.seedTheme(theme);
      await search.gotoBuilder();
      await typeUnknown(search, "asdasd");
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await expect(search.builder.flaggedInputs).toHaveCount(1);
      // The row's modifier <select> has no accessible name today (a gap that
      // predates this state and is not part of it), so it is left out here.
      const { violations } = await new AxeBuilder({ page })
        .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa"])
        .exclude(".builder-row__modifier")
        .analyze();
      expect(violations, axeSummary(violations)).toEqual([]);
    });
  }
});

searchLifecycleTests("/ui/search");

test("issue1577 first search cancellation returns to the initial empty state", async ({ page, search }) => {
  const pending = await holdSearches(page);
  await search.gotoBuilder();
  await expect(search.results.card).toBeHidden();
  await search.builder.run("Patient?_id=issue1577-first");
  await expect.poll(() => pending.held.has("issue1577-first")).toBe(true);
  await search.builder.cancel.click();
  await expect(search.results.card).toBeHidden();
  await expect(search.results.rows).toHaveCount(0);
  await expect(search.results.error).toBeHidden();
});

for (const [locale, cancel, elapsed] of [
  ["en", "Cancel", "2 seconds elapsed"],
  ["es", "Cancelar", "2 segundos transcurridos"],
  ["de", "Abbrechen", "2 Sekunden vergangen"],
]) {
  test(`issue1577 waiting controls are translated — ${locale}`, async ({ page, search }) => {
    await page.clock.install();
    const pending = await holdSearches(page);
    await page.goto(`/ui/search?lang=${locale}`, { waitUntil: "networkidle" });
    await search.showBuilder();
    await page.clock.pauseAt(await page.evaluate(() => Date.now() + 1000));
    await search.builder.run("Patient?_id=issue1577-localized");
    await expect.poll(() => pending.held.has("issue1577-localized")).toBe(true);
    await expect(search.builder.cancel).toHaveText(cancel);
    await page.clock.runFor(2000);
    await expect(search.builder.elapsed).toHaveText(elapsed);
    await search.builder.cancel.click();
    await page.clock.resume();
  });
}

test("issue1577 cancelling a search does not release a blocked builder", async ({ page, search }) => {
  const pending = await holdSearches(page);
  let catalog: import("@playwright/test").Route | undefined;
  await page.route("**/ui/resources/params?type=Patient", route => { catalog = route; });
  await search.gotoBuilder();
  await search.builder.run("Patient?_id=issue1577-blocked");
  await expect.poll(() => pending.held.has("issue1577-blocked")).toBe(true);
  await search.builder.setUrl("Patient?name:contains=Alpha");
  const row = search.builder.conditionRows.first();
  await row.locator(".builder-row__key").fill("gender");
  await expect(search.builder.runButton).toBeDisabled();
  await search.builder.cancel.click();
  await expect(search.builder.runButton).toBeDisabled();
  await expect.poll(() => !!catalog).toBe(true);
  await catalog!.continue();
  await expect(search.builder.runButton).toBeEnabled();
});


for (const workspace of ["Resources", "Search"] as const) {
  test(`issue1772 ${workspace} loads an existing Saved query and explicitly runs it into Recent`, async ({ page, request, resources, search }) => {
    const id = await createResource(request, "Patient", { name: [{ family: `Saved1772${workspace}` }] });
    let saved: Awaited<ReturnType<typeof seedSavedQuery>> = null;
    try {
      saved = await seedSavedQuery(request, "Patient", `Saved1772 ${workspace} ${id}`, `_id=${id}`);
      test.skip(!saved, "no per-user settings store on this backend");
      const active = workspace === "Resources" ? resources : search;
      if (workspace === "Resources") await resources.goto("Patient");
      else await search.gotoBuilder("Patient");
      const mode = page.locator("[data-mode-btn=builder]");
      if (await mode.count()) await mode.click();
      let runs = 0;
      page.on("request", candidate => { if (new URL(candidate.url()).pathname === "/Patient" && new URL(candidate.url()).searchParams.get("_id") === id) runs++; });
      await active.builder.recentToggle.click();
      await active.builder.recentPanel.locator("[data-saved-load]", { hasText: saved!.entry.name }).click();
      await expect(active.builder.url).toHaveValue(`GET /Patient?_id=${id}`);
      expect(runs, "loading Saved leaves execution to Run").toBe(0);
      await active.builder.runButton.click();
      await active.results.waitShown();
      await expect(active.results.rows).toHaveCount(1);
      await expect(active.results.rows).toContainText(id);
      expect(runs).toBe(1);
      await expect.poll(async () => {
        const document = await (await request.get("/_user/settings")).json();
        return document.recentSearches?.some((entry: { query: string }) => entry.query === `/Patient?_id=${id}`);
      }).toBe(true);
      const document = await (await request.get("/_user/settings")).json();
      expect(document.savedQueries.Patient[saved!.id]).toEqual(saved!.entry);
    } finally {
      await saved?.remove();
      await request.delete(`/Patient/${id}`);
    }
  });
}

test("issue1772 visual builder and parameter suggestions work when settings are unavailable", async ({ page, search }) => {
  await page.route("**/_user/settings", route => route.fulfill({ status: 501, contentType: "application/fhir+json", body: JSON.stringify({ resourceType: "OperationOutcome" }) }));
  await search.gotoBuilder();
  await search.builder.setUrl("Patient?name=Issue1772Unavailable");
  await expect(search.builder.form).toBeVisible();
  await expect(search.builder.typeaheadComboboxes).toHaveCount(1);
  await search.builder.typeaheadComboboxes.first().focus();
  await expect(search.builder.typeaheadOptions.first()).toBeVisible();
  await search.builder.runButton.click();
  await search.results.waitShown();
  await expect(search.results.error).toBeHidden();
});
