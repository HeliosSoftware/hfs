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
 * itself rather than the live DOM. The waiting page re-requests `#dash-live`
 * through htmx after a delay, so by the time the DOM is inspected a first
 * render that was waiting may already have been swapped for a ready one; the
 * body cannot have been. */
export interface FirstRender {
  /** `data-dash-notice` slugs, in document order. */
  notices: string[];
  /** Plotted `polyline.series` in the chart. */
  series: number;
  /** Whether the chart area rendered `.chart-empty` instead of a chart. */
  chartEmpty: boolean;
  /** Whether `#dash-live` carried the htmx auto-retry (a waiting page). */
  autoRetry: boolean;
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
  /** `#dash-live` while it still has an htmx auto-retry scheduled. */
  get pendingAutoRetry(): Locator {
    return this.page.locator("#dash-live[hx-get]");
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
  get picker(): Locator {
    return this.page.locator(".chart-pick");
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
  return {
    notices: [...html.matchAll(/data-dash-notice="([^"]*)"/g)].map((m) => m[1]),
    series: (html.match(/<polyline class="series series--/g) ?? []).length,
    chartEmpty: html.includes('class="chart-empty"'),
    autoRetry: liveOpen.includes("hx-get="),
    unavailableCards: (statGrid.match(/stat__value--unavailable/g) ?? []).length,
  };
}
