# `locales/` — UI message catalogs

This directory holds the **translatable strings for the HFS user interface**,
one subdirectory per locale, in [Project Fluent](https://projectfluent.org/)
(`.ftl`) format.

```
locales/
├── en/main.ftl   ← SOURCE locale (canonical key set) — edit here first
├── es/main.ftl   ← Spanish
└── de/main.ftl   ← German
```

## Rules of the road

- **English (`en`) is the source of truth.** Add or rename a key in
  `en/main.ftl` first; every other locale must define the same key set.
- **Missing keys fall back** through the negotiated chain to `en` (see the
  full policy in [`docs/multi-language.md`](../docs/multi-language.md)). A
  missing translation degrades to English, never to a raw key or a blank.
- **Translations are data, not code.** No HTML, no logic, no string
  concatenation in templates — put the whole sentence in the catalog and
  interpolate values with Fluent placeables (`{ $var }`).
- **Pluralization uses CLDR categories** via Fluent selectors (`[one]`,
  `*[other]`, …). Do not build `"1 result(s)"` by hand.
- **Keep keys stable and semantic** (`nav-dashboard`, not `label_17`).

## Adding a language

1. Copy `en/` to a new locale directory (e.g. `fr/`).
2. Translate every value; leave keys and placeables untouched.
3. Register the locale in the UI's supported-locale list and add it to the
   language switcher (`language-*` keys).

See [`docs/multi-language.md`](../docs/multi-language.md) for how these
catalogs are loaded, how locale negotiation works end to end, and how UI
localization relates to FHIR content and terminology (SNOMED) localization.
