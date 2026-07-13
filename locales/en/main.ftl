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

## Home / landing page

home-lede = Server-rendered, HTMX-first UI. This panel is refreshed as an HTML fragment.

## Status panel

status-last-checked = Last checked: { $timestamp }

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

## Dashboard shell (Figma "Dashboard V1.1")

nav-section-work = Work
nav-section-batch-data = Batch & Data
nav-section-server = Server
nav-section-conditional = Conditional

nav-home = Home
nav-search = Search
nav-resource-editor = Resource Editor
nav-history-versions = History & Versions
nav-compartments = Compartments
nav-batch-transaction = Batch / Transaction
nav-bulk-export = Bulk Export
nav-sql-on-fhir = SQL-on-FHIR
nav-capability-conformance = Capability & Conformance
nav-search-parameters = Search Parameters
nav-admin-ops = Admin / Ops
nav-subscriptions = Subscriptions

tenant-heading = Tenants
tenant-all = All tenants
tenant-search-placeholder = Search tenants

theme-label = Theme
theme-light = Light theme
theme-dark = Dark theme

fhir-version = FHIR { $version }

card-resource-types = Resource types
card-resource-types-sub = enabled for { $version }
card-stored-resources = Stored resources
card-stored-resources-sub = across active tenant
card-export-jobs = Export jobs
card-export-jobs-sub = running ({ $queued } queued)
card-uptime = Uptime
card-uptime-sub = last 30 days

chart-title = FHIR resources over time
chart-unit-patients = patients
chart-expand = Expand chart

## Footer

footer-copyright = © { $year } { -org-name }

## Saved queries (#234)

nav-saved-queries = Saved Queries

queries-heading = Saved queries
queries-lede = Keep FHIR search queries per resource type, sorted by when you last ran them. Saved to your user settings, so they roam across devices.
queries-add-heading = Save a query
queries-type-label = Resource type
queries-type-placeholder = e.g. Patient
queries-name-label = Name
queries-name-placeholder = e.g. Smiths in Boston
queries-query-label = Query string
queries-query-placeholder = e.g. name=smith&address-city=Boston
queries-empty = No saved queries yet. Save one above to get started.
queries-never-run = Never run
queries-run = Run
queries-rename = Rename
queries-delete = Delete
queries-rename-prompt = New name
queries-confirm-delete = Delete "{ $name }"?
queries-unavailable = Saved queries are unavailable: this server's storage backend does not support per-user settings.

## SearchParameter viewer (#238)

sp-heading = Search parameters
sp-lede = Browse the parameters this server resolves searches against, filtered by base resource type. Spec parameters are read-only; tenant-scoped editing arrives once search parameters live in storage.
sp-version-label = FHIR version
sp-spec-missing = The full spec bundle (search-parameters-*.json) was not found in the data directory — only the minimal embedded fallback parameters are shown.
sp-rail-label = Resource filter
sp-rail-search = Filter types
sp-rail-recent = Recently used
sp-rail-types = Resource types
sp-rail-all = All types
sp-facet-type = Type
sp-facet-type-label = Filter by parameter type
sp-facet-source = Source
sp-facet-source-label = Filter by source
sp-source-embedded = embedded
sp-source-stored = stored
sp-source-config = config
sp-chip-conflict = conflict
sp-chip-overrides = overrides spec
sp-chip-shadowed = shadowed
sp-col-code = Code
sp-col-type = Type
sp-col-base = Base
sp-col-expression = Expression
sp-col-source = Source
sp-total = { $count } parameters
sp-pagination-label = Pages
sp-page-prev = Previous
sp-page-next = Next
sp-detail-label = Parameter detail
sp-detail-empty = No parameter selected
sp-detail-empty-hint = Select a row to inspect its definition, expression, and how it resolves against the registry.
sp-detail-readonly = Spec parameter (compiled in from the data file) — read-only.
sp-field-url = Canonical URL
sp-field-name = Name
sp-field-status = Status
sp-field-base = Base resource types
sp-field-expression = FHIRPath expression
sp-field-description = Description
sp-field-target = Target types
sp-field-components = Components
sp-status-hint = The loader promotes the spec's draft status to active on load.
sp-note-conflict = Duplicate (base, code) within the same source as { $url } — the registry rejects this collision (DuplicateCode).
sp-note-overrides = Overrides { $url } on (base, code): a Stored definition outranks the spec parameter, so this one resolves searches. The registry logs a WARN naming both URLs.
sp-note-shadowed = Shadowed by { $url } on (base, code): a higher-precedence source resolves searches for this slot.
sp-note-empty-expression = Empty expression: the extractor indexes zero rows, so every search on this parameter silently returns empty.
sp-note-no-target = Reference parameter with no target types: chained search cannot resolve the referenced type.
sp-note-choice-type = Choice-type expression: the extractor rewrites ofType(T) / as T to the concrete element (for example valueQuantity) before evaluating against raw stored JSON.
sp-writes-pending = Creating, overriding, and deleting tenant parameters lands once search parameters are stored in the database (#235).

## Compartment viewer & tester (#237)

cmp-heading = Compartments
cmp-lede = The compartment definitions this server routes /{"{"}compartment{"}"}/{"{"}id{"}"}/{"{"}type{"}"} requests with, and a tester that answers: is this type in this compartment, via which parameters, and what search does the server run?
cmp-rail-label = Compartment definitions
cmp-rail-heading = Compartments
cmp-rail-note = Base definitions ship with the server (codegen'd from the FHIR spec). Editing them implies a tenant-scoped override layer — open question on the issue.
cmp-tabs-label = Compartment sections
cmp-tab-definition = Definition
cmp-tab-members = Members
cmp-tab-tester = Tester
cmp-field-code = Code
cmp-field-status = Status
cmp-field-url = Canonical URL
cmp-field-version = Version
cmp-field-publisher = Publisher
cmp-field-description = Description
cmp-field-search = search
cmp-field-experimental = experimental
cmp-search-why = Off would mean no compartment route resolves for this compartment.
cmp-on = on
cmp-off = off
cmp-yes = yes
cmp-no = no
cmp-readonly-note = Read-only: these values come from the spec definitions compiled into the server.
cmp-filter-members = Members
cmp-filter-all = All types
cmp-filter-excluded = Excluded
cmp-member = member
cmp-excluded = excluded
cmp-tester-id = Id
cmp-tester-target = Target type (or *)
cmp-tester-run = Test
cmp-result-member = ✓ member — via { $params }
cmp-result-flat = // equivalent flat search
cmp-result-member-note = The server resolves the compartment route to this search over the type's reference parameters.
cmp-result-self = ✓ member — the compartment resource itself ({"{"}def{"}"})
cmp-result-self-note = The compartment instance is trivially in its own compartment; the route reads the resource directly.
cmp-result-notmember = ✕ { $type } is not a member of this compartment
cmp-result-notmember-note = The server returns 404 with an OperationOutcome for types that are not compartment members.
cmp-result-fanout = Fans out to { $count } member types
cmp-result-fanout-note = Excluded types are skipped, not failed — the fan-out drops non-member types rather than erroring.
queries-builder-heading = Search builder
queries-url-label = FHIR search URL
queries-url-placeholder = GET /Patient?name=smith&birthdate=ge1980-01-01
queries-builder-hint = Edit the GET URL directly or through the rows below — they stay in sync. Run executes the search here and records it under Recent; give it a name to keep it in the saved list.
queries-recent = Recent
queries-recent-heading = Recent searches
queries-recent-empty = No recent searches yet — Run one to record it here.
queries-invalid-url = Enter a search like GET /Patient?name=smith — the resource type comes from the path.

queries-conditions = Conditions
queries-add-condition = Add condition
queries-includes = Includes
queries-result-controls = Result controls
queries-remove = Remove
queries-param-placeholder = parameter
queries-value-placeholder = value
queries-results = Results
queries-results-total = { $count } results
queries-results-included = { $count } included
queries-results-empty = No results.
queries-open-tab = Open in new tab
queries-col-updated = Updated
queries-prev = Previous
queries-next = Next

queries-rail-heading = Resource types
queries-rail-filter = Filter types
