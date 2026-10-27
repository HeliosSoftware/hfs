const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

// #821: the vendored CodeMirror bundle (crates/ui/vendor/codemirror/, see its
// README for the regeneration ritual) is a minified IIFE with no
// `import`/`export`. The only way to assert its exported surface hasn't
// regressed is to actually execute it — the way the browser does — and
// inspect the resulting global. A Node `vm` context with a synthetic
// `window` stands in for the browser: `src/entry.js`'s header comment
// explains why the bundle reads `window`/`globalThis` as free variables
// instead of relying on Rollup's dotted iife name, which is exactly what
// lets this same source run unmodified under `vm`.

const bundlePath = path.join(__dirname, "../../assets/vendor/codemirror.bundle.js");
const source = fs.readFileSync(bundlePath, "utf8");

function loadHfsCodeMirror() {
  const sandbox = { window: {} };
  const context = vm.createContext(sandbox);
  vm.runInContext(source, context, { filename: "codemirror.bundle.js" });
  return sandbox.window.HfsCodeMirror;
}

test("defines exactly one global and no other property on window", () => {
  const sandbox = { window: {} };
  const context = vm.createContext(sandbox);
  vm.runInContext(source, context, { filename: "codemirror.bundle.js" });
  assert.deepEqual(Object.keys(sandbox.window), ["HfsCodeMirror"]);
});

test("contains no eval, new Function, or document.write", () => {
  assert.doesNotMatch(source, /\beval\s*\(/);
  assert.doesNotMatch(source, /new\s+Function\s*\(/);
  assert.doesNotMatch(source, /document\.write\s*\(/);
});

test("banner cites the MIT license for lezer-fhirpath and drops the old placeholder", () => {
  assert.match(source, /lezer-fhirpath@1\.2\.0 \(MIT \(declared in the package README, not in package\.json\)\)/);
  assert.doesNotMatch(source, /not declared in package metadata/);
});

// Every name @codemirror/autocomplete, @codemirror/lint, and
// @codemirror/view gained in this bundle regeneration, plus a representative
// sample of names already exported before it (a regression here would mean
// an entry.js edit silently dropped one of them).
const NEW_AUTOCOMPLETE_EXPORTS = [
  "snippetCompletion",
  "snippet",
  "startCompletion",
  "closeCompletion",
  "acceptCompletion",
  "completionStatus",
  "currentCompletions",
  "insertCompletionText",
];
const NEW_LINT_EXPORTS = [
  "forEachDiagnostic",
  "openLintPanel",
  "closeLintPanel",
  "nextDiagnostic",
  "previousDiagnostic",
  "diagnosticCount",
];
const NEW_VIEW_EXPORTS = ["hoverTooltip", "showTooltip"];
const PREEXISTING_EXPORTS = [
  "EditorState",
  "EditorView",
  "autocompletion",
  "linter",
  "lintGutter",
  "json",
  "sql",
  "fhirpath",
  "version",
];

test("every newly added export resolves to a defined function or value", () => {
  const HfsCodeMirror = loadHfsCodeMirror();
  for (const name of [...NEW_AUTOCOMPLETE_EXPORTS, ...NEW_LINT_EXPORTS, ...NEW_VIEW_EXPORTS]) {
    assert.notEqual(
      typeof HfsCodeMirror[name],
      "undefined",
      `HfsCodeMirror.${name} must be defined`,
    );
  }
});

test("previously existing exports are unaffected by the new entry.js imports", () => {
  const HfsCodeMirror = loadHfsCodeMirror();
  for (const name of PREEXISTING_EXPORTS) {
    assert.notEqual(
      typeof HfsCodeMirror[name],
      "undefined",
      `HfsCodeMirror.${name} must still be defined`,
    );
  }
  assert.deepEqual(Object.keys(HfsCodeMirror.fhirpath).sort(), ["parser", "props", "terminals"]);
});
