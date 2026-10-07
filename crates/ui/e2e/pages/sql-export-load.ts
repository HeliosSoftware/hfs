// Shared by the chromium and `nojs` SQL Export lifecycle specs: a `$sql-export`
// job that stays observably `in-progress` while one request carries no more
// `subject` entries than the server accepts.
//
// A single trivial subject finishes before the redirect to the list even
// renders. Those specs used to pad the job with 200 trivial ViewDefinitions;
// since #1705 `$sql-export` rejects a request over 64 `subject` entries, so
// that job failed at once. The run time now comes from the work each subject
// does instead: every ViewDefinition evaluates `LOAD_COLUMNS` columns over
// `LOAD_PATIENTS` Patients, one subject after another.
import type { APIRequestContext } from "@playwright/test";
import { createResources, deleteResources } from "./api";

/** Most `subject` entries one `$sql-export` request may carry (#1705,
 * `MAX_EXPORT_SUBJECTS` in crates/rest/src/handlers/sof/input_limits.rs). */
export const EXPORT_SUBJECT_CAP = 64;

/** Subjects a lifecycle job carries: the most the server accepts. */
export const LOAD_SUBJECTS = EXPORT_SUBJECT_CAP;

/** Patients each load subject is evaluated over. */
export const LOAD_PATIENTS = 1_000;

// Simple paths the in-DB runner supports, cycled to fill `LOAD_COLUMNS`
// distinct columns (a function it lacks, e.g. `upper()`, fails the job).
const LOAD_PATHS = [
  "name.family.first()",
  "name.given.first()",
  "address.city.first()",
  "address.line.first()",
  "identifier.where(system='x').value.first()",
  "telecom.where(system='phone').value.first()",
  "name.exists()",
  "address.exists()",
  "name.given.count()",
  "getResourceKey()",
];

/** Columns per load ViewDefinition; with `LOAD_PATIENTS` and `LOAD_SUBJECTS`
 * this keeps the job running for several seconds on a quiet machine. */
export const LOAD_COLUMNS = 40;

/** The body of one load ViewDefinition named `name`. */
export function loadViewDefinition(name: string): Record<string, unknown> {
  return {
    name,
    status: "active",
    resource: "Patient",
    select: [
      {
        column: Array.from({ length: LOAD_COLUMNS }, (_, i) => ({
          name: `c${i}`,
          path: LOAD_PATHS[i % LOAD_PATHS.length],
        })),
      },
    ],
  };
}

/**
 * Seeds `LOAD_PATIENTS` Patients whose family names start with `prefix`,
 * reporting each chunk's ids to `onChunk` as it lands (for incremental
 * cleanup). Returns the last created id so the caller can wait for it to be
 * searchable.
 */
export async function seedLoadPatients(
  request: APIRequestContext,
  prefix: string,
  onChunk: (ids: string[]) => void,
): Promise<string> {
  const ids = await createResources(
    request,
    Array.from({ length: LOAD_PATIENTS }, (_, i) => ({
      type: "Patient",
      body: {
        name: [{ family: `${prefix}${i}`, given: ["a", "b"] }],
        identifier: [{ system: "x", value: String(i) }],
        address: [{ city: "c", line: ["l"] }],
        telecom: [{ system: "phone", value: "1" }],
      },
    })),
    undefined,
    onChunk,
  );
  return ids[ids.length - 1];
}

/**
 * Deletes every Patient whose family name starts with `prefix`: the backstop
 * for a `seedLoadPatients` chunk whose request failed outright, so its ids
 * were never reported (the server may still have committed it, #1070).
 * Re-searches after each delete round, since the matching set shrinks.
 */
export async function deleteLoadPatients(
  request: APIRequestContext,
  prefix: string,
): Promise<void> {
  for (let round = 0; round < 100; round++) {
    const res = await request.get(`/Patient?family=${encodeURIComponent(prefix)}&_count=100&_elements=id`, {
      headers: { Accept: "application/fhir+json" },
    });
    if (!res.ok()) throw new Error(`search Patient -> ${res.status()}: ${await res.text()}`);
    const ids = (((await res.json()).entry ?? []) as { resource?: { id?: string } }[])
      .map((entry) => entry.resource?.id)
      .filter((id): id is string => Boolean(id));
    if (ids.length === 0) return;
    await deleteResources(request, "Patient", ids);
  }
}
