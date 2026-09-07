// Seed helpers: create/update/read FHIR resources over the ordinary REST API so
// tests can set up state (versions for history, rows for the results table,
// tenants) without driving the UI. Uses Playwright's request context, which
// carries the suite baseURL.
import type { APIRequestContext } from "@playwright/test";

const FHIR_JSON = "application/fhir+json";

/** POST a resource; returns the server-assigned id. `tenant` scopes the write
 * via `X-Tenant-ID` (#553 — the default-tenant routes need no header). */
export async function createResource(
  request: APIRequestContext,
  type: string,
  body: Record<string, unknown>,
  tenant?: string,
): Promise<string> {
  const res = await request.post(`/${type}`, {
    headers: {
      "Content-Type": FHIR_JSON,
      Accept: FHIR_JSON,
      ...(tenant ? { "X-Tenant-ID": tenant } : {}),
    },
    data: { resourceType: type, ...body },
  });
  if (!res.ok()) throw new Error(`create ${type} -> ${res.status()}: ${await res.text()}`);
  return (await res.json()).id as string;
}

/** One resource to create via [`createResources`]. */
export type BatchEntry = { type: string; body: Record<string, unknown> };

/**
 * Create several resources in a single batch Bundle round trip, returning
 * their server-assigned ids in submission order. For tests that need many
 * resources (e.g. enough `$sql-export` subjects to keep a job genuinely
 * `in-progress` for a moment — a single tiny job finishes before the page
 * even finishes rendering), calling `createResource` in a loop is one HTTP
 * round trip per resource; this is one round trip total.
 */
export async function createResources(
  request: APIRequestContext,
  entries: BatchEntry[],
  tenant?: string,
): Promise<string[]> {
  const bundle = {
    resourceType: "Bundle",
    type: "batch",
    entry: entries.map(({ type, body }) => ({
      resource: { resourceType: type, ...body },
      request: { method: "POST", url: type },
    })),
  };
  const res = await request.post("/", {
    headers: {
      "Content-Type": FHIR_JSON,
      Accept: FHIR_JSON,
      ...(tenant ? { "X-Tenant-ID": tenant } : {}),
    },
    data: bundle,
  });
  if (!res.ok()) throw new Error(`batch create -> ${res.status()}: ${await res.text()}`);
  type BatchResponseEntry = { response?: { status?: string; location?: string } };
  const responseEntries = ((await res.json()).entry ?? []) as BatchResponseEntry[];
  return responseEntries.map((entry, index) => {
    const status = entry.response?.status ?? "";
    if (!status.startsWith("201")) {
      throw new Error(`batch entry ${index} (${entries[index]?.type}) failed: ${status}`);
    }
    // `Location: {Type}/{id}/_history/{version}`.
    const location = entry.response?.location ?? "";
    const id = location.split("/")[1];
    if (!id) {
      throw new Error(`batch entry ${index} had no usable Location: ${location}`);
    }
    return id;
  });
}

/**
 * Delete several resources of the same type in one batch Bundle round trip.
 * For specs that seed many resources as `$sql-export`/`$sql-run` padding
 * (e.g. enough `ViewDefinition`s to keep a job observably `in-progress` —
 * see [`createResources`]): those resources are real, tenant-visible FHIR
 * data, and the suite shares one server/database across every spec file
 * (`playwright.config.ts`: `fullyParallel: false`, `workers: 1`). Left
 * behind, they leak into any other page that lists that resource type
 * without a filter — e.g. `/ui/sql/view-definitions`, whose rail defaults
 * to the first `ViewDefinition` it finds and renders it, CodeMirror and
 * all. A spec that seeds resources it doesn't otherwise clean up through
 * the UI must delete them here once it's done. Missing ids (already
 * removed by the spec itself) are tolerated: a `404` batch entry is not an
 * error, only a genuine delete failure is.
 */
export async function deleteResources(
  request: APIRequestContext,
  type: string,
  ids: string[],
  tenant?: string,
): Promise<void> {
  // One request per DELETE_BATCH ids: a batch runs its entries at the
  // backend's write concurrency inside the server's 30s request timeout,
  // and on the ES composites (synchronous sync, refreshed writes) a 400-entry
  // padding cleanup ran past it — a 408 with half the entries still live.
  for (let start = 0; start < ids.length; start += DELETE_BATCH) {
    await deleteBatch(request, type, ids.slice(start, start + DELETE_BATCH), tenant);
  }
}

const DELETE_BATCH = 100;

async function deleteBatch(
  request: APIRequestContext,
  type: string,
  ids: string[],
  tenant?: string,
): Promise<void> {
  if (ids.length === 0) return;
  const bundle = {
    resourceType: "Bundle",
    type: "batch",
    entry: ids.map((id) => ({ request: { method: "DELETE", url: `${type}/${id}` } })),
  };
  const res = await request.post("/", {
    headers: {
      "Content-Type": FHIR_JSON,
      Accept: FHIR_JSON,
      ...(tenant ? { "X-Tenant-ID": tenant } : {}),
    },
    data: bundle,
  });
  if (!res.ok()) throw new Error(`batch delete ${type} -> ${res.status()}: ${await res.text()}`);
  type BatchResponseEntry = { response?: { status?: string } };
  const responseEntries = ((await res.json()).entry ?? []) as BatchResponseEntry[];
  responseEntries.forEach((entry, index) => {
    const status = entry.response?.status ?? "";
    if (!status.startsWith("200") && !status.startsWith("204") && !status.startsWith("404")) {
      throw new Error(`batch delete entry ${index} (${type}/${ids[index]}) failed: ${status}`);
    }
  });
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

/**
 * Wait until a created resource is *searchable*, not merely readable. On the
 * SQLite/PostgreSQL/MongoDB backends search is read-your-write and the first
 * probe returns immediately; on the Elasticsearch composites a write only
 * becomes searchable after the index's refresh tick (~1s), so a spec that
 * creates and then immediately searches — through the UI or the API — must
 * wait here first or it races the refresh (nightly ui-tests-matrix, ES legs).
 */
export async function waitSearchable(
  request: APIRequestContext,
  type: string,
  id: string,
  tenant?: string,
  timeoutMs = 15_000,
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const res = await request.get(`/${type}?_id=${id}&_summary=count`, {
      headers: { Accept: FHIR_JSON, ...(tenant ? { "X-Tenant-ID": tenant } : {}) },
    });
    if (res.ok() && (((await res.json()).total as number) ?? 0) >= 1) return;
    if (Date.now() > deadline) {
      throw new Error(`${type}/${id} still not searchable after ${timeoutMs}ms`);
    }
    await new Promise((r) => setTimeout(r, 250));
  }
}

/**
 * Seeds a `Library` sql-query subject depending on `canonical` (an
 * already-created ViewDefinition's own `url`), aliased "v" in its SQL — the
 * same shape `sql-libraries.spec.ts` uses for a genuinely runnable SQLQuery
 * Library. `sql` defaults to a query that always succeeds; the chromium and
 * `nojs` SQL Export job-detail specs both pass a deliberately broken one to
 * seed a `failed` job.
 *
 * `parameters` (#837) is the Library's own `parameter` array, verbatim —
 * each entry the exact FHIR shape `Library.parameter[use=in]` takes (e.g.
 * `{ name: "ward", use: "in", type: "string" }`, or with a
 * `defaultString`/`defaultInteger`/… for an optional one) — omitted
 * entirely from the created resource when left undefined, exactly like an
 * unparameterized SQL Query.
 */
export async function createSqlQueryLibrary(
  request: APIRequestContext,
  name: string,
  canonical: string,
  sql = "SELECT COUNT(*) AS n FROM v",
  parameters?: Record<string, unknown>[],
): Promise<string> {
  return createResource(request, "Library", {
    name,
    status: "active",
    type: {
      coding: [
        {
          system: "http://hl7.org/fhir/uv/sql-on-fhir/CodeSystem/LibraryTypesCodes",
          code: "sql-query",
        },
      ],
    },
    relatedArtifact: [{ type: "depends-on", resource: canonical, label: "v" }],
    content: [{ contentType: "application/sql", data: Buffer.from(sql).toString("base64") }],
    ...(parameters ? { parameter: parameters } : {}),
  });
}
