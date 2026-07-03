# Helios FHIR Server — UI message catalog
# Locale: English (en) — SOURCE LOCALE. Every key defined here is the
# canonical set; other locales are expected to provide the same keys.
#
# Syntax: Project Fluent (https://projectfluent.org/). Terms (prefixed with
# `-`) are reusable snippets; messages (bare identifiers) are what the UI
# looks up. Placeables `{ $var }` are interpolated by the caller. Do NOT put
# markup or logic here — translations are data, the template renders them.

## Brand / shared terms

-app-name = Helios FHIR Server
-org-name = Helios Software

## Page chrome

app-title = { -app-name }
app-tagline = A fast, multi-version FHIR server

nav-dashboard = Dashboard
nav-terminology = Terminology
nav-resources = Resources
nav-settings = Settings
nav-signout = Sign out

## Language switcher

language-label = Language
language-en = English
language-es = Spanish
language-de = German

## Dashboard / health

dashboard-heading = Server dashboard
health-status-ok = All systems operational
health-status-degraded = Some systems are degraded
health-uptime = Uptime: { $duration }

# Pluralized count — every locale must supply the plural categories its
# grammar requires (CLDR rules; Fluent selects the branch automatically).
resource-count = { $count ->
    [one] { $count } resource
   *[other] { $count } resources
}

## Terminology browsing

terminology-search-label = Search CodeSystems and ValueSets
terminology-search-placeholder = e.g. 73211009, "diabetes", http://snomed.info/sct
terminology-display-language = Display language
terminology-no-results = No matching concepts found.

## Common actions

action-search = Search
action-save = Save
action-cancel = Cancel
action-retry = Retry

## Errors (mirrors OperationOutcome text; see docs/multi-language.md §5)

error-not-found = The requested resource was not found.
error-unauthorized = You are not authorized to perform this action.
error-generic = Something went wrong. Please try again.

## Footer

footer-copyright = © { $year } { -org-name }
