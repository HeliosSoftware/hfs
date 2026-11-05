/*
 * FHIR search-value escaping shared by the visual query builder.
 *
 * This codec starts after URL percent-decoding. It owns only the FHIR layer:
 * unescaped commas separate OR alternatives. A literal comma is displayed as
 * itself because one input already represents one alternative. Other valid
 * escapes (`\\`, `\|`, and `\$`) stay visible because the generic input does
 * not model token or composite segments separately; retaining their escape
 * syntax also keeps every edited value reversible.
 */
(function (root, factory) {
  "use strict";

  var codec = factory();
  if (typeof module === "object" && module.exports) module.exports = codec;
  if (root) root.HfsFhirSearchValue = codec;
})(typeof globalThis !== "undefined" ? globalThis : this, function () {
  "use strict";

  function parseAlternatives(wireValue) {
    var input = wireValue == null ? "" : String(wireValue);
    var alternatives = [];
    var value = "";
    var wire = "";
    var error = null;

    function finish() {
      alternatives.push({ value: value, wire: wire, error: error });
      value = "";
      wire = "";
      error = null;
    }

    for (var i = 0; i < input.length; i++) {
      var current = input[i];
      if (current === ",") {
        finish();
        continue;
      }
      if (current !== "\\") {
        value += current;
        wire += current;
        continue;
      }

      wire += current;
      if (i + 1 >= input.length) {
        value += current;
        error = error || "trailing-escape";
        continue;
      }

      var escaped = input[++i];
      wire += escaped;
      if (escaped === ",") {
        value += ",";
      } else if (escaped === "\\") {
        value += "\\\\";
      } else if (escaped === "|" || escaped === "$") {
        value += "\\" + escaped;
      } else {
        value += "\\" + escaped;
        error = error || "invalid-escape";
      }
    }
    finish();

    return {
      alternatives: alternatives,
      error: alternatives.some(function (alternative) {
        return !!alternative.error;
      })
        ? "invalid-escape"
        : null,
    };
  }

  function serializeAlternative(value) {
    var input = value == null ? "" : String(value);
    var wire = "";
    for (var i = 0; i < input.length; i++) {
      var current = input[i];
      if (current === "\\") {
        var next = input[i + 1];
        if (next === "\\" || next === "|" || next === "$") {
          wire += "\\" + next;
          i++;
        } else {
          wire += "\\\\";
        }
      } else if (current === ",") {
        wire += "\\,";
      } else {
        wire += current;
      }
    }
    return wire;
  }

  function serializeAlternatives(values) {
    return (values || []).map(serializeAlternative).join(",");
  }

  return {
    parseAlternatives: parseAlternatives,
    serializeAlternative: serializeAlternative,
    serializeAlternatives: serializeAlternatives,
  };
});
