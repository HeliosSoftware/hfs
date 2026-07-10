# UI mockups

Interactive, self-contained HTML mockups for screens that are not built yet.
Open them directly in a browser (double-click; no server, no network, no
account — fonts and data are embedded). Each one is loaded with **real data
from this repo**, so counts and behaviors are what the real screen will show.

| Mockup | Issue | Data |
|---|---|---|
| [`search-parameter-editor.html`](search-parameter-editor.html) | [#238](https://github.com/HeliosSoftware/hfs/issues/238) | `data/search-parameters-r4.json` (1,375 params, 135 base types) |
| [`compartment-editor.html`](compartment-editor.html) | [#237](https://github.com/HeliosSoftware/hfs/issues/237) | `crates/fhir-gen/resources/R4/compartmentdefinition-*.json` |

A static `.png` preview sits next to each file.

## Relationship to the real screens

These are **prototypes, not the implementation**. Per the `helios-ui` crate
rules, the real markup lives in `templates/` (Askama), interactions are htmx
fragment swaps served by the server, and read paths work with JavaScript
disabled. The mockups' inline JS stands in for what will be server round
trips — e.g. the compartment route tester becomes a `GET /ui/compartments/test`
fragment, and the SearchParameter table reads the live registry (storage-backed
after #235) instead of an embedded snapshot.

Style is the crate's design system verbatim: `crates/ui/assets/app.css` tokens,
the vendored Figtree face (inlined as a data URI), and both themes via
`prefers-color-scheme` plus the `data-theme` override — with `--ok` / `--warn` /
`--danger` added as semantic tokens held off the accent hue.

The SearchParameter editor's validation reflects the registry semantics after
#242: a cross-source `(base, code)` shadow is a supported override (chip:
`overrides spec`), a same-source collision is a blocking `conflict`
(`RegistryError::DuplicateCode`).
