// Hand-written (not generated from the catalog; no unit test compares it).
// Usage: mongosh "$HFS_MONGODB_URL" --file <this file>, with HFS_MONGODB_DATABASE exported
// Clears the reference-normalization records (#2394) so the next start of a
// schema-17 build runs the backfill again from the first _id, and readers fall
// back to the legacy `_history` / absolute-URL regex arms until it finishes.
// Use it when a build older than schema 17 took writes after the marker was
// set (it can leave `Type/id/_history/N` rows), or when the marker is in doubt.
// Does not change `version`, `search_indexes.generation` or any search_index row:
// the backfill is idempotent, and rows that are already normalized are skipped.
// Normalized rows need no revert: an older build still matches exact `Type/id`.
if (!process.env.HFS_MONGODB_DATABASE) {
  throw new Error("HFS_MONGODB_DATABASE is not set: export the name of the database HFS uses (default helios)");
}
db = db.getSiblingDB(process.env.HFS_MONGODB_DATABASE);
db.schema_version.updateOne(
  { _id: "schema_version" },
  { $unset: { "search_indexes.references_normalized": "", "search_indexes.reference_normalize_last_id": "" } }
);
printjson(db.schema_version.findOne({ _id: "schema_version" }));
