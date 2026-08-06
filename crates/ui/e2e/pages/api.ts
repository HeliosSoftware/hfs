// Seed helpers: create/update/read FHIR resources over the ordinary REST API so
// tests can set up state (versions for history, rows for the results table,
// tenants) without driving the UI. Uses Playwright's request context, which
// carries the suite baseURL.
import type { APIRequestContext } from "@playwright/test";

const FHIR_JSON = "application/fhir+json";

/** POST a resource; returns the server-assigned id. */
export async function createResource(
  request: APIRequestContext,
  type: string,
  body: Record<string, unknown>,
): Promise<string> {
  const res = await request.post(`/${type}`, {
    headers: { "Content-Type": FHIR_JSON, Accept: FHIR_JSON },
    data: { resourceType: type, ...body },
  });
  if (!res.ok()) throw new Error(`create ${type} -> ${res.status()}: ${await res.text()}`);
  return (await res.json()).id as string;
}

/** PUT a resource, minting a new version. */
export async function updateResource(
  request: APIRequestContext,
  type: string,
  id: string,
  body: Record<string, unknown>,
): Promise<void> {
  const res = await request.put(`/${type}/${id}`, {
    headers: { "Content-Type": FHIR_JSON, Accept: FHIR_JSON },
    data: { resourceType: type, id, ...body },
  });
  if (!res.ok()) throw new Error(`update ${type}/${id} -> ${res.status()}: ${await res.text()}`);
}

/** Read a resource back as parsed JSON. */
export async function readResource(
  request: APIRequestContext,
  type: string,
  id: string,
): Promise<Record<string, unknown>> {
  const res = await request.get(`/${type}/${id}`, { headers: { Accept: FHIR_JSON } });
  if (!res.ok()) throw new Error(`read ${type}/${id} -> ${res.status()}`);
  return res.json();
}

/**
 * Wait until a just-created resource is visible to FHIR search.
 *
 * On composite backends (sqlite/s3 + Elasticsearch) create returns as soon as
 * the write store acknowledges; the search index can lag a few hundred ms.
 * Nightly ui-tests-matrix flakes when the next step searches immediately.
 */
export async function waitForSearchHit(
  request: APIRequestContext,
  type: string,
  query: string,
  opts: { timeoutMs?: number } = {},
): Promise<void> {
  const timeoutMs = opts.timeoutMs ?? 15_000;
  const deadline = Date.now() + timeoutMs;
  let lastStatus = 0;
  let lastTotal = "n/a";
  while (Date.now() < deadline) {
    const res = await request.get(`/${type}?${query}`, {
      headers: { Accept: FHIR_JSON },
    });
    lastStatus = res.status();
    if (res.ok()) {
      const bundle = await res.json();
      const total =
        typeof bundle.total === "number"
          ? bundle.total
          : Array.isArray(bundle.entry)
            ? bundle.entry.length
            : 0;
      lastTotal = String(total);
      if (total > 0) return;
    }
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error(
    `timed out waiting for ${type}?${query} (last status=${lastStatus}, total=${lastTotal})`,
  );
}

/**
 * Create a resource and immediately update it, leaving two versions — the
 * minimum a history diff needs. Returns the id. `mutate` produces the second
 * version's body from the first.
 */
export async function seedTwoVersions(
  request: APIRequestContext,
  type: string,
  first: Record<string, unknown>,
  mutate: (first: Record<string, unknown>) => Record<string, unknown>,
): Promise<string> {
  const id = await createResource(request, type, first);
  await updateResource(request, type, id, mutate(first));
  return id;
}
