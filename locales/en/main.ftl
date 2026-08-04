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
nav-tenants = Tenants
nav-toggle = Collapse or expand the navigation

## Tenant maintenance (/ui/tenants)

tenants-title = Tenant Maintenance
tenants-unavailable = The tenant registry is not available on this storage backend.
tenants-stat-total = Total tenants
tenants-stat-total-sub = { $count ->
    [one] { $count } registered
   *[other] { $count } registered
}
tenants-stat-resources = Resources stored
tenants-stat-resources-sub = across all tenants
tenants-search-placeholder = Search by name or tenant id…
tenants-add = Add tenant
tenants-add-title = Add a tenant
tenants-field-id = Tenant id
tenants-field-id-hint = Used in the API (X-Tenant-ID header, URL prefix, JWT claim).
tenants-field-name = Display name (optional)
tenants-field-name-hint = A human-friendly label; not used for routing.
tenants-add-submit = Provision tenant
tenants-col-tenant = Tenant
tenants-col-resources = Resources
tenants-col-created = Created
tenants-col-actions = Actions
tenants-empty = No tenants match.
tenants-unregistered = unregistered
tenants-delete = Delete tenant
tenants-delete-confirm = Deregister tenant "{ $id }"? Its stored data is kept unless purged via the API.

tenant-heading = Tenants
tenant-all = All tenants
tenant-search-placeholder = Search tenants

theme-label = Theme
theme-light = Light theme
theme-dark = Dark theme

fhir-version = FHIR { $version }
fhir-version-heading = FHIR version

card-resource-types = Resource types
card-resource-types-sub = enabled for { $version }
card-stored-resources = Stored resources
card-stored-resources-sub = across active tenant
card-export-jobs = Export jobs
card-export-jobs-sub = running ({ $queued } queued)
card-uptime = Uptime
card-uptime-sub = last 30 days

chart-title = FHIR resources over time
chart-expand = Expand chart
chart-window = Chart time window

## Footer

footer-copyright = © { $year } { -org-name }

## History & Versions (#236)

history-heading = History & Versions
history-lede = Compare two versions of a resource. Storage is fully versioned; this reads it through the ordinary _history and vread API.
history-type-label = Resource type
history-id-label = Resource id
history-id-placeholder = resource id
history-load = Load
history-tabs-label = History scope
history-tab-instance = Instance
history-tab-type = Type feed
history-tab-system = System feed
history-versions-label = Versions
history-pick-instance = Pick an instance
history-current = current
history-from = From
history-to = To
history-show-metadata = Show metadata changes
history-empty = Load a resource, then pick two versions to compare.
history-load-error = Could not load that resource's history.
history-not-found = No history for that resource — check the type and id.
history-diff-heading = { $from }
history-metadata-hidden = { $count ->
    [one] { $count } metadata change hidden
   *[other] { $count } metadata changes hidden
}
history-textual = Show full text diff
history-only-metadata = Only metadata changed between these versions.
history-identical = These two versions are identical.
history-deleted = { $version } is a deletion — there is nothing to diff against.
history-parse-error = Those versions could not be read as JSON.
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
queries-match-is = is
queries-or = + or
plain-pill = In plain English
plain-find = Find {"{type}"} records
plain-clause = {"{path}"} {"{verb}"} {"{value}"}
plain-and = and
plain-or = or
plain-arrow = ’s
plain-has = that have a related {"{type}"} whose {"{param}"} {"{verb}"} {"{value}"}
plain-include = Also returning the {"{param}"} of each {"{type}"}{"{target}"}
plain-revinclude = Plus every {"{type}"} whose {"{param}"} points here
plain-iterate = (repeatedly)
plain-count = Showing {"{n}"} per page
plain-sort = Sorted by {"{sort}"}
plain-verb-is = is
plain-verb-contains = contains
plain-verb-exact = is exactly
plain-verb-missing = is present/absent
plain-verb-not = is not
plain-verb-text = matches the text
plain-verb-in = is in the value set
plain-verb-not-in = is not in the value set
plain-verb-identifier = has the identifier
plain-verb-of-type = has an identifier of type
plain-verb-ge = is on or after
plain-verb-le = is on or before
plain-verb-gt = is after
plain-verb-lt = is before
plain-verb-ne = is not
plain-verb-eq = is
plain-verb-sa = starts after
plain-verb-eb = ends before
plain-verb-ap = is approximately
queries-related-heading = Include related data
queries-related-sub = Adds connected resources to the results.
queries-related-add-include = Include a resource that points to
queries-related-add-revinclude = Include resources that point here
queries-iterate = Iterate
queries-sort-label = Sort
queries-sort-default = Default
queries-sort-recent = Most recent
queries-sort-oldest = Oldest
queries-sort-id = ID
queries-modify-heading = Modifiers
queries-mod-exact = whole value incl. case & accents
queries-mod-contains = match anywhere in the text
queries-mod-missing = field is present / absent
queries-mod-text = advanced text handling
queries-mod-not = none of the values match
queries-mod-above = this or an ancestor
queries-mod-below = this or a descendant
queries-mod-in = member of the value set
queries-mod-not-in = not a member of the value set
queries-mod-identifier = match the reference by identifier
queries-mod-of-type = match identifier type, system and value
queries-chain-into = Filter by a property of the referenced resource
queries-chain-any-target = any
queries-has-pill = has a related
queries-has-type-placeholder = resource type
queries-has-via = linked via
queries-has-where = where its
queries-add-has = ⧉ Filter a resource that links here
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

## Search — natural language & visual builder (#255)

search-heading = Search
search-lede = Describe what you're looking for, or build the query by hand. Either way you get a FHIR search query you can read, correct, and run.
search-query-tag = QUERY
search-copy = Copy the query

search-mode-label = How to write the query
search-mode-nl = Natural language
search-mode-builder = Visual builder

search-nl-label = Describe the search
search-nl-placeholder = Describe what you're looking for — e.g. patients named Smith born after 1980
search-nl-hint = Your text and this server's search parameters go to the language model. Patient data never does. The query it writes is shown below for you to check and run.
search-nl-working = Translating…
search-nl-caveats = Worth knowing:
search-nl-unsupported = That isn't a search this server can run. Try describing the records you want to find.

search-nl-example-1 = Female patients over 65 with a diabetes diagnosis
search-nl-example-2 = Observations from the last 30 days, most recent first
search-nl-example-3 = Encounters at Boston General still in progress

search-setup-heading = Natural-language search is available
search-setup-body = Turn plain-language descriptions into FHIR search queries. It needs an API key for a language model — the server reads it from the environment, and it never reaches this page. Until one is set, use the visual builder below.
search-setup-key-placeholder = your API key
search-setup-disable = To remove the feature entirely — endpoint, page, and this notice — set HFS_NL_SEARCH_ENABLED=false.
search-setup-docs = Read the how-to

## Resource editor (#264)

editor-heading = Resource editor
editor-lede = Edit a resource against its schema: add any element the schema allows, at any depth — including extensions, on any node that accepts one.
editor-title = Edit resource
editor-view-label = How to edit
editor-view-form = Guided form
editor-view-json = JSON
editor-save = Save changes
editor-delete = Delete
editor-remove = Remove this node
editor-saved = Saved.
editor-load-error = Could not load that resource.
editor-confirm-delete = Delete this resource? This cannot be undone.
editor-invalid-json = That is not valid JSON, so it cannot be edited as a form. Your text is untouched.
editor-source-hint = Edit the source directly. Switching back to the guided form parses it.

editor-add = Add element
editor-must-support-badge = MS
editor-binding-hint = Bound to a value set — codes come from it; strength shown
editor-legend-live = Checked as you type: structure, cardinality, required bindings
editor-legend-save = Checked on save: constraints and terminology
editor-deferred-badge = on save
editor-deferred-hint = Codes are verified against the value set when you save (and live in the picker where a terminology server is configured)
editor-must-support-hint = Must-support: consumers of this profile are expected to handle this element
editor-add-filter = Filter elements
editor-add-another = add another
editor-pick-type = Pick a type…
editor-extension-url = Extension URL
editor-add-extension = Add extension

editor-valid = No issues.
editor-issues = { $count ->
    [one] { $count } issue
   *[other] { $count } issues
}

editor-modifier-badge = modifier
editor-modifier-warning = A modifier extension changes the meaning of this resource. A system that does not recognise it must refuse to process the resource.
editor-unknown-badge = not in schema
editor-unknown-hint = The schema does not describe this element. It is shown so it is not silently lost, and it is kept on save.

editor-primitive-extension-badge = + extension
editor-primitive-extension-hint = This value carries extensions of its own (a `_` sibling in the JSON). They are kept when you save.

editor-collapse-all = Collapse all
editor-expand-all = Expand all
editor-edit-raw = Edit raw
editor-versions = Versions
editor-versions-none = No prior versions.

## Resources workspace (#282)

resources-heading = Resources
resources-lede = Browse, search, create, and edit FHIR resources. Search in natural language or build the query by hand, then open any result to edit it.
resources-create = Create new
resources-save-blocked = Fix the validation issues before saving.
resources-save-invalid = The JSON is not valid — fix it before saving.
resources-edit-title = Edit resource
resources-tab-edit = Edit
resources-tab-history = History
resources-types-heading = Resource types

queries-saved-group = Saved

nav-collapse = Collapse menu

batch-heading = Batch / Transaction
batch-lede = Upload a FHIR Bundle, review the actions it will run, execute it against this server, and read the outcome of every entry.
batch-upload = Upload
batch-drop-hint = Drop a bundle JSON file here
batch-drop-browse = or click to browse
batch-invalid-json = That file is not valid JSON
batch-not-a-bundle = That JSON is not a FHIR Bundle
batch-bad-type = Only Bundles of type batch or transaction can be executed here
batch-request = Request
batch-entries = entries
batch-semantics-batch = Batch: entries run independently — a failed entry does not stop or undo the others.
batch-semantics-transaction = Transaction: all or nothing — if any entry fails, the server rolls the whole bundle back.
batch-tab-actions = Actions
batch-tab-json = Bundle JSON
batch-no-body = (no body — this entry only addresses a resource)
batch-cancel = Cancel
batch-upload-another = Upload another
batch-execute = Execute
batch-response-heading = Per-action outcomes
batch-sum-created = created
batch-sum-updated = updated
batch-sum-other = read/other
batch-sum-failed = failed
batch-request-failed = The request failed
batch-back = Back to bundle
batch-execute-again = Execute again
