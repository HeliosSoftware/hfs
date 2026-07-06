# Multi-language support — approach, guidelines, and rules of the road

**Status:** Foundation / discussion (living document)
**Branch:** `feat/i18n-foundation` (off `main`)
**Owner:** Angela
**Relates to:** #186 (HTMX-first web UI foundation), `feat/user-ui-settings`,
`feat/smart-ui-auth`

This document describes **how Helios FHIR Server (HFS) supports multiple
languages, from the front end to the back end.** The initial target languages
are **English (source), Spanish, and German**; the architecture is designed so
that adding a fourth (French, Portuguese, …) is a translation task, not an
engineering project.

It is deliberately opinionated: the point of a "rules of the road" document is
to make the *first* localization PR set the *right* precedent rather than an
accidental one. Where a concrete tool is named, it is a recommendation to be
ratified — the reasoning matters more than the pick.

---

## 1. What "multi-language" means here

Localization in HFS is **not one thing**. A user request touches several
independently-translated layers, and conflating them is the most common way i18n
efforts go wrong. We separate them explicitly:

| # | Layer | Example | Where it lives | Status |
|---|-------|---------|----------------|--------|
| 1 | **UI chrome / static strings** | "Dashboard", "Search", "Sign out" | `locales/*.ftl` message catalogs | **new (this issue)** |
| 2 | **Locale negotiation** | pick `de` for this request | middleware: header + user setting + override | **new (this issue)** |
| 3 | **API / error messages** | `OperationOutcome.text` / `.issue.details` | `helios-rest` responses, catalog-backed | scaffolded here, wired incrementally |
| 4 | **FHIR *content* localization** | `Resource.text` narrative, extensions | resource data itself | out of scope to translate; must be *passed through* correctly |
| 5 | **Terminology display** | SNOMED "diabetes mellitus" → German term | HTS designations + `displayLanguage` | **already implemented** (see §4) |
| 6 | **Formatting** | dates, numbers, units | locale-aware formatting at render time | guidelines here, applied in UI crate |

The critical mental model: **layers 1–3 are *our* strings** (we author and
translate them), **layers 4–5 are *the data's* strings** (we select and render
the right one but do not invent translations), and **layer 6 is presentation**.

---

## 2. Front-to-back request flow

A localized request flows through the stack like this:

```
Browser ──Accept-Language: de-DE, de;q=0.9, en;q=0.7──▶  hfs (Axum)
   │                                                       │
   │  (or ?lang=de override, or user-saved preference)     ▼
   │                                             ┌──────────────────────┐
   │                                             │ locale-negotiation   │  Layer 2
   │                                             │ middleware           │
   │                                             │  → RequestLocale     │
   │                                             └──────────┬───────────┘
   │                                                        │
   │                        ┌───────────────────────────────┼───────────────┐
   │                        ▼                               ▼               ▼
   │                 helios-ui (UI)                helios-rest (API)   helios-hts
   │                 renders Askama template        OperationOutcome    $expand/$lookup
   │                 + Fluent catalog (Layer 1)     text (Layer 3)      displayLanguage
   │                                                                    (Layer 5)
   ▼
HTML fragment / page   ◀── all three consult the SAME negotiated RequestLocale ──
```

**One negotiated locale per request, computed once, threaded everywhere.** The UI
chrome, the error text, and the terminology `displayLanguage` must all agree.
A page that says "Terminología" in the nav but shows English SNOMED terms is a
bug, and it is only avoidable if every layer reads the same `RequestLocale`.

### Locale negotiation precedence (highest wins)

1. **Explicit override** — `?lang=` query param or a `hfs_lang` cookie set by the
   language switcher. Lets a user read in a language other than their browser's.
2. **Saved user preference** — from `feat/user-ui-settings` (a per-user setting),
   when authenticated.
3. **`Accept-Language`** — RFC 4647 lookup against our supported set.
4. **Server default** — `en`.

The negotiated result is a single value carried in request extensions (e.g.
`RequestLocale(LanguageIdentifier)`), available to every handler. The same value
is passed to HTS as `displayLanguage` / forwarded as `Accept-Language` so
terminology display matches the UI.

---

## 3. UI strings — the chosen approach (Layers 1–2)

### 3.1 Format: Project Fluent

We store UI strings as **[Project Fluent](https://projectfluent.org/) (`.ftl`)**
catalogs under `locales/<locale>/main.ftl` (seeded in this branch for `en`,
`es`, `de`).

**Why Fluent over flat key/value (JSON/`gettext`):**

- **Grammar-correct plurals and selectors** via CLDR categories — German and
  Spanish plural rules differ from English, and Fluent selects the right branch
  (`[one] … *[other] …`) instead of forcing `"1 result(s)"`.
- **Asymmetric translation:** a translation may need a plural/gender branch the
  English source does not. Fluent allows a locale to add branches without
  changing the source — flat maps cannot express this.
- **Placeables and terms** (`{ -app-name }`, `{ $count }`) keep interpolation and
  reusable brand strings in the catalog, not in Rust.
- **First-class Rust support** via [`fluent`](https://docs.rs/fluent) /
  [`fluent-templates`](https://docs.rs/fluent-templates), which integrates with
  **Askama** — the templating engine proposed for the UI in #186. This keeps the
  toolchains aligned.

**Trade-off:** Fluent is a richer format than a JSON map, so translators need a
one-page primer. That cost is paid once and is far smaller than the cost of
retrofitting plural/gender handling onto a flat catalog later.

*Rejected alternatives:* a hand-rolled `HashMap<&str,&str>` (no plurals, no
fallback semantics, invites string-concatenation bugs); `gettext`/`.po` (mature,
but weaker Rust ergonomics and a clumsier plural model than Fluent).

### 3.2 Loading and rendering

- Catalogs are **embedded at build time** (via `fluent-templates`'
  `static_loader!` / `rust-embed`) so the server is a single binary with no
  runtime file dependency — consistent with the asset-embedding stance in #186
  (no runtime CDN, air-gap-friendly for a healthcare deployment).
- Templates look up strings by key through a small helper (e.g. an Askama filter
  or a `t("nav-dashboard")`-style function bound to the request's
  `RequestLocale`). **Templates never hold English text** — they hold keys.
- Missing-key and missing-locale behavior is a **fallback chain**, never a crash
  and never a blank: `negotiated locale → its base language → en`. A key present
  in `en` but absent in `de` renders the English string (logged in debug), so a
  half-translated locale is always usable.

### 3.3 Where things go — rules of the road

**Where UI localization lives**
- `locales/<locale>/main.ftl` — all translatable UI text. Split into multiple
  `.ftl` files per feature area only when `main.ftl` gets unwieldy; keep the same
  split across all locales.
- Locale-negotiation middleware — in the UI/rest layer, producing one
  `RequestLocale` per request.
- A thin template helper that resolves keys against the request locale.

**Where it must NOT go**
- **No hardcoded human-readable strings in Rust or templates.** If a user can
  read it, it comes from a catalog. (Log messages and developer-facing errors are
  exempt — those stay English.)
- **No string concatenation to build sentences** ("You have " + n + " results").
  Sentence structure varies by language; use one keyed message with placeables
  and a plural selector.
- **No per-locale branching in handler logic** (`if lang == "de"`). Differences
  live in the catalog, not in Rust control flow.
- **No new browser-facing JSON API for translations** — the UI renders localized
  HTML server-side (consistent with #186's hypermedia stance).
- **Do not localize identifiers, codes, URLs, or FHIR element names** — only
  human-facing prose.

---

## 4. Terminology localization — already implemented (Layer 5)

HFS's terminology server (HTS) **already supports multi-language SNOMED CT** (and
LOINC linguistic variants), and the UI must build on this rather than reinvent
it. This is the most mature localization layer in the product.

- **Multi-language import.** The SNOMED RF2 importer
  (`crates/hts/src/import/snomed_rf2.rs`) imports *all* active descriptions in
  *every language present in the archive*, including the per-language description
  files shipped by national editions, as FHIR `concept.designation` entries
  tagged with the RF2 `languageCode`. Language reference sets are consulted to
  emit `preferredForLanguage` designations for the refset-preferred synonym in
  each language.
- **Import can be scoped by language** via `LanguageFilter`
  (`HTS_IMPORT_LANGUAGES` / `--languages`) so a deployment can import only the
  languages it serves; excluded per-language files are skipped without being
  parsed.
- **Runtime language selection.** `$expand` / `$lookup` / `$validate-code` honor
  `displayLanguage` (and the `Accept-Language` header) to return the right term.
  The matching logic lives in `crates/hts/src/language.rs`, which implements
  **RFC 4647 §3.4 Lookup** with progressive tag truncation — deliberately built
  to reconcile the *heterogeneous* language tags terminologies ship (SNOMED RF2
  bare tags like `de`; LOINC region-qualified tags like `de-DE`) with whatever a
  browser sends (`de-DE` via `Accept-Language`). It ranks candidates so `es-ES`
  beats `esES` beats `es`/`es-MX`, and a `de` request accepts a stored `de-CH`.

**Rule of the road:** the UI's terminology screens pass the request's negotiated
locale straight through to HTS as `displayLanguage`. **Do not** add a second,
UI-local translation table for clinical terms — the authoritative multilingual
terms already exist as designations, and duplicating them would drift from the
source terminology. UI *chrome* around a term ("Preferred term", "Synonyms") is
Layer 1; the term itself is Layer 5.

---

## 5. API and error messages (Layer 3)

`OperationOutcome` is a FHIR resource returned to *both* machines and humans. Its
diagnostic prose (`issue.details.text`, `issue.diagnostics`, the human `text`
narrative) should be localizable to the request locale, while the machine-facing
parts stay stable:

- **Localize:** `OperationOutcome.text` (the human narrative) and
  `issue.details.text`.
- **Do NOT localize:** `issue.code`, `issue.details.coding` (the codes machines
  branch on), resource identifiers, or field paths in `issue.expression`.

Error strings live in the same Fluent catalogs (`error-*` keys, seeded here) so
there is one translation workflow, not two. This layer is **scaffolded** in this
branch and wired into `helios-rest` responses incrementally — negotiation and the
catalog exist; individual message sites are migrated off hardcoded English as
they are touched, to avoid one giant risky sweep.

---

## 6. FHIR content localization (Layer 4) — pass-through, don't invent

FHIR resources carry their own language metadata, and HFS's job is to **preserve
and surface it correctly, not to translate clinical content**:

- `Resource.language` (the language of the resource) and `Resource.text` (the
  narrative) are authored data. Round-trip them faithfully across all supported
  FHIR versions (R4/R4B/R5/R6) — do not strip, rewrite, or machine-translate.
- Translatable content extensions (e.g. the R5+ translation extension on
  string elements) are data: render the variant matching the request locale when
  present, fall back to the base value otherwise.

**Rule of the road:** translating patient/clinical content is a clinical-safety
concern and is **out of scope** for HFS's UI i18n. We select among translations
that already exist in the data; we never generate them.

---

## 7. Formatting (Layer 6)

Dates, times, numbers, and quantities are **presentation** and must be formatted
for the render locale, not hardcoded to `en-US`:

- Format at the edge (in the UI/template layer), from locale-neutral values
  (ISO 8601 instants, numeric quantities). Never store a pre-formatted localized
  string.
- Prefer a maintained locale-data library (e.g. ICU4X) over ad-hoc `strftime`
  patterns; a German user expects `2. Juli 2026`, a US user `July 2, 2026`.
- **Never localize FHIR wire formats** — `date`/`dateTime`/`instant` on the FHIR
  API are always ISO 8601 regardless of UI locale. Formatting is a display-only
  transform applied after the data leaves the FHIR layer.
- UCUM units are codes, not prose — do not translate them.

---

## 8. Security & correctness notes

- **Auto-escaping still applies to translated text.** Treat catalog values as
  untrusted for output-encoding purposes; the template engine's auto-escaping
  (Askama, per #186) must not be bypassed for "trusted" translations. Interpolate
  values as placeables so they are escaped, not spliced into raw markup.
- **No user-controlled format strings.** Fluent placeables are named and typed;
  never build a catalog key or a message body from user input.
- **Locale is not authorization.** Negotiated locale changes *presentation only*;
  it must never widen data access or alter tenant/scope decisions.

---

## 9. Language roadmap

| Phase | Languages | Notes |
|-------|-----------|-------|
| Foundation (this issue) | `en` (source), `es`, `de` | catalogs, negotiation, docs, POC hook |
| Next | fill error/API messages (§5) | migrate hardcoded strings incrementally |
| Later | `fr`, `pt`, … | pure translation once §1–§7 hold |

Adding a language = copy `locales/en/`, translate values, register the locale,
add it to the switcher. If that ever requires touching Rust control flow, the
rules of the road in §3.3 have been violated.

---

## 10. Deliverables in this branch

- `locales/{en,es,de}/main.ftl` — seeded UI message catalogs (source + two).
- `locales/README.md` — catalog conventions.
- `docs/multi-language.md` — this document.

Structure that lands with the UI crate (#186), not here: the negotiation
middleware, the Askama/Fluent template helper, and the `Cargo.toml` wiring — this
branch establishes the **shape and the rules**; the runtime wiring rides on the
`helios-ui` foundation so the two don't conflict. That wiring now lives in
`crates/ui/src/i18n.rs` (the `RequestLocale` middleware, the `hfs_lang`
switcher cookie, and the Fluent lookup helper the templates use).

---

## 11. References

- **Project Fluent** — https://projectfluent.org/ (syntax guide, plural/selector
  model) and the Fluent **Rust** crates: https://docs.rs/fluent and
  https://docs.rs/fluent-templates
- **Unicode CLDR plural rules** — https://cldr.unicode.org/index/cldr-spec/plural-rules
- **BCP 47** (language tags) — https://www.rfc-editor.org/info/bcp47 and
  **RFC 4647** (language-tag matching / Lookup) —
  https://www.rfc-editor.org/rfc/rfc4647 (basis for `crates/hts/src/language.rs`)
- **HTTP `Accept-Language`** — https://developer.mozilla.org/docs/Web/HTTP/Headers/Accept-Language
- **ICU4X** (locale-aware date/number formatting in Rust) — https://github.com/unicode-org/icu4x
- **FHIR — resource language & narrative** — https://hl7.org/fhir/resource.html#language
  and https://hl7.org/fhir/narrative.html
- **FHIR terminology — designations & `displayLanguage`** —
  https://hl7.org/fhir/valueset-operation-expand.html (see `displayLanguage`)
- **SNOMED CT — language reference sets** —
  https://confluence.ihtsdotools.org/display/DOCGLOSS/language+reference+set
- **W3C Internationalization** (best practices) — https://www.w3.org/International/
- **OWASP — output encoding / XSS** (applies to server-rendered translated
  strings) — https://cheatsheetseries.owasp.org/cheatsheets/Cross_Site_Scripting_Prevention_Cheat_Sheet.html
- Related HFS work: #186 (HTMX-first web UI foundation), `feat/user-ui-settings`
  (per-user preferences, incl. language), `feat/smart-ui-auth`.
