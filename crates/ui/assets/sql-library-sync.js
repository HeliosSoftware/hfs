/*
 * Live two-way sync between the Details JSON attachment and the SQL card
 * (#1233), on `/ui/sql/queries` and `/ui/sql/views` (`sql-library.html`).
 * `sql-library-details.js` (the Details JSON pairing) and `sql-editor.js`
 * (the SQL pane) each mount their own editor over their own textarea, kept
 * as two genuinely separate documents by design (#840/#838) — this file is
 * what makes them read as one Library while a visitor types, on top of both,
 * without either of those two files needing to know about the other:
 *
 *   - typing in the SQL card (`input` on `textarea[name="sql"]`) re-encodes
 *     that text into the first `application/sql` attachment of the Details
 *     JSON (`jsonWithSql`, appending one when none exists yet) and writes it
 *     back through `window.HfsSqlLibraryDetails.host.setDoc` when that host
 *     mounted (one undoable transaction, Ctrl+Z-able, refreshes the guided
 *     form) — or, without it, straight into the JSON textarea plus a
 *     bubbling `input` event, the same plain-textarea fallback every host in
 *     this family keeps;
 *   - editing that attachment's `data` in the Details JSON (`input` on
 *     `textarea[name="json"]`) decodes it (`sqlFromJson`) and writes the SQL
 *     card through `window.HfsSqlEditor.view`'s own minimal transaction
 *     (`window.HfsEditorPair.minimalChange`, the same diff `editor-pair.js`
 *     uses for its own form->editor sync) when the SQL editor mounted — or,
 *     without it, straight into the SQL textarea plus `input`. A JSON that
 *     does not parse, carries no `application/sql` attachment, or carries
 *     one this cannot read (missing `data`, invalid base64, non-UTF-8 bytes)
 *     never touches the SQL card — `mount`'s optional `onState` callback
 *     still hears about it (`"invalid-json"` / `"none"` / `"unreadable"` /
 *     `"ok"`), an extension point for a caller that wants to surface it.
 *     The auto-mount below is exactly that caller (#1233): its `onState`
 *     shows the `#sql-attachment-state` chip (`sql-library.html`) only for
 *     `"unreadable"`, and hides it again for every other state — the chip
 *     goes away both when the JSON is repaired and when the SQL card is
 *     typed into, since a card edit re-encodes the attachment and makes it
 *     readable again by construction.
 *
 * Anti-echo: both textareas fire `input` for a programmatic write exactly
 * like a manual edit (`host.setDoc`, `view.dispatch`, and the plain-textarea
 * fallbacks all do), which would otherwise bounce straight back into the
 * handler that just wrote it. A single `applying` flag, set for the
 * duration of every programmatic write on either side, is the first guard —
 * both handlers return immediately while it is set. The second is the value
 * comparison already needed to decide whether to write at all (the JSON's
 * own canonical form for the SQL->JSON direction, the decoded SQL text
 * itself for the JSON->SQL direction): a write that would not change
 * anything never happens, so even a write this flag somehow missed cannot
 * start a second round trip.
 *
 * Nothing is written when this file first runs — the server already painted
 * both sides from the same stored (or starter) document, so mounting adds no
 * extra live-run request beyond what a visitor's own first keystroke
 * triggers.
 *
 * `encodeSql`/`decodeSql`/`findSqlAttachment`/`sqlFromJson`/`jsonWithSql` are
 * pure and exported (`window.HfsSqlLibrarySync`, `module.exports`, the same
 * UMD-ish shape `editor-pair.js` uses) for their own unit test
 * (`crates/ui/e2e/unit/sql-library-sync.test.cjs`) — the "first attachment
 * whose `contentType` starts with `application/sql`" rule they implement is
 * the client-side mirror of `crates/ui/src/sql_libraries.rs`'s own
 * `readable_sql`/`embed_sql`. `mount` is the page wiring; it auto-runs at
 * the bottom of this file, browser-only, when both textareas this page
 * defines exist — without either, without `window` itself (Node's `require`
 * for the unit ring), this file only defines the functions above and does
 * nothing else, exactly like `editor-pair.js` does for its own `mount`.
 * `getJsonHost`/`getSqlView` are read fresh on every keystroke rather than
 * captured once at mount, since this script loads before
 * `sql-library-panels.js` in `sql-library.html`'s own script order but a
 * page without the vendored CodeMirror bundle (or a construction error in
 * either editor) never sets either global at all — reading them lazily
 * means this file degrades exactly like everything around it, never
 * throwing on a missing host or view.
 */
(function (root, factory) {
  "use strict";

  var api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root) root.HfsSqlLibrarySync = api;
})(typeof window !== "undefined" ? window : null, function () {
  "use strict";

  /* ---- base64 <-> text, matching `sql_libraries.rs`'s own `BASE64`
   * (`base64::engine::general_purpose::STANDARD`, padded) ------------------
   *
   * `btoa`/`atob` only ever see a binary string (one JS char per byte,
   * 0-255) - `TextEncoder`/`TextDecoder` do the actual UTF-8 <-> bytes work,
   * same as the server's own `String::from_utf8`/UTF-8 bytes round trip. */

  function encodeSql(text) {
    var bytes = new TextEncoder().encode(text);
    var binary = "";
    for (var i = 0; i < bytes.length; i++) {
      binary += String.fromCharCode(bytes[i]);
    }
    return btoa(binary);
  }

  /* `{ ok: false }` for anything not decodable as UTF-8 text: not a string,
   * not valid base64 (`atob` throws), or valid base64 whose bytes are not
   * valid UTF-8 (`TextDecoder`'s own `fatal: true` throws instead of
   * silently substituting U+FFFD - matching `String::from_utf8`'s own
   * strictness server-side). */
  function decodeSql(data) {
    if (typeof data !== "string") return { ok: false };
    var binary;
    try {
      binary = atob(data);
    } catch (invalidBase64) {
      return { ok: false };
    }
    var bytes = new Uint8Array(binary.length);
    for (var i = 0; i < binary.length; i++) {
      bytes[i] = binary.charCodeAt(i);
    }
    try {
      return { ok: true, text: new TextDecoder("utf-8", { fatal: true }).decode(bytes) };
    } catch (invalidUtf8) {
      return { ok: false };
    }
  }

  /* The index of `doc.content`'s first element whose `contentType` starts
   * with `application/sql` - `-1` when `doc.content` is not an array, or
   * has no such element. Mirrors `sql_libraries.rs`'s own `readable_sql`/
   * `embed_sql` predicate (prefix, not exact match) exactly. */
  function findSqlAttachment(doc) {
    var content = doc && doc.content;
    if (!Array.isArray(content)) return -1;
    for (var i = 0; i < content.length; i++) {
      var item = content[i];
      var contentType = item && item.contentType;
      if (typeof contentType === "string" && contentType.indexOf("application/sql") === 0) {
        return i;
      }
    }
    return -1;
  }

  /* The Details JSON's own effective SQL, or why there isn't one:
   *   - "invalid-json"  - `jsonText` does not parse at all.
   *   - "none"          - parses, but carries no `application/sql` attachment.
   *   - "unreadable"    - carries one, but its `data` is missing or not
   *                       decodable (see `decodeSql`).
   *   - { state: "ok", sql } - carries one this can read, decoded. */
  function sqlFromJson(jsonText) {
    var doc;
    try {
      doc = JSON.parse(jsonText);
    } catch (invalidJson) {
      return { state: "invalid-json" };
    }
    var index = findSqlAttachment(doc);
    if (index === -1) return { state: "none" };
    var decoded = decodeSql(doc.content[index].data);
    if (!decoded.ok) return { state: "unreadable" };
    return { state: "ok", sql: decoded.text };
  }

  /* `jsonText` with `sql` patched into its first `application/sql`
   * attachment's `data` (appending one, creating `content` when it is
   * missing entirely, when there is none yet) - `null` when `jsonText` does
   * not parse, parses to something other than a JSON object, or already has
   * a `content` key that is not an array (a hand-typed object, say): there
   * is nothing safe to patch or append to there, so this leaves it
   * completely untouched rather than silently replacing it - the same
   * no-op `embed_sql` (`sql_libraries.rs`) falls back to when its own
   * `as_array_mut()` fails for the same reason. Every other attachment, and
   * the patched one's own order among them, is left exactly as it was; the
   * result is a 2-space pretty print, matching the Details pane's own
   * server-side formatting. */
  function jsonWithSql(jsonText, sql) {
    var doc;
    try {
      doc = JSON.parse(jsonText);
    } catch (invalidJson) {
      return null;
    }
    if (typeof doc !== "object" || doc === null || Array.isArray(doc)) return null;

    var index = findSqlAttachment(doc);
    if (index === -1) {
      if (doc.content === undefined) {
        doc.content = [];
      } else if (!Array.isArray(doc.content)) {
        return null;
      }
      doc.content.push({ contentType: "application/sql", data: encodeSql(sql) });
    } else {
      doc.content[index].data = encodeSql(sql);
    }
    return JSON.stringify(doc, null, 2);
  }

  /* ---- mount --------------------------------------------------------------
   *
   * `jsonTextarea`/`sqlTextarea` (required) - the two textareas to keep in
   * step. `getJsonHost()`/`getSqlView()` (required) - read fresh on every
   * keystroke, returning the current `editor-pair.js` host (`{getDoc,
   * setDoc}`) and CodeMirror `EditorView` respectively, or a falsy value
   * when either has not mounted (or never will, no bundle) - this file then
   * falls back to writing the plain textarea plus `input` for that side,
   * same as every other host in this family. `minimalChange` (required for
   * the JSON->SQL direction with a mounted `view`) -
   * `window.HfsEditorPair.minimalChange`. `onState` (optional) - an
   * extension point for a caller that wants to reflect the Details JSON's
   * own attachment state somewhere; called with `sqlFromJson`'s own `state`
   * after every JSON->SQL attempt, and with `"ok"` after every SQL->JSON
   * write. The auto-mount below passes one that drives the
   * `#sql-attachment-state` chip (#1233).
   */
  function mount(options) {
    options = options || {};
    var jsonTextarea = options.jsonTextarea;
    var sqlTextarea = options.sqlTextarea;
    if (!jsonTextarea || !sqlTextarea) return;

    var getJsonHost = options.getJsonHost || function () {
      return null;
    };
    var getSqlView = options.getSqlView || function () {
      return null;
    };
    var minimalChange = options.minimalChange;
    var onState = options.onState;

    // The anti-echo flag: set for the duration of every programmatic write
    // below, on either textarea - both handlers bail immediately while it
    // is set, before even re-parsing anything.
    var applying = false;

    function canonicalOrNull(jsonText) {
      try {
        return JSON.stringify(JSON.parse(jsonText));
      } catch (invalid) {
        return null;
      }
    }

    // SQL card -> Details JSON.
    function onSqlInput() {
      if (applying) return;
      var jsonText = jsonTextarea.value;
      var next = jsonWithSql(jsonText, sqlTextarea.value);
      // `next === null`: the JSON does not parse at all right now - the
      // pairing's own "Invalid JSON" chip already says so; nothing to patch
      // until it parses again. Otherwise, only write when the canonical
      // form actually moved - the second anti-echo guard, on top of
      // `applying` above.
      if (next !== null && canonicalOrNull(next) !== canonicalOrNull(jsonText)) {
        applying = true;
        try {
          var host = getJsonHost();
          if (host) {
            host.setDoc(next);
          } else {
            jsonTextarea.value = next;
            jsonTextarea.dispatchEvent(new Event("input", { bubbles: true }));
          }
        } finally {
          applying = false;
        }
      }
      if (onState) onState("ok");
    }

    // Details JSON -> SQL card.
    function onJsonInput() {
      if (applying) return;
      var result = sqlFromJson(jsonTextarea.value);
      if (onState) onState(result.state);
      // "invalid-json" / "none" / "unreadable": nothing readable to show -
      // the SQL card is left exactly as it is.
      if (result.state !== "ok") return;
      var sqlText = sqlTextarea.value;
      if (result.sql === sqlText) return;
      applying = true;
      try {
        var view = getSqlView();
        if (view && minimalChange) {
          var change = minimalChange(sqlText, result.sql);
          if (change) {
            view.dispatch({ changes: { from: change.from, to: change.to, insert: change.insert } });
          }
        } else {
          sqlTextarea.value = result.sql;
          sqlTextarea.dispatchEvent(new Event("input", { bubbles: true }));
        }
      } finally {
        applying = false;
      }
    }

    sqlTextarea.addEventListener("input", onSqlInput);
    jsonTextarea.addEventListener("input", onJsonInput);
  }

  return {
    encodeSql: encodeSql,
    decodeSql: decodeSql,
    findSqlAttachment: findSqlAttachment,
    sqlFromJson: sqlFromJson,
    jsonWithSql: jsonWithSql,
    mount: mount,
  };
});

// Auto-mount (browser only - this whole block never runs under Node's own
// `require`, which is exactly how the unit ring above loads this same file
// with no `document` in sight): silently does nothing without both
// textareas `sql-library.html` defines, same silent-degradation contract
// every script in this family follows.
if (typeof window !== "undefined" && window.document) {
  (function () {
    "use strict";

    var jsonTextarea = document.querySelector('textarea[name="json"][form="lib-editor-form"]');
    var sqlTextarea = document.querySelector('#lib-editor-form textarea[name="sql"]');
    if (!jsonTextarea || !sqlTextarea) return;

    // The "unreadable attachment" chip (#1233): server-painted `hidden`,
    // with no text of its own — `sql-library.html` only ever sets
    // `data-msg-unreadable`. Missing entirely on a page without this chip
    // (there is none today, but nothing here requires one), `onState`
    // below is simply a no-op for every state.
    var stateChip = document.getElementById("sql-attachment-state");

    window.HfsSqlLibrarySync.mount({
      jsonTextarea: jsonTextarea,
      sqlTextarea: sqlTextarea,
      getJsonHost: function () {
        return window.HfsSqlLibraryDetails && window.HfsSqlLibraryDetails.host;
      },
      getSqlView: function () {
        return window.HfsSqlEditor && window.HfsSqlEditor.view;
      },
      minimalChange: window.HfsEditorPair && window.HfsEditorPair.minimalChange,
      onState: function (state) {
        if (!stateChip) return;
        if (state === "unreadable") {
          stateChip.textContent = stateChip.dataset.msgUnreadable;
          stateChip.hidden = false;
          stateChip.classList.remove("editor-validity--ok");
        } else {
          stateChip.hidden = true;
          stateChip.textContent = "";
        }
      },
    });
  })();
}
