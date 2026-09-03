// Entry point for the CodeMirror 6 vendoring ritual (see ../README.md).
//
// Rollup bundles this module — plus everything it imports — into a single
// IIFE. The module itself performs the one side effect that matters: an
// explicit `window.HfsCodeMirror = ...` assignment at the bottom (falling
// back to `globalThis` when `window` is not defined, e.g. a worker or a
// non-browser JS host). This is deliberately NOT done via Rollup's
// `output.name` — a dotted iife name (`window.HfsCodeMirror`) relies on
// Rollup emitting `this.window.HfsCodeMirror = ...`, and `this` at the top
// of a script is not reliably the global object outside a browser classic
// script (e.g. under Node's `vm` module with a synthetic `window` set on the
// sandbox, `this` is the sandbox's global, not `sandbox.window`). Reading
// `window`/`globalThis` as free variables resolves correctly in both cases.
//
// It is a flat namespace (RF3 of ticket 01): every named export below lives
// directly on `HfsCodeMirror`, except the FHIRPath grammar
// (`HfsCodeMirror.fhirpath`, since it is not part of CodeMirror's own
// surface) and `HfsCodeMirror.version` (generated from each package's
// `package.json`, see rollup.config.js).
//
// This module is never imported anywhere else — it exists only to be the
// rollup `input`. Consumers (tickets 02 and 03) use the global, not this file.

import {
  Compartment,
  EditorState,
  RangeSet,
  RangeSetBuilder,
  StateEffect,
  StateField,
} from '@codemirror/state';
import {
  Decoration,
  EditorView,
  ViewPlugin,
  drawSelection,
  highlightActiveLine,
  highlightActiveLineGutter,
  keymap,
  lineNumbers,
  placeholder,
} from '@codemirror/view';
import {
  HighlightStyle,
  LRLanguage,
  LanguageSupport,
  bracketMatching,
  defaultHighlightStyle,
  foldGutter,
  foldKeymap,
  indentOnInput,
  indentUnit,
  syntaxHighlighting,
  syntaxTree,
} from '@codemirror/language';
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands';
import {
  autocompletion,
  closeBrackets,
  closeBracketsKeymap,
  completionKeymap,
} from '@codemirror/autocomplete';
import { forceLinting, linter, lintGutter, lintKeymap, setDiagnostics } from '@codemirror/lint';
import { highlightSelectionMatches, searchKeymap } from '@codemirror/search';
import { json, jsonLanguage, jsonParseLinter } from '@codemirror/lang-json';
import {
  SQLDialect,
  SQLite,
  StandardSQL,
  keywordCompletionSource,
  schemaCompletionSource,
  sql,
} from '@codemirror/lang-sql';
import { parseMixed } from '@lezer/common';
import { classHighlighter, styleTags, tags } from '@lezer/highlight';
import { parser as fhirpathParser, props as fhirpathProps, terminals as fhirpathTerminals } from 'lezer-fhirpath';

// Resolved by the `hfs-codemirror-versions` Rollup plugin (rollup.config.js)
// from each bundled package's own `package.json` — not written by hand.
import version from 'virtual:hfs-codemirror-versions';

/**
 * The namespace this vendoring ritual exposes as `window.HfsCodeMirror`.
 *
 * Flat by design: tickets 02 and 03 destructure straight off the global
 * (`const { EditorState, EditorView } = window.HfsCodeMirror`) without a
 * per-package sub-namespace to remember. The one exception is `fhirpath`,
 * kept nested because it is not a CodeMirror export — it is the
 * `lezer-fhirpath` grammar injected into the JSON language via `parseMixed`.
 */
const HfsCodeMirror = {
  // @codemirror/state
  Compartment,
  EditorState,
  RangeSet,
  RangeSetBuilder,
  StateEffect,
  StateField,

  // @codemirror/view
  Decoration,
  EditorView,
  ViewPlugin,
  drawSelection,
  highlightActiveLine,
  highlightActiveLineGutter,
  keymap,
  lineNumbers,
  placeholder,

  // @codemirror/language
  HighlightStyle,
  LRLanguage,
  LanguageSupport,
  bracketMatching,
  defaultHighlightStyle,
  foldGutter,
  foldKeymap,
  indentOnInput,
  indentUnit,
  syntaxHighlighting,
  syntaxTree,

  // @codemirror/commands
  defaultKeymap,
  history,
  historyKeymap,
  indentWithTab,

  // @codemirror/autocomplete
  autocompletion,
  closeBrackets,
  closeBracketsKeymap,
  completionKeymap,

  // @codemirror/lint
  forceLinting,
  linter,
  lintGutter,
  lintKeymap,
  setDiagnostics,

  // @codemirror/search
  highlightSelectionMatches,
  searchKeymap,

  // @codemirror/lang-json
  json,
  jsonLanguage,
  jsonParseLinter,

  // @codemirror/lang-sql — only the SQLite dialect (the engine
  // `$sqlquery-run` executes on) the SQL pane editor mounts, plus what a
  // future dialect swap or schema-aware autocomplete (#839/#842) needs; the
  // package's other built-in dialects (MySQL, PostgreSQL, ...) are not
  // exposed (#838).
  SQLDialect,
  SQLite,
  StandardSQL,
  keywordCompletionSource,
  schemaCompletionSource,
  sql,

  // @lezer/common
  parseMixed,

  // @lezer/highlight
  classHighlighter,
  styleTags,
  tags,

  // lezer-fhirpath — nested, see the doc comment above.
  fhirpath: {
    parser: fhirpathParser,
    props: fhirpathProps,
    terminals: fhirpathTerminals,
  },

  version,
};

// The one global this bundle defines. `window` exists in every real browser
// context, including the classic `<script>` tag this bundle is loaded with;
// `globalThis` is the portable fallback for anything else (a worker, a
// non-browser JS host, or a Node `vm` sandbox with no `window` property).
// No other identifier is ever assigned on either object.
(typeof window !== 'undefined' ? window : globalThis).HfsCodeMirror = HfsCodeMirror;
