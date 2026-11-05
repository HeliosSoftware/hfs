import type { APIRequestContext } from "@playwright/test";

/** Add one saved entry without replacing another test's or user's queries. */
export async function seedSavedQuery(
  request: APIRequestContext,
  type: string,
  name: string,
  query: string,
) {
  const id = `issue1772-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
  const entry = { name, query, createdAt: new Date().toISOString() };
  const response = await request.patch("/_user/settings", {
    data: { savedQueries: { [type]: { [id]: entry } } },
  });
  if (response.status() === 501) return null;
  if (!response.ok()) throw new Error(`seed saved query -> ${response.status()}: ${await response.text()}`);
  return {
    id,
    entry,
    async remove() {
      const removed = await request.patch("/_user/settings", {
        data: { savedQueries: { [type]: { [id]: null } } },
      });
      if (!removed.ok()) throw new Error(`remove saved query -> ${removed.status()}`);
    },
  };
}
