// Landing dashboard (/ui): stat cards, the resources-over-time chart, the
// time-window selector, the type picker, and the per-series legend.
import type { Page, Locator, Response } from "@playwright/test";

/** A `data-dash-notice` slug (#956, #1078) — see `DashboardNotice` in
 * crates/ui/src/lib.rs. A render carries exactly one. `live` and
 * `approximate` are real readings; `pending` (nothing known, cards blank) is
 * the waiting state; `sample` flags invented figures; `unsupported` is a
 * backend that cannot count at all. */
export type DashNotice = "live" | "pending" | "approximate" | "sample" | "unsupported";

/** Whether the suite drives a server it did not boot (the backend matrix sets
 * HFS_E2E_BASE_URL), which runs with production cadences unless told
 * otherwise. */
const externalServer = !!process.env.HFS_E2E_BASE_URL;

/** One dashboard knob in seconds: the exported variable when set, else the
 * value boot.mjs gives a server this suite boots, else the production default
 * an external server runs with. */
function knobSecs(name: string, booted: number, production: number): number {
  const value = Number(process.env[name]);
  if (Number.isFinite(value) && value > 0) return value;
  return externalServer ? production : booted;
}

/** The Home dashboard's server knobs (#1078), in seconds. **Must match
 * boot.mjs**, which sets them for every server it boots; an exported
 * variable of the same name overrides a value on both sides. Every dashboard
 * wait in this suite is sized from these, or read from the page itself
 * (`data-dash-refresh`), never from a hard-coded production cadence. */
export const DASH_KNOBS = {
  /** `HFS_DASHBOARD_RECONCILE_SECS`: the background reconcile interval that
   * turns approximate figures exact (production 30). */
  reconcileSecs: knobSecs("HFS_DASHBOARD_RECONCILE_SECS", 3, 30),
  /** `HFS_DASHBOARD_REFRESH_SECS`: the refresh cadence of a page whose figures
   * are moving (production 5). */
  movingSecs: knobSecs("HFS_DASHBOARD_REFRESH_SECS", 2, 5),
  /** `HFS_DASHBOARD_IDLE_REFRESH_SECS`: the watch cadence of a settled page
   * (production 10). */
  settledSecs: knobSecs("HFS_DASHBOARD_IDLE_REFRESH_SECS", 3, 10),
} as const;

/** Fixed server timings the knobs do not cover: a seeded tenant's snapshot is
 * cached for at most 2s, and a refresh waits up to 500ms for a fresh value. */
export const DASH_SNAPSHOT = { cacheTtlMs: 2_000, freshWaitMs: 500 } as const;

/** Headroom added to every dashboard budget for a machine busy running the
 * whole suite next to three servers: a tick answered late, a request dropped
 * while another is in flight (`hx-sync`), a slow reconcile pass. */
export const CI_SLACK_MS = 15_000;

/** How long new figures may take to land on an open page after a write: two
 * ticks of the slower (settled) cadence — the page may be moving or settled,
 * and a tick in flight during the write can still carry the old figures —
 * plus the snapshot cache's TTL and its wait for a fresh value. */
export const FIGURES_LAND_MS =
  2 * DASH_KNOBS.settledSecs * 1000 + DASH_SNAPSHOT.cacheTtlMs + DASH_SNAPSHOT.freshWaitMs + CI_SLACK_MS;

/** How long a quiet tenant may take to settle after its last write: a
 * reconcile pass that starts after the write makes the totals exact, a later
 * one may still have to re-seed the charted rings, and the page's moving tick
 * must then bring the exact figures in. */
export const SETTLE_MS =
  3 * DASH_KNOBS.reconcileSecs * 1000 +
  DASH_KNOBS.movingSecs * 1000 +
  DASH_SNAPSHOT.cacheTtlMs +
  DASH_SNAPSHOT.freshWaitMs +
  CI_SLACK_MS;

/** What the server sent for one hard navigation, read from the response body
 * itself rather than the live DOM. `#dash-live` re-requests itself through
 * htmx — a waiting page after a short delay, every ready page on a periodic
 * poll — so by the time the DOM is inspected the first render may already
 * have been swapped for a newer one; the body cannot have been. */
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
  /** The period, in seconds, of the self-refresh `#dash-live` carried
   * (#1078, `data-dash-refresh`), or `null` when it carried none. Every ready
   * page schedules one: {@link DASH_KNOBS}.movingSecs while the figures are
   * moving, {@link DASH_KNOBS}.settledSecs once they settle. Never set
   * together with {@link autoRetry}. */
  refreshSecs: number | null;
  /** Whether that refresh was marked `data-dash-moving`: the figures are
   * approximate or an import is running. Only ever set with
   * {@link refreshSecs}. */
  moving: boolean;
  /** The refresh's `data-dash-state` digest of the figures, or `null` when
   * the render carries no refresh. */
  state: string | null;
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
    return parseFirstRender(this.page, response);
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
   * scheduled. Every ready page's periodic self-refresh also rides on
   * `hx-get` but carries `data-dash-refresh`, so it never matches here. */
  get pendingAutoRetry(): Locator {
    return this.page.locator("#dash-live[hx-get]:not([data-dash-refresh])");
  }
  /** `#dash-live` on any ready page: it polls itself (#1078) — every
   * {@link DASH_KNOBS}.movingSecs while the figures are moving
   * ({@link movingRefresh}), every {@link DASH_KNOBS}.settledSecs once they
   * settle ({@link settledRefresh}); the period is on the page as
   * `data-dash-refresh` ({@link refreshSecsOnScreen}). The tick only stands down while the tab
   * is hidden or keyboard focus sits inside the region outside the type
   * picker (a picker request in flight aborts or drops it through `hx-sync`);
   * an open picker or data table, the tooltip and a mouse click do not stop
   * it. Its requests carry `HX-Target: dash-live`. */
  get liveRefresh(): Locator {
    return this.page.locator("#dash-live[data-dash-refresh]");
  }
  /** `#dash-live` while its figures are approximate or an import is running
   * (`data-dash-moving`, polling at the moving cadence). */
  get movingRefresh(): Locator {
    return this.page.locator("#dash-live[data-dash-moving]");
  }
  /** `#dash-live` on a ready page whose figures have settled: still polling
   * (at the settled cadence), so a later write reaches it, but the tick sends its
   * `data-dash-state` as `?state=` and the server answers `204` (nothing
   * swapped) while the figures are unchanged. */
  get settledRefresh(): Locator {
    return this.page.locator("#dash-live[data-dash-refresh]:not([data-dash-moving])");
  }
  /** The refresh period, in seconds, the `#dash-live` on screen renders
   * (`data-dash-refresh`) — the tick length to size a wait from — or `null`
   * when the region carries no periodic refresh. Reads once, no waiting. */
  async refreshSecsOnScreen(): Promise<number | null> {
    const values = await this.page
      .locator("#dash-live[data-dash-refresh]")
      .evaluateAll((els) => els.map((el) => el.getAttribute("data-dash-refresh")));
    const secs = Number(values[0] ?? "");
    return values.length === 1 && Number.isFinite(secs) && secs > 0 ? secs : null;
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
   * While it is open a refresh keeps this very node (#1078: the request says
   * `open=pick`, the server renders it `hx-preserve`). */
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
   * load timing and that run against slower backends in the matrix, and for a
   * freshly provisioned tenant, whose figures wait on a reconcile pass to seed
   * it — hence the reload budget in {@link reloadUntil}. New tests must not use
   * it (or any reload loop) to paper over a waiting state — assert on the
   * first render instead (`gotoFirstRender`). */
  async waitForSeries(): Promise<void> {
    await this.reloadUntil(async () => (await this.seriesLines.count()) > 0, "no chart series appeared after seeding");
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
    await this.reloadUntil(async () => {
      await this.openPicker();
      return (await this.pickerOption(type).count()) > 0;
    }, `the type picker never offered ${type}`);
  }

  /** Checks `done` on the page, reloading between checks, until it holds or
   * the budget is spent (then throws `failure`). Sized from the reconcile knob:
   * the budget covers two reconcile passes (one to seed or reconcile the
   * tenant, one more in case the first began just before the write) plus the
   * snapshot cache, and a reload waits a third of an interval — never less
   * than the cache's TTL, which is the soonest a reload can see anything new. */
  private async reloadUntil(done: () => Promise<boolean>, failure: string): Promise<void> {
    const reconcileMs = DASH_KNOBS.reconcileSecs * 1000;
    const pauseMs = Math.max(DASH_SNAPSHOT.cacheTtlMs, Math.ceil(reconcileMs / 3));
    const deadline =
      Date.now() + 2 * reconcileMs + DASH_SNAPSHOT.cacheTtlMs + DASH_SNAPSHOT.freshWaitMs + CI_SLACK_MS;
    for (;;) {
      if (await done()) return;
      if (Date.now() >= deadline) throw new Error(failure);
      await this.page.waitForTimeout(pauseMs);
      await this.page.reload({ waitUntil: "networkidle" });
    }
  }
}

/** Reads a dashboard response into a {@link FirstRender}: a full page, or a
 * `#dash-live` or `#dash-chart` fragment an htmx request got back (a chart
 * fragment has no stat grid and no `#dash-live`, so it reports no unavailable
 * cards and no refresh). The markup hooks are the template's own
 * (crates/ui/templates/pages/index.html).
 *
 * The body is parsed as HTML by the browser's `DOMParser`, in `page` — Node
 * has no DOM — and never touches the live document. Only a `200` is read: a
 * refresh answered `204` (figures unchanged) has no body, and anything else
 * is not a render, so both throw. */
export async function parseFirstRender(page: Page, response: Response): Promise<FirstRender> {
  const status = response.status();
  if (status !== 200) throw new Error(`${response.url()} -> ${status}: no dashboard render to read`);
  const html = await response.text();
  // A navigation that commits while the parse runs destroys the context it
  // runs in; the parse does not depend on the document, so run it again in
  // the new one.
  for (let attempt = 0; ; attempt++) {
    try {
      return await page.evaluate(readRender, html);
    } catch (error) {
      if (attempt >= 2 || !/context was destroyed|navigat/i.test(String(error))) throw error;
      await page.waitForLoadState("domcontentloaded");
    }
  }
}

/** Runs in the browser (see {@link parseFirstRender}); self-contained, since
 * Playwright ships only its source. */
function readRender(html: string): FirstRender {
  const doc = new DOMParser().parseFromString(html, "text/html");
  const live = doc.getElementById("dash-live");
  // Both htmx polls ride on `hx-get`; only the ready page's periodic refresh
  // (#1078) marks itself with `data-dash-refresh`.
  const polls = !!live?.hasAttribute("hx-get");
  const refreshAttr = polls ? live?.getAttribute("data-dash-refresh") : null;
  const refreshSecs = refreshAttr == null || refreshAttr === "" ? null : Number(refreshAttr);
  return {
    notices: Array.from(doc.querySelectorAll("[data-dash-notice]"), (el) => el.getAttribute("data-dash-notice") ?? ""),
    series: doc.querySelectorAll("svg polyline.series").length,
    chartEmpty: doc.querySelector(".chart-empty") !== null,
    autoRetry: polls && refreshSecs === null,
    refreshSecs,
    moving: refreshSecs !== null && !!live?.hasAttribute("data-dash-moving"),
    state: refreshSecs !== null ? (live?.getAttribute("data-dash-state") ?? null) : null,
    unavailableCards: doc.querySelectorAll(".stat-grid .stat__value--unavailable").length,
  };
}
