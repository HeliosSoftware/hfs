// Hand-written (not generated from the catalog; no unit test compares it).
// Usage: mongosh "$HFS_MONGODB_URL/$HFS_MONGODB_DATABASE" <this file>
// Clears the reference-normalization records (#2394) so the next start of a
// schema-17 build runs the backfill again from the first _id, and readers fall
// back to the legacy `_history` / absolute-URL regex arms until it finishes.
// Use it when a build older than schema 17 took writes after the marker was
// set (it can leave `Type/id/_history/N` rows), or when the marker is in doubt.
// Does not change `version`, `search_indexes.generation` or any search_index row:
// the backfill is idempotent, and rows that are already normalized are skipped.
// Normalized rows need no revert: an older build still matches exact `Type/id`.
db.schema_version.updateOne(
  { _id: "schema_version" },
  { $unset: { "search_indexes.references_normalized": "", "search_indexes.reference_normalize_last_id": "" } }
);
printjson(db.schema_version.findOne({ _id: "schema_version" }));
