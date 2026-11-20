// Hand-written (not generated from the catalog; a unit test checks the names it uses).
// Usage: mongosh "$HFS_MONGODB_URL" --file <this file>, with HFS_MONGODB_DATABASE exported
// Rolls search_index generation 8 back to 7 (quantity intervals, #2602):
//   1. drops idx_search_quantity_high ("index not found" is ignored);
//   2. lowers a recorded search_indexes.generation of 8 to 7, so a generation-8
//      build that boots again records 8 anew and rebuilds the index.
// Leaves value_quantity_high on the rows (an older build ignores it). A Range
// row indexed by generation 8 stores its low in value_quantity_value, which an
// older build compares as a plain point: run `$reindex` after rolling forward
// again only if you want to be sure no stale Range row remains.
if (!process.env.HFS_MONGODB_DATABASE) {
  throw new Error("HFS_MONGODB_DATABASE is not set: export the name of the database HFS uses (default helios)");
}
db = db.getSiblingDB(process.env.HFS_MONGODB_DATABASE);
try {
  printjson(db.search_index.dropIndex("idx_search_quantity_high"));
} catch (e) {
  if (e.codeName !== "IndexNotFound" && e.codeName !== "NamespaceNotFound") {
    throw e;
  }
  print("idx_search_quantity_high: " + e.codeName + " (nothing to drop)");
}
db.schema_version.updateOne(
  { _id: "schema_version", "search_indexes.generation": 8 },
  { $set: { "search_indexes.generation": NumberInt(7) } }
);
printjson(db.schema_version.findOne({ _id: "schema_version" }));
