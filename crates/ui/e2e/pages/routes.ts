import type { APIRequestContext } from "@playwright/test";

// The single route list behind every cross-page guard (no-cdn, a11y,
// design-system). It exists because the guards used to carry private copies,
// so a new page was opted out of all of them by default — /ui/bulk-import
// shipped unguarded exactly that way (#543). A new full page belongs here,
// once, and every guard picks it up.
export const ROUTES = [
  "/ui",
  "/ui/resources",
  "/ui/batch",
  "/ui/capability-statement",
  "/ui/compartments",
  "/ui/search-parameters",
  "/ui/terminology",
  "/ui/queries",
  "/ui/history",
  "/ui/search",
  "/ui/tenants",
  "/ui/editor?type=Patient",
  "/ui/bulk-import",
  "/ui/bulk-export",
  "/ui/bulk-export/new",
  "/ui/subscriptions",
  "/ui/sql/view-definitions",
  // `?vd=new` (#843), not the bare route above: with no view selected the
  // page renders its "nothing selected" card and neither the editor nor the
  // guided-form grid exists at all, so the bare route never exercises either
  // one — the starter document is the cheapest selection that does, with no
  // seeding required.
  "/ui/sql/view-definitions?vd=new",
  "/ui/sql/queries",
  "/ui/sql/views",
  "/ui/sql/export",
  "/ui/sql/export/new",
  "/ui/sql/files",
];

// The bulk-import detail page only exists with a submission behind it. Seed
// one through the same form the page uses and hand back its route; callers
// append it to ROUTES so the detail layout is guarded too.
export async function seedBulkImportDetail(request: APIRequestContext): Promise<string> {
  const res = await request.post("/ui/bulk-import", {
    form: { name: "e2e-guard", manifest_url: "https://example.test/manifest.json" },
    maxRedirects: 0,
  });
  const location = res.headers()["location"];
  if (!location || !location.startsWith("/ui/bulk-import/")) {
    throw new Error(`seeding a bulk-import submission did not redirect to a detail page (got ${res.status()} ${location ?? "no Location"})`);
  }
  return location;
}
