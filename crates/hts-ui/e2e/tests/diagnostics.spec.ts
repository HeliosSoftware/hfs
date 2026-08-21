import { expect, test } from "@playwright/test";

// Phase 2 Slice G: the standalone Diagnostics page at
// `/ui/hts/diagnostics`. Mirrors the Rust-side coverage in
// `crates/hts-ui/tests/diagnostics.rs` (5 tokio tests, green as of
// the 78/0 Slice G baseline) but exercises the a11y contract +
// `hx-push-url` deep-linking that HTML-only Rust http tests cannot
// reach.
//
// Design ref: `edson/docs/hts-ui-design.md` §7.9 Diagnostics and
// §7.10 row 7.9 (per-tab OperationOutcome isolation, plain-anchor
// nojs fallback). Handler: `crates/hts-ui/src/diagnostics.rs`.
// Templates: `templates/pages/diagnostics.html` +
// `templates/partials/hts-diagnostics-panel.html`.
//
// Tab slugs / query-param values (must match `Tab::slug` in the
// handler): capability | terminology-capabilities | health | metrics.
// The `?tab=` query param name is fixed by `TabQuery` (serde-renamed
// nowhere; the field is literally `tab`).
//
// Boot fixture (see e2e/boot.mjs): the Playwright suite boots a real
// `hts` binary against a throwaway SQLite DB with `HTS_UI_ENABLED=1`.
// Every diagnostic endpoint (/metadata, /metadata?mode=terminology,
// /health, /metrics) is served by the same in-process HTS, so the
// three "healthy" tabs all return 200 during a suite run.

test.describe("HTS Diagnostics page shell (§7.9)", () => {
  test("responds at /ui/hts/diagnostics with the H1 + four-tab strip", async ({
    page,
  }) => {
    const response = await page.goto("/ui/hts/diagnostics");
    expect(response?.status(), "diagnostics route must respond 200").toBe(200);
    // H1 comes from Fluent `hts-diagnostics-heading` (en: "Diagnostics").
    await expect(
      page.getByRole("heading", { name: /Diagnostics/i, level: 1 }),
    ).toBeVisible();
    // Single tablist with four tab roles (one per diagnostic source).
    const tablist = page.getByRole("tablist");
    await expect(tablist).toBeVisible();
    await expect(tablist.getByRole("tab")).toHaveCount(4);
    // Every tab is labelled per the en Fluent stub in
    // `locales/en/main.ftl` (`hts-diagnostics-tab-*`).
    for (const label of [
      "Capability",
      "TerminologyCapabilities",
      "/health",
      "/metrics",
    ]) {
      await expect(
        page.getByRole("tab", { name: label, exact: true }),
      ).toBeVisible();
    }
  });

  test("defaults to the Capability tab with aria-selected=true", async ({
    page,
  }) => {
    // §7.9 default: an unknown / missing `?tab=` collapses to
    // Capability (see `Tab::from_slug` in diagnostics.rs).
    await page.goto("/ui/hts/diagnostics");
    await expect(
      page.getByRole("tab", { name: "Capability", exact: true }),
    ).toHaveAttribute("aria-selected", "true");
    for (const label of ["TerminologyCapabilities", "/health", "/metrics"]) {
      await expect(
        page.getByRole("tab", { name: label, exact: true }),
      ).toHaveAttribute("aria-selected", "false");
    }
    // The shared `<section id="diag-panel" role="tabpanel">` is
    // visible with the default (Capability) content pre-rendered.
    await expect(page.getByRole("tabpanel")).toBeVisible();
    await expect(page.locator("#diag-panel")).toBeVisible();
  });

  test("nav bar exposes Diagnostics after Import", async ({ page }) => {
    await page.goto("/ui/hts");
    const navLinks = page
      .locator("nav")
      .getByRole("link")
      .filter({
        hasText: /Home|Code|Value|Concept|Operations|Import|Diagnostics/,
      });
    const names = await navLinks.allTextContents();
    const importIdx = names.findIndex((n) => /Import/i.test(n));
    const diagIdx = names.findIndex((n) => /Diagnostics/i.test(n));
    expect(importIdx, "Import must be present in the sidebar nav").toBeGreaterThan(
      -1,
    );
    expect(diagIdx, "Diagnostics must be present in the sidebar nav").toBeGreaterThan(
      -1,
    );
    expect(
      diagIdx,
      "Diagnostics must appear after Import in the sidebar nav",
    ).toBeGreaterThan(importIdx);
  });
});

test.describe("HTS Diagnostics tab swap (§7.9 hx-push-url)", () => {
  test("clicking TerminologyCapabilities swaps the panel and pushes the URL", async ({
    page,
  }) => {
    await page.goto("/ui/hts/diagnostics");
    const tcTab = page.getByRole("tab", {
      name: "TerminologyCapabilities",
      exact: true,
    });
    // Every tab carries the hx-* contract (design invariant): a
    // partial GET into the shared `#diag-panel` region, innerHTML
    // swap, and browser-history push for deep-linkability.
    await expect(tcTab).toHaveAttribute(
      "hx-get",
      /\/ui\/hts\/diagnostics\/panel\?tab=terminology-capabilities/,
    );
    await expect(tcTab).toHaveAttribute("hx-target", "#diag-panel");
    await expect(tcTab).toHaveAttribute("hx-swap", "innerHTML");
    await expect(tcTab).toHaveAttribute("hx-push-url", "true");

    await tcTab.click();
    // hx-push-url="true" updates the browser URL to the request URL.
    // We wait on a substring so this test does not have to encode
    // whether htmx picks the shell URL or the panel URL — both
    // contain `tab=terminology-capabilities`.
    await page.waitForURL(/tab=terminology-capabilities/);
    // The panel body now renders the TerminologyCapabilities arm of
    // `hts-diagnostics-panel.html`, whose H2 is the Fluent value
    // `hts-diagnostics-terminology-capabilities-heading`.
    await expect(page.getByRole("tabpanel")).toContainText(
      /TerminologyCapabilities/,
    );
  });

  test("clicking /health swaps the panel and pushes tab=health", async ({
    page,
  }) => {
    await page.goto("/ui/hts/diagnostics");
    await page.getByRole("tab", { name: "/health", exact: true }).click();
    await page.waitForURL(/tab=health/);
    // The H2 rendered by the health arm of the panel template is
    // `hts-diagnostics-health-heading` = "Health".
    await expect(page.getByRole("tabpanel")).toContainText(/Health/i);
  });

  test("deep-linking /ui/hts/diagnostics?tab=metrics pre-selects the metrics tab", async ({
    page,
  }) => {
    // Nojs / deep-link contract (§7.10 row 7.9 "plain anchor tabs"):
    // hitting the URL directly must land with the metrics tab
    // aria-selected and its panel body pre-rendered inside
    // `#diag-panel` — no htmx swap involved.
    await page.goto("/ui/hts/diagnostics?tab=metrics");
    await expect(
      page.getByRole("tab", { name: "/metrics", exact: true }),
    ).toHaveAttribute("aria-selected", "true");
    // The three other tabs must be aria-selected=false at initial
    // paint — no double-active state.
    for (const label of ["Capability", "TerminologyCapabilities", "/health"]) {
      await expect(
        page.getByRole("tab", { name: label, exact: true }),
      ).toHaveAttribute("aria-selected", "false");
    }
  });
});

test.describe("HTS Diagnostics per-tab content shape (§7.9)", () => {
  test("Capability tab surfaces a CapabilityStatement marker", async ({
    page,
  }) => {
    await page.goto("/ui/hts/diagnostics?tab=capability");
    // Fluent `hts-diagnostics-capability-heading` = "CapabilityStatement".
    // The Rust-side test `capability_tab_renders_property_table` in
    // `crates/hts-ui/tests/diagnostics.rs` locks in the property
    // table + url/version cells against a seeded mock; here we
    // only assert the resourceType marker so the spec is robust
    // to whatever CapabilityStatement HTS actually emits.
    await expect(page.getByRole("tabpanel")).toContainText(
      /CapabilityStatement/i,
    );
  });

  test("TerminologyCapabilities tab surfaces the resourceType marker", async ({
    page,
  }) => {
    await page.goto("/ui/hts/diagnostics?tab=terminology-capabilities");
    // Fluent `hts-diagnostics-terminology-capabilities-heading` =
    // "TerminologyCapabilities". Any richer content (codeSystem[]
    // list) depends on the seed; the heading marker is stable.
    await expect(page.getByRole("tabpanel")).toContainText(
      /TerminologyCapabilities/,
    );
  });

  test("/health tab surfaces the Status label with an up-ish value", async ({
    page,
  }) => {
    await page.goto("/ui/hts/diagnostics?tab=health");
    const panel = page.getByRole("tabpanel");
    // The tab's H2 = "Health" (Fluent `hts-diagnostics-health-heading`).
    // The DL renders a Status row whose value comes from the upstream
    // `/health` `status` field — HTS emits `ok` today.
    await expect(panel).toContainText(/Health/i);
    await expect(panel).toContainText(/\b(ok|up|healthy|degraded)\b/i);
  });

  test("/metrics tab wraps Prometheus text-format in a <pre>", async ({
    page,
  }) => {
    await page.goto("/ui/hts/diagnostics?tab=metrics");
    const panel = page.getByRole("tabpanel");
    // Fluent `hts-diagnostics-metrics-heading` = "Prometheus metrics".
    await expect(panel).toContainText(/Prometheus metrics/i);
    // The metrics body is wrapped in `<pre class="hts-diagnostics-metrics__body"><code>…</code></pre>`
    // (see `hts-diagnostics-panel.html`). If HTS returns an empty
    // body the tab falls back to the neutral "no body" state — we
    // accept either shape so the spec does not lock in whether
    // `/metrics` is populated at boot time. The Rust-side test
    // `metrics_tab_renders_prometheus_text_verbatim` covers the
    // populated case against a seeded mock.
    const preBlock = panel.locator("pre");
    const emptyState = panel.getByText(/no body/i);
    await expect(preBlock.or(emptyState).first()).toBeVisible();
    // When a <pre> renders it must carry Prometheus signature text
    // — a `# HELP` / `# TYPE` line or a metric name prefix.
    if ((await preBlock.count()) > 0) {
      await expect(preBlock).toContainText(/#\s+(HELP|TYPE)\b|\bhts_\w+|\b\w+\{/m);
    }
  });
});

test.describe("HTS Diagnostics per-tab error isolation (§7.9)", () => {
  test("each tab targets the shared #diag-panel with innerHTML swap (design invariant)", async ({
    page,
  }) => {
    // The per-tab OperationOutcome isolation contract from §7.9 is
    // guaranteed at the DOM level: every tab anchor swaps only into
    // `#diag-panel`, never into the tab strip itself. An outage on
    // one tab therefore cannot disable navigation to the others.
    // The Rust-side outcome test
    // (`any_tab_5xx_renders_outcome_in_diag_panel_only` in
    // `crates/hts-ui/tests/diagnostics.rs`) exercises the actual
    // 500-on-/health outcome render against a seeded mock; here we
    // lock down the structural invariant that makes that outcome
    // safe under all tabs.
    await page.goto("/ui/hts/diagnostics");
    const slugsByLabel: Array<[string, string]> = [
      ["Capability", "capability"],
      ["TerminologyCapabilities", "terminology-capabilities"],
      ["/health", "health"],
      ["/metrics", "metrics"],
    ];
    for (const [label, slug] of slugsByLabel) {
      const tab = page.getByRole("tab", { name: label, exact: true });
      await expect(tab).toHaveAttribute("hx-target", "#diag-panel");
      await expect(tab).toHaveAttribute("hx-swap", "innerHTML");
      await expect(tab).toHaveAttribute(
        "hx-get",
        new RegExp(`/ui/hts/diagnostics/panel\\?tab=${slug}`),
      );
      // Every tab also carries a real `href` so the nojs fallback
      // works — §7.10 row 7.9 "plain anchor tabs".
      await expect(tab).toHaveAttribute(
        "href",
        new RegExp(`/ui/hts/diagnostics\\?tab=${slug}`),
      );
    }
  });

  test.skip("forcing one panel to 5xx renders an OperationOutcome inside #diag-panel only", () => {
    // Skipped: the Playwright suite boots a real hts binary (see
    // `crates/hts-ui/e2e/boot.mjs`) and there is no way from the
    // browser to force `/metadata`, `/health`, or `/metrics` to
    // fail — HTS is its own upstream for those endpoints and is
    // guaranteed to be up while the suite is running. The outcome
    // contract is covered by the Rust integration test
    // `any_tab_5xx_renders_outcome_in_diag_panel_only` in
    // `crates/hts-ui/tests/diagnostics.rs`, which uses an in-process
    // axum mock to seed a 500 on `/health` and asserts that the
    // shared `hts-outcome.html` partial renders inside `#diag-panel`
    // while the three other tab id markers survive in the shell.
  });
});
