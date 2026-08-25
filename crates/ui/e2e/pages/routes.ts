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
  "/ui/bulk-export/active",
  "/ui/subscriptions",
];

// The bulk-import detail page only exists with a submission behind it. Seed
// one through the same form the page uses and hand back its route; callers
// append it to ROUTES so the detail layout is guarded too.
export async function seedBulkImportDetail(request: APIRequestContext): Promise<string> {
  const res = await request.post("/ui/bulk-import", {
    form: { name: "e2e-guard", recipient_base_url: "http://127.0.0.1:9/fhir" },
    maxRedirects: 0,
  });
  const location = res.headers()["location"];
  if (!location || !location.startsWith("/ui/bulk-import/")) {
    throw new Error(`seeding a bulk-import submission did not redirect to a detail page (got ${res.status()} ${location ?? "no Location"})`);
  }
  return location;
}
