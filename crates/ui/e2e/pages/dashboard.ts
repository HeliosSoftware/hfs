// Landing dashboard (/ui): stat cards, the resources-over-time chart, the
// time-window selector, the type picker, and the per-series legend.
import type { Page, Locator, Response } from "@playwright/test";

/** A `data-dash-notice` slug (#956, #1078) — see `DashboardNotice` in
 * crates/ui/src/lib.rs. `live` and `approximate` are real readings; `pending`
 * (nothing known, cards blank) and `series-pending` (cards known, only the
 * chart waits) are the waiting states; `partial` and `sample` flag figures
 * that are filled in or invented. */
export type DashNotice =
  | "live"
  | "pending"
  | "series-pending"
  | "partial"
  | "approximate"
  | "sample";

/** What the server sent for one hard navigation, read from the response body
 * itself rather than the live DOM. `#dash-live` re-requests itself through
 * htmx — a waiting page after a short delay, a ready page with approximate
 * figures on a periodic poll — so by the time the DOM is inspected the first
 * render may already have been swapped for a newer one; the body cannot have
 * been. */
export interface FirstRender {
  /** `data-dash-notice` slugs, in document order. */
  notices: string[];
  /** Plotted `polyline.series` in the chart. */
  series: number;
  /** Whether the chart area rendered `.chart-empty` instead of a chart. */
  chartEmpty: boolean;
  /** Whether `#dash-live` carried the bounded htmx auto-retry of a waiting
   * page (`hx-get` without `data-dash-refresh`). */
  autoRetry: boolean;
  /** Whether `#dash-live` carried the periodic self-refresh of a ready page
   * whose figures are approximate or an import is running (#1078,
   * `data-dash-refresh`, every 5s). Never set together with {@link autoRetry}. */
  liveRefresh: boolean;
  /** Stat-grid values rendered as the unavailable "—". */
  unavailableCards: number;
}

export class DashboardPage {
  constructor(readonly page: Page) {}

  async goto(query = ""): Promise<Response | null> {
    return this.page.goto(`/ui${query}`, { waitUntil: "networkidle" });
  }

  /** Navigates to `/ui{query}` and reports what the server's own response
   * rendered (see {@link FirstRender}) — no reload, no retry. */
  async gotoFirstRender(query = ""): Promise<FirstRender> {
    const response = await this.goto(query);
    if (!response) throw new Error(`no response for /ui${query}`);
    if (!response.ok()) throw new Error(`/ui${query} -> ${response.status()}`);
    return parseFirstRender(await response.text());
  }

  get statCards(): Locator {
    return this.page.locator(".card.stat");
  }
  /** The headline cards' values (the stat grid only — the chart card's own
   * total is not a headline card). */
  get statValues(): Locator {
    return this.page.locator(".stat-grid .stat__value");
  }
  /** Headline values rendered as "—": no figure is known for that card. */
  get unavailableStatValues(): Locator {
    return this.page.locator(".stat-grid .stat__value--unavailable");
  }
  /** Whether every headline card shows a figure rather than "—". */
  async cardsShowFigures(): Promise<boolean> {
    const values = await this.statValues.count();
    return values > 0 && (await this.unavailableStatValues.count()) === 0;
  }
  /** Every notice line the dashboard renders (`p.notice[data-dash-notice]`). */
  get notices(): Locator {
    return this.page.locator("[data-dash-notice]");
  }
  notice(kind: DashNotice): Locator {
    return this.page.locator(`[data-dash-notice="${kind}"]`);
  }
  /** The notice slugs currently on the page, in document order. */
  async noticeKinds(): Promise<string[]> {
    return this.notices.evaluateAll((lines) =>
      lines.map((line) => line.getAttribute("data-dash-notice") ?? ""),
    );
  }
  /** The "As of HH:MM:SS UTC" stamp; it rides on the first notice line of a
   * live render (#1078). */
  get asOfTime(): Locator {
    return this.page.locator("[data-dash-notice] time[datetime]");
  }
  /** The chart area's placeholder: waiting for this window's series, or no
   * data to chart at all. */
  get chartWaiting(): Locator {
    return this.page.locator(".chart-card .chart-empty");
  }
  /** The swappable region every snapshot-derived figure lives in (#956). */
  get live(): Locator {
    return this.page.locator("#dash-live");
  }
  /** `#dash-live` while a waiting page still has its bounded htmx auto-retry
   * scheduled. A ready page's periodic self-refresh also rides on `hx-get`
   * but carries `data-dash-refresh`, so it never matches here. */
  get pendingAutoRetry(): Locator {
    return this.page.locator("#dash-live[hx-get]:not([data-dash-refresh])");
  }
  /** `#dash-live` while a ready page polls itself every 5s because its
   * figures are approximate or an import is running (#1078). The tick only
   * stands down while the tab is hidden, a picker fetch is in flight, or
   * keyboard focus sits inside the region outside the type picker; an open
   * picker or data table, the tooltip and a mouse click do not stop it. */
  get liveRefresh(): Locator {
    return this.page.locator("#dash-live[data-dash-refresh]");
  }
  /** Marks the `#dash-live` node on screen. A refresh swaps the region's
   * outerHTML, so the mark is gone once one has landed — see
   * {@link unrefreshedLive}. */
  async markLive(): Promise<void> {
    await this.live.evaluate((el) => el.setAttribute("data-e2e-before-refresh", ""));
  }
  /** The `#dash-live` node {@link markLive} marked, while it is still on
   * screen: a count of 0 means a refresh has replaced it. */
  get unrefreshedLive(): Locator {
    return this.page.locator("#dash-live[data-e2e-before-refresh]");
  }
  /** The "Stored Resources" headline value. Compact ("1.4k") past 999, so
   * compare exact counts through {@link legendTotal} or {@link chartTotal}. */
  get storedResourcesValue(): Locator {
    return this.statCards.filter({ hasText: "Stored Resources" }).locator(".stat__value");
  }
  /** The chart card's headline total, thousands-separated and exact. */
  get chartTotal(): Locator {
    return this.page.locator(".chart-card__head .stat__value");
  }
  /** The legend entry's exact count for `type`, or `null` when `type` has no
   * legend entry (or no parsable count) right now. Reads once, no waiting. */
  async legendTotal(type: string): Promise<number | null> {
    const totals = await this.legendItems
      .filter({ has: this.page.locator("span", { hasText: new RegExp(`^${type}$`) }) })
      .locator(".chart-legend__total")
      .allTextContents();
    if (totals.length !== 1) return null;
    const value = Number(totals[0].replace(/,/g, "").trim());
    return Number.isFinite(value) ? value : null;
  }
  /** The first notice's "as of" `datetime`, or `null` when there is none.
   * Reads once, no waiting. */
  async asOfDatetime(): Promise<string | null> {
    const stamps = await this.asOfTime.first().evaluateAll((els) =>
      els.map((el) => el.getAttribute("datetime")),
    );
    return stamps[0] ?? null;
  }
  /** The "Resource Types" card; its `.stat__sub` names the effective FHIR
   * version ("used for R4", #553). */
  get resourceTypesCard(): Locator {
    return this.page.locator(".card.stat", { hasText: "Resource Types" });
  }
  get exportJobsCard(): Locator {
    return this.page.locator("a.card.stat", { hasText: "Export Jobs" });
  }
  get importJobsCard(): Locator {
    return this.page.locator("a.card.stat", { hasText: "Import Jobs" });
  }
  get chart(): Locator {
    return this.page.locator("svg.chart");
  }
  get seriesLines(): Locator {
    return this.page.locator("svg.chart polyline.series");
  }
  windowOption(label: RegExp | string): Locator {
    return this.page.locator(".window-picker__option", { hasText: label });
  }
  get legendItems(): Locator {
    return this.page.locator(".chart-legend__item");
  }
  /** The type picker, `<details class="menu chart-pick" id="chart-pick">`.
   * While it is open a refresh keeps this very node (#1078). */
  get picker(): Locator {
    return this.page.locator("details.chart-pick");
  }
  async openPicker(): Promise<void> {
    if ((await this.picker.getAttribute("open")) === null) {
      await this.picker.locator("summary").click();
    }
  }
  pickerOption(type: string): Locator {
    return this.page.locator(`[data-pick-name="${type}"]`);
  }
  get pickerFilter(): Locator {
    return this.page.locator("[data-pick-filter]");
  }
  /** "View all resources" (#599): offers every type of the active FHIR
   * version, not just the ones the tenant stores. */
  get viewAllToggle(): Locator {
    return this.page.locator(".chart-pick__option--all");
  }
  get tooltip(): Locator {
    return this.page.locator("#chart-tip");
  }
  /** The chart's tabular alternative, `<details class="chart-table"
   * id="chart-table">`; an open one stays open across a refresh (#1078). */
  get dataTable(): Locator {
    return this.page.locator("details.chart-table");
  }
  get dataTableToggle(): Locator {
    return this.page.locator(".chart-table > summary");
  }

  /** Reloads until the chart shows at least one plotted series.
   *
   * Written to outlast the 15s snapshot cache after seeding, back when every
   * snapshot was a storage aggregate. Since #1078 a seeded tenant is served
   * from the write counters: a cold window or selection charts on its first
   * view, and a warm key serves its cached snapshot (which already plots
   * series) while it refreshes, so on the SQLite leg this normally returns on
   * the first iteration. It stays for the older tests whose subject is not
   * load timing and that run against slower backends in the matrix. New tests
   * must not use it (or any reload loop) to paper over a waiting state —
   * assert on the first render instead (`gotoFirstRender`). */
  async waitForSeries(): Promise<void> {
    for (let attempt = 0; attempt < 12; attempt++) {
      if ((await this.seriesLines.count()) > 0) return;
      await this.page.waitForTimeout(2000);
      await this.page.reload({ waitUntil: "networkidle" });
    }
    throw new Error("no chart series appeared after seeding");
  }

  /** Reloads until the type picker offers `type`, leaving the picker open.
   *
   * `waitForSeries` is not enough for anything that asserts on *which* types
   * are offered: the snapshot cache serves the last computed snapshot while
   * it refreshes in the background, and a stale snapshot already plots
   * series, so the wait returns on the first load with the option list still
   * showing the types of a minute ago. A type seeded moments earlier only
   * appears once a refresh has landed. */
  async waitForPickerOption(type: string): Promise<void> {
    for (let attempt = 0; attempt < 12; attempt++) {
      await this.openPicker();
      if ((await this.pickerOption(type).count()) > 0) return;
      await this.page.waitForTimeout(2000);
      await this.page.reload({ waitUntil: "networkidle" });
    }
    throw new Error(`the type picker never offered ${type}`);
  }
}

/** Reads a dashboard response body into a {@link FirstRender}. The markup
 * hooks are the template's own (crates/ui/templates/pages/index.html). */
export function parseFirstRender(html: string): FirstRender {
  const statGrid = /<section class="stat-grid[^"]*">([\s\S]*?)<\/section>/.exec(html)?.[1] ?? "";
  const liveOpen = /<div id="dash-live"[^>]*>/.exec(html)?.[0] ?? "";
  // Both htmx polls ride on `hx-get`; only the ready page's periodic refresh
  // (#1078) marks itself with `data-dash-refresh`.
  const polls = liveOpen.includes("hx-get=");
  const liveRefresh = polls && liveOpen.includes("data-dash-refresh");
  return {
    notices: [...html.matchAll(/data-dash-notice="([^"]*)"/g)].map((m) => m[1]),
    series: (html.match(/<polyline class="series series--/g) ?? []).length,
    chartEmpty: html.includes('class="chart-empty"'),
    autoRetry: polls && !liveRefresh,
    liveRefresh,
    unavailableCards: (statGrid.match(/stat__value--unavailable/g) ?? []).length,
  };
}
