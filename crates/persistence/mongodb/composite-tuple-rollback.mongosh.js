// Hand-written (not generated from the catalog; a unit test checks the names it uses).
// Usage: mongosh "$HFS_MONGODB_URL" --file <this file>, with HFS_MONGODB_DATABASE exported
// Rolls search_index generation 6 back to 5 (token+quantity tuple_code, cq-p3):
//   1. deletes every tuple_code marker (search_layout, layout "composite_tuple_code_v2"),
//      first, so no instance of a generation-6 build can trust one afterwards;
//   2. drops idx_search_composite_tuple_qty ("index not found" is ignored);
//   3. lowers a recorded search_indexes.generation of 6 to 5, so a generation-6
//      build that boots again records 6 anew (deleting markers again) and rebuilds the index.
// Leaves tuple_code on the rows (an older build ignores it) and does not change `version`
// or the #2394 reference records. Tenants need the activating `$reindex` again after a
// return to generation 6.
if (!process.env.HFS_MONGODB_DATABASE) {
  throw new Error("HFS_MONGODB_DATABASE is not set: export the name of the database HFS uses (default helios)");
}
db = db.getSiblingDB(process.env.HFS_MONGODB_DATABASE);
printjson(db.search_layout.deleteMany({ layout: "composite_tuple_code_v2" }));
try {
  printjson(db.search_index.dropIndex("idx_search_composite_tuple_qty"));
} catch (e) {
  if (e.codeName !== "IndexNotFound" && e.codeName !== "NamespaceNotFound") {
    throw e;
  }
  print("idx_search_composite_tuple_qty: " + e.codeName + " (nothing to drop)");
}
db.schema_version.updateOne(
  { _id: "schema_version", "search_indexes.generation": 6 },
  { $set: { "search_indexes.generation": NumberInt(5) } }
);
printjson(db.schema_version.findOne({ _id: "schema_version" }));
