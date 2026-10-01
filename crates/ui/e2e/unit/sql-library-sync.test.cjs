const test = require("node:test");
const assert = require("node:assert/strict");

const sync = require("../../assets/sql-library-sync.js");

// #1233: the pure helpers behind the live Details JSON <-> SQL card sync.
// `mount` itself needs `document`/DOM listeners and is covered by
// `tests/sql-libraries.spec.ts` instead; this ring only exercises the
// base64/JSON logic that must match `crates/ui/src/sql_libraries.rs`'s own
// `readable_sql`/`embed_sql`.

test("encodeSql/decodeSql round-trip plain ASCII", () => {
  const encoded = sync.encodeSql("SELECT 1");
  const decoded = sync.decodeSql(encoded);
  assert.equal(decoded.ok, true);
  assert.equal(decoded.text, "SELECT 1");
});

test("encodeSql/decodeSql round-trip text with non-ASCII and a BMP-adjacent glyph", () => {
  const text = "SELECT 'ñ' -- ✓";
  const decoded = sync.decodeSql(sync.encodeSql(text));
  assert.equal(decoded.ok, true);
  assert.equal(decoded.text, text);
});

test("decodeSql fails on a string that is not valid base64", () => {
  assert.deepEqual(sync.decodeSql("%%%"), { ok: false });
});

test("decodeSql fails on valid base64 whose bytes are not valid UTF-8", () => {
  // 0xff 0xfe is not a valid UTF-8 sequence on its own.
  const invalidUtf8 = Buffer.from([0xff, 0xfe]).toString("base64");
  assert.deepEqual(sync.decodeSql(invalidUtf8), { ok: false });
});

test("decodeSql fails on a non-string", () => {
  assert.deepEqual(sync.decodeSql(42), { ok: false });
  assert.deepEqual(sync.decodeSql(null), { ok: false });
  assert.deepEqual(sync.decodeSql(undefined), { ok: false });
});

test("sqlFromJson: invalid JSON", () => {
  assert.deepEqual(sync.sqlFromJson("{not json"), { state: "invalid-json" });
});

test("sqlFromJson: no content array at all", () => {
  assert.deepEqual(sync.sqlFromJson(JSON.stringify({ resourceType: "Library" })), {
    state: "none",
  });
});

test("sqlFromJson: content present but only text/plain", () => {
  const doc = { content: [{ contentType: "text/plain", data: sync.encodeSql("not sql") }] };
  assert.deepEqual(sync.sqlFromJson(JSON.stringify(doc)), { state: "none" });
});

test("sqlFromJson: application/sql attachment with no data", () => {
  const doc = { content: [{ contentType: "application/sql" }] };
  assert.deepEqual(sync.sqlFromJson(JSON.stringify(doc)), { state: "unreadable" });
});

test("sqlFromJson: application/sql attachment with invalid data", () => {
  const doc = { content: [{ contentType: "application/sql", data: "%%%" }] };
  assert.deepEqual(sync.sqlFromJson(JSON.stringify(doc)), { state: "unreadable" });
});

test("sqlFromJson: a valid application/sql attachment decodes", () => {
  const doc = { content: [{ contentType: "application/sql", data: sync.encodeSql("SELECT 1") }] };
  assert.deepEqual(sync.sqlFromJson(JSON.stringify(doc)), { state: "ok", sql: "SELECT 1" });
});

test("sqlFromJson: contentType prefix match, like the server's own readable_sql", () => {
  const doc = {
    content: [{ contentType: "application/sql; charset=utf-8", data: sync.encodeSql("SELECT 2") }],
  };
  assert.deepEqual(sync.sqlFromJson(JSON.stringify(doc)), { state: "ok", sql: "SELECT 2" });
});

test("jsonWithSql: patches the middle application/sql attachment and keeps every attachment's order", () => {
  const doc = {
    resourceType: "Library",
    content: [
      { contentType: "text/cql", data: "cql-data" },
      { contentType: "application/sql", data: sync.encodeSql("SELECT 1") },
      { contentType: "text/plain", data: "plain-data" },
    ],
  };
  const result = sync.jsonWithSql(JSON.stringify(doc), "SELECT 2");
  const parsed = JSON.parse(result);
  assert.equal(parsed.content.length, 3);
  assert.equal(parsed.content[0].contentType, "text/cql");
  assert.equal(parsed.content[0].data, "cql-data");
  assert.equal(parsed.content[1].contentType, "application/sql");
  assert.equal(parsed.content[1].data, sync.encodeSql("SELECT 2"));
  assert.equal(parsed.content[2].contentType, "text/plain");
  assert.equal(parsed.content[2].data, "plain-data");
});

test("jsonWithSql: appends an application/sql attachment when there is no readable one", () => {
  const doc = { content: [{ contentType: "text/plain", data: "plain-data" }] };
  const parsed = JSON.parse(sync.jsonWithSql(JSON.stringify(doc), "SELECT 3"));
  assert.equal(parsed.content.length, 2);
  assert.equal(parsed.content[1].contentType, "application/sql");
  assert.equal(parsed.content[1].data, sync.encodeSql("SELECT 3"));
});

test("jsonWithSql: creates content when missing entirely", () => {
  const doc = { resourceType: "Library" };
  const parsed = JSON.parse(sync.jsonWithSql(JSON.stringify(doc), "SELECT 4"));
  assert.deepEqual(parsed.content, [{ contentType: "application/sql", data: sync.encodeSql("SELECT 4") }]);
});

test("jsonWithSql: invalid JSON returns null", () => {
  assert.equal(sync.jsonWithSql("{not json", "SELECT 5"), null);
});

test("jsonWithSql: a non-object root returns null", () => {
  assert.equal(sync.jsonWithSql("[1, 2, 3]", "SELECT 6"), null);
  assert.equal(sync.jsonWithSql("null", "SELECT 6"), null);
  assert.equal(sync.jsonWithSql('"just a string"', "SELECT 6"), null);
});

test("jsonWithSql: a content key that exists but is not an array is left untouched (returns null)", () => {
  assert.equal(sync.jsonWithSql('{"content":{"a":1}}', "SELECT 1"), null);
});

test("jsonWithSql: the result is a 2-space pretty print whose re-parsed attachment decodes back to the same SQL", () => {
  const doc = { content: [{ contentType: "application/sql", data: sync.encodeSql("SELECT 1") }] };
  const result = sync.jsonWithSql(JSON.stringify(doc), "SELECT 'ñ'");
  assert.equal(result, JSON.stringify(JSON.parse(result), null, 2));
  const index = sync.findSqlAttachment(JSON.parse(result));
  assert.equal(JSON.parse(result).content[index].data, sync.encodeSql("SELECT 'ñ'"));
});

test("findSqlAttachment: -1 when content is not an array", () => {
  assert.equal(sync.findSqlAttachment({ content: "not an array" }), -1);
  assert.equal(sync.findSqlAttachment({}), -1);
  assert.equal(sync.findSqlAttachment(null), -1);
});
