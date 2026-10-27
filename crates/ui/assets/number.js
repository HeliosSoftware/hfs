/*
 * Shared locale number formatting: `window.HfsNumber.format(value, options?)`
 * renders a figure the way the page's own locale writes it — `70,048` in
 * English, `70.048` in German and Spanish — the same rule #1426 introduced
 * for the result-header counts, now applied to every number the UI writes.
 *
 * The locale is the negotiated `<html lang>`, never the browser's own: a
 * page rendered in Spanish groups its numbers the Spanish way even in an
 * English browser, matching the server, which formats with the same CLDR
 * rules (`helios_ui_chrome::number`). An absent attribute leaves the choice
 * to the platform.
 *
 * Only displayed text goes through here. Wire values — query strings,
 * request bodies, input values, `data-*` attributes — and identifiers such
 * as HTTP statuses and version ids stay as they are. Plural choices keep
 * comparing the raw number; format at the point the text is written.
 *
 * `options` is passed to `Intl.NumberFormat` (e.g. `maximumFractionDigits`);
 * `options.lang` overrides the page locale. A value that is not a finite
 * number comes back as `String(value)`, unchanged.
 *
 * UMD wrapper — same shape as `unsaved.js` — so a unit test can `require()`
 * this file under plain Node: nothing touches `document` at load time.
 */
(function (root, factory) {
  "use strict";

  var api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root) root.HfsNumber = api;
})(typeof window !== "undefined" ? window : null, function () {
  "use strict";

  function pageLang() {
    if (typeof document === "undefined") return undefined;
    return document.documentElement.lang || undefined;
  }

  function format(value, options) {
    var number = typeof value === "number" ? value : Number(value);
    if (value === null || value === "" || !Number.isFinite(number)) return String(value);
    var opts = {};
    var lang = pageLang();
    if (options) {
      for (var key in options) {
        if (key === "lang") lang = options.lang || undefined;
        else opts[key] = options[key];
      }
    }
    return number.toLocaleString(lang, opts);
  }

  return { format: format };
});
