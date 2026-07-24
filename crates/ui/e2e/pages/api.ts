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
