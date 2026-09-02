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
nav-terminology-new-window = Terminology (opens in a new tab)
nav-resources = Resources
nav-settings = Settings
nav-signout = Sign Out

## Language switcher

language-label = Language
language-en = English
language-es = Spanish
language-de = German
user-menu-label = Account menu
user-anonymous = Anonymous user
user-local-hint = Authentication is disabled
user-logout = Sign out

## Home / landing page

home-lede = Server-rendered, HTMX-first UI. This panel is refreshed as an HTML fragment.

## Status panel

status-last-checked = Last checked: { $timestamp }

## Dashboard / health

dashboard-heading = Server Dashboard
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

terminology-heading = Terminology Server
terminology-lede = Connect HFS to a FHIR terminology server.
terminology-configured-heading = Terminology Server Configured
terminology-configured-body = HFS_TERMINOLOGY_SERVER points to a valid server URL.
terminology-configured-open = Open Terminology Server
terminology-invalid-heading = HFS_TERMINOLOGY_SERVER is invalid.
terminology-invalid-body = Use an absolute HTTP or HTTPS URL with a host. Paths and a trailing slash are allowed. Do not include credentials, a query string, or a fragment.
terminology-invalid-note = Update the environment variable, then restart HFS.
terminology-setup-heading = Connect a Terminology Server
terminology-setup-body = Set HFS_TERMINOLOGY_SERVER to the base URL of the FHIR terminology server that HFS should use.
terminology-setup-note = Set the variable in the environment that starts HFS, then restart the server.
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
error-generic = Something went wrong. Try again.

## Dashboard shell (Figma "Dashboard V1.1")

nav-section-work = Work
nav-section-batch-data = Batch & Data
nav-section-sql-on-fhir = SQL on FHIR
nav-section-server = Server
nav-section-tools = Tools

nav-home = Home
nav-search = Search
nav-resource-editor = Resource Editor
nav-history-versions = History & Versions
nav-compartments = Compartments
nav-batch-transaction = Batch / Transaction
nav-import = Import
nav-export = Export
nav-sql-view-definitions = View Definitions
nav-sql-queries = SQL Queries
nav-sql-views = SQL Views
nav-sql-export = SQL Export
nav-sql-files = Files
nav-capability-conformance = Capability & Conformance
nav-search-parameters = Search Parameters
nav-subscriptions = Subscriptions
nav-tenants = Tenants

## Tenant maintenance (/ui/tenants)

tenants-title = Tenant Maintenance
tenants-lede = Provision, inspect, and delete the tenants this server isolates data between.
tenants-unavailable = The tenant registry is not available on this storage backend.
tenants-stat-total = Total tenants
tenants-stat-total-sub = { $count ->
    [one] { $count } registered
   *[other] { $count } registered
}
tenants-stat-resources = Resources stored
tenants-stat-resources-sub = across all tenants
tenants-search-placeholder = Search by name or tenant id…
tenants-add = Add Tenant
tenants-add-title = Add a Tenant
tenants-field-id = Tenant id
tenants-field-id-hint = Used in the API (X-Tenant-ID header, URL prefix, JWT claim).
tenants-field-name = Display name (optional)
tenants-field-name-hint = A human-friendly label; not used for routing.
tenants-add-submit = Provision Tenant
tenants-col-tenant = Tenant
tenants-col-resources = Resources
tenants-col-created = Created
tenants-col-actions = Actions
tenants-empty = No tenants match.
tenants-unregistered = unregistered
tenants-delete = Delete Tenant
tenants-delete-confirm = Deregister tenant "{ $id }"? Its stored data is kept unless purged via the API.
tenants-row-provisioning = Provisioning… this may take a moment.
tenants-row-failed = Could not provision the tenant.
tenants-dismiss = Dismiss

tenant-heading = Tenants
tenant-all = All tenants
tenant-search-placeholder = Search tenants

theme-label = Theme
theme-light = Light theme
theme-dark = Dark theme

fhir-version = FHIR { $version }
fhir-version-heading = FHIR Version

card-resource-types = Resource Types
card-resource-types-sub = used for { $version }
card-stored-resources = Stored Resources
card-stored-resources-sub = across active tenant
card-export-jobs = Export Jobs
card-export-jobs-sub = running ({ $queued } queued)
card-import-jobs = Import Jobs
card-import-jobs-sub = active
card-jobs-unavailable = unavailable
card-uptime = Uptime
card-uptime-sub = since process start

chart-title = FHIR Resources over Time
chart-window = Chart time window
chart-pick-heading = Charted Resource Types
chart-pick-all = View all resource types
chart-pick-filter = Filter types
chart-empty = Nothing to chart yet — stored resources appear here as they are created.
chart-sample-note = Sample data: no live metrics provider is registered on this build.
chart-table-toggle = View as Table
chart-table-when = Time
chart-focus-series = Focus this series
chart-unfocus-series = Show all series

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
history-tab-type = Type Feed
history-tab-system = System Feed
history-versions-label = Versions
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

queries-heading = Saved Queries
queries-lede = Keep FHIR search queries per resource type, sorted by when you last ran them. Saved to your user settings, so they roam across devices.
queries-add-heading = Save a Query
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

sp-heading = Search Parameters
sp-lede = Browse the parameters this server resolves searches against, filtered by base resource type. Stored parameters can be created, edited, and deleted; the registry picks changes up per tenant.
sp-version-label = FHIR version
sp-spec-missing = The full spec bundle (search-parameters-*.json) was not found in the data directory — only the minimal embedded fallback parameters are shown.
sp-rail-label = Resource filter
sp-rail-search = Filter types
sp-rail-recent = Recently used
sp-rail-types = Resource types
sp-rail-all = All Types
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
sp-new = New Search Parameter
sp-edit = Edit
sp-delete = Delete
sp-delete-confirm = Delete this stored search parameter? Searches that use it stop matching once the registry refreshes.
cmp-new = New Compartment Definition
cmp-edit = Edit
cmp-delete = Delete
cmp-delete-confirm = Delete this compartment definition? Its compartment routes stop resolving.
crud-delete-failed = Could not delete this item.

## Compartment viewer & tester (#237)

cmp-heading = Compartments
cmp-lede = The compartment definitions this server routes /{"{"}compartment{"}"}/{"{"}id{"}"}/{"{"}type{"}"} requests with, and a tester that answers: is this type in this compartment, via which parameters, and what search does the server run?
cmp-rail-label = Compartment definitions
cmp-rail-heading = Compartments
cmp-degraded = Compartment definitions could not be loaded from this server right now — the self-call to /CompartmentDefinition failed (with authentication enabled this usually means the outbound service token is missing or invalid). The page retries on the next request.
cmp-rail-note = Definitions are stored resources, seeded from the FHIR spec at startup. Edits and deletions here are tenant-scoped.
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
cmp-filter-all = All Types
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
queries-builder-heading = Search Builder
queries-url-label = FHIR search URL
queries-url-placeholder = GET /Patient?name=smith&birthdate=ge1980-01-01
queries-builder-hint = Edit the GET URL directly or through the rows below — they stay in sync. Run executes the search here and records it under Recent; give it a name to keep it in the saved list.
queries-recent = Recent
queries-recent-heading = Recent Searches
queries-recent-empty = No recent searches yet — Run one to record it here.
queries-invalid-url = Enter a search like GET /Patient?name=smith — the resource type comes from the path.
queries-invalid-fhir-escape = This query contains an invalid FHIR escape. Correct the escaped value before editing it visually.

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
plain-clause-no-value = {"{path}"} {"{verb}"}
plain-and = and
plain-or = or
plain-arrow = ’s
plain-has = that have a related {"{type}"} whose {"{param}"} {"{verb}"} {"{value}"}
plain-has-no-value = that have a related {"{type}"} whose {"{param}"} {"{verb}"}
plain-include = Also returning the {"{param}"} of each {"{type}"}{"{target}"}
plain-revinclude = Plus every {"{type}"} whose {"{param}"} points here
plain-iterate = (repeatedly)
plain-count = Showing {"{n}"} per page
plain-sort = Sorted by {"{sort}"}
plain-verb-is = is
plain-verb-contains = contains
plain-verb-exact = is exactly
plain-verb-missing = is present/absent
plain-verb-missing-true = is absent
plain-verb-missing-false = is present
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
queries-related-heading = Include Related Data
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
queries-open-tab = Open in New Tab
queries-col-updated = Updated
queries-prev = Previous
queries-next = Next
queries-results-fetch-error = Could not load results from { $origin }. Check HFS_BASE_URL and try again.

queries-rail-heading = Resource Types
queries-rail-filter = Filter types

## Search — natural language & visual builder (#255)

search-heading = Search
search-lede = Describe what you're looking for, or build the query by hand. Either way you get a FHIR search query you can read, correct, and run.
search-query-tag = QUERY
search-copy = Copy the Query

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

editor-heading = Resource Editor
editor-lede = Edit a resource against its schema: add any element the schema allows, at any depth — including extensions, on any node that accepts one.
editor-title = Edit Resource
editor-view-label = How to edit
editor-view-form = Guided form
editor-view-json = JSON
editor-save = Save Changes
editor-delete = Delete
editor-remove = Remove This Node
editor-saved = Saved.
editor-load-error = Could not load that resource.
editor-confirm-delete = Delete this resource? This cannot be undone.
editor-invalid-json = That is not valid JSON, so it cannot be edited as a form. Your text is untouched.
editor-source-hint = Edit the source directly. Switching back to the guided form parses it.

editor-add = Add Element
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
json-view-toggle-fold = Toggle JSON section
editor-edit-raw = Edit raw
editor-versions = Versions
editor-versions-none = No prior versions.

## Resources workspace (#282)

resources-heading = Resources
resources-lede = Browse, search, create, and edit FHIR resources. Search in natural language or build the query by hand, then open any result to edit it.
resources-create = Create new resource
resources-create-typed = Create new { $type }
resources-create-invalid-type = This resource type is not available in the selected FHIR version. Correct the query or choose a type from the list.
resources-create-not-advertised = This server does not allow creating this resource type. You can still search it.
resources-create-schema-unavailable = This resource type has no editor schema in the selected FHIR version, so the UI cannot create it safely.
resources-create-metadata-unavailable = Server capabilities are unavailable. Creation stays disabled until the UI can verify them.
resources-save-blocked = Fix the validation issues before saving.
resources-save-invalid = The JSON is not valid — fix it before saving.
resources-edit-title = Edit Resource
resources-tab-edit = Edit
resources-tab-history = History
resources-types-heading = Resource Types
rail-all-types-heading = All Types

queries-saved-group = Saved

nav-collapse = Collapse Menu

batch-heading = Batch / Transaction
batch-lede = Upload a FHIR Bundle, review the actions it will run, execute it against this server, and read the outcome of every entry.
batch-upload = Upload
batch-drop-hint = Drop a bundle JSON file here
batch-drop-browse = or click to browse
batch-invalid-json = That file is not valid JSON.
batch-not-a-bundle = That JSON is not a FHIR Bundle.
batch-bad-type = Only Bundles of type batch or transaction can be executed here.
batch-request = Request
batch-entries = entries
batch-semantics-batch = Batch: entries run independently — a failed entry does not stop or undo the others.
batch-semantics-transaction = Transaction: all or nothing — if any entry fails, the server rolls the whole bundle back.
batch-tab-actions = Actions
batch-tab-json = Bundle JSON
batch-no-body = (no body — this entry only addresses a resource)
batch-cancel = Cancel
batch-execute = Execute
batch-plan-heading = Execution Plan
batch-done = Done
batch-response-heading = Per-Action Outcomes
batch-sum-created = created
batch-sum-updated = updated
batch-sum-other = read/other
batch-sum-failed = failed
batch-request-failed = The request failed.
batch-reading = Reading bundle…
batch-executing = Executing…
batch-read-failed = The file could not be read.

## Bulk Import workspace (#527)

bulk-import-title = Bulk Import
bulk-import-lede = Send precoordinated FHIR data sets to a Data Recipient with the Bulk Data $bulk-submit operation.
bulk-import-detail-lede = The manifests, status, and run log of this submission.
bulk-import-new = New Submission
bulk-import-create-title = Create Bulk Submission
bulk-import-field-name = Submission name
bulk-import-auth = Authentication
bulk-import-auth-hint = How to authenticate to the recipient server.
bulk-import-auth-none = None
bulk-import-auth-none-hint = No authorization header will be sent.
bulk-import-auth-backend = Backend services authentication
bulk-import-auth-backend-hint = Obtains an access token and sends it as a Bearer token in the authorization header.
bulk-import-field-client-id = Client ID
bulk-import-field-client-id-hint = Register this data provider with the Data Recipient and get back a client ID.
bulk-import-field-token-url = Token URL
bulk-import-field-token-url-hint = Authorization server's token endpoint URL.
bulk-import-jwks-hint = Register this server's public key with the recipient using the JWKS URL:
bulk-import-test-auth = Test authentication
bulk-import-test-auth-ok = Authentication succeeded.
bulk-import-create-submit = Submit
bulk-import-advanced = Advanced options
bulk-import-unavailable = The storage backend does not host the settings store, so submissions cannot be saved.
bulk-import-submissions = Submissions
bulk-import-records = records
bulk-import-col-name = Name
bulk-import-col-status = Status
bulk-import-col-created = Created
bulk-import-col-destination = Destination
bulk-import-empty = No submissions yet. Create one to get started.
bulk-import-all = All Submissions
bulk-import-status-not-started = Not Started
bulk-import-status-in-progress = In Progress
bulk-import-status-stopped = Stopped
bulk-import-status-completed = Completed
bulk-import-status-failed = Failed
bulk-import-detail-recipient = Data Recipient
bulk-import-detail-id = Submission ID
bulk-import-detail-submitter = Submitter
bulk-import-detail-created = Created
bulk-import-detail-status = Status
bulk-import-detail-auth = Authentication
bulk-import-abort = Abort
bulk-import-delete = Delete
bulk-import-edit = Edit
bulk-import-edit-title = Edit Submission
bulk-import-edit-submit = Save Changes
bulk-import-field-manifest-url = Manifest URL
bulk-import-field-manifest-url-hint = URL pointing to a Bulk Export Manifest with a precoordinated FHIR data set.
bulk-import-field-output-format = Format
bulk-import-field-output-format-hint = The format for the Bulk Data files in the manifest.
bulk-import-field-headers = File request headers
bulk-import-field-headers-hint = HTTP headers the Data Recipient should use when requesting a data file, one "Name: value" per line.
bulk-import-log = Submission Log
bulk-import-log-empty = Nothing submitted yet.
bulk-import-field-submitter-system = Submitter system
bulk-import-field-submitter-value = Submitter value
bulk-import-field-submitter-hint = Must match an identifier registered with the Data Recipient (coordinated out-of-band). Leave empty to use the generated defaults.
bulk-import-processing = Processing
bulk-import-processing-waiting = Waiting for the recipient's first status report…
bulk-import-result = Result
bulk-import-result-finished = Processing finished at
bulk-import-result-outputs = Output files
bulk-import-result-errors = Error files
ui-cancel = Cancel
ui-close = Close
ui-combobox-selected-label = Selected items
ui-combobox-remove = Remove
ui-combobox-added = Added
ui-combobox-removed = Removed
ui-combobox-loading = Loading suggestions…
ui-combobox-results-updated = Suggestions available:
ui-combobox-error = Suggestions could not be loaded. Try again.
editor-orphans-title = These issues have no field yet — add the elements to fix them
editor-hint-date = FHIR date: YYYY, YYYY-MM, or YYYY-MM-DD
editor-hint-datetime = FHIR dateTime: YYYY, YYYY-MM, YYYY-MM-DD, or a full timestamp with timezone (2024-05-17T14:30:00+02:00)
editor-hint-time = FHIR time: HH:MM:SS
editor-hint-instant = FHIR instant: full timestamp with timezone, e.g. 2024-05-17T14:30:00.000Z

## Subscriptions operator page (#580)

subs-title = Subscriptions
subs-lede = Read-only view of the subscriptions engine: every registered subscription, its channel, live status, and delivery counters.
subs-unavailable = The subscriptions engine is not enabled on this server.
subs-unavailable-how = Turn it on by starting HFS with:
subs-empty = No subscriptions are registered for this tenant.
subs-card-failing = Failing
subs-card-failing-sub = Needs attention
subs-card-idle = Idle
subs-card-idle-sub = No clients
subs-card-active = Active
subs-card-active-sub = delivering
subs-card-delivered = Delivered in 24 h
subs-card-delivered-sub = { $rate }% first try
subs-card-delivered-none = no deliveries in the window
subs-table-heading = Subscriptions
subs-sort = Sort
subs-sort-status = Status
subs-sort-sent = Most sent
subs-sort-fails = Fail streak
subs-col-subscription = Subscription
subs-col-channel = Channel
subs-col-status = Status
subs-col-last24 = Last 24 hrs
subs-col-sent = Sent
subs-col-fails = Fail streak
subs-state-active = Active
subs-state-error = Error
subs-state-idle = 0 clients
subs-state-requested = Requested
subs-state-off = Off

## Bulk Export workspace (#537)

bulk-export-title = Bulk Export
bulk-export-lede = Pull data out of this server as NDJSON files with the FHIR Bulk Data $export operation.
bulk-export-active-title = Exports
bulk-export-new = New Export
bulk-export-unavailable = The storage backend does not host the settings store, so export jobs cannot be tracked.
bulk-export-scope = What are you exporting?
bulk-export-scope-system = Everything
bulk-export-scope-system-hint = The whole server — every resource type you select below.
bulk-export-scope-patient = Patients
bulk-export-scope-patient-hint = Every patient and the records that belong to them. Nothing patient-unrelated.
bulk-export-scope-group = Group
bulk-export-scope-group-hint = Just the members of a cohort you've already defined.
bulk-export-field-group-id = Group ID
bulk-export-field-group-id-hint = Required for the Group scope: the id of the FHIR Group to export.
bulk-export-field-patients = Patients
bulk-export-field-patients-placeholder = Search name, surname, or exact FHIR ID
bulk-export-field-patients-hint = Search by the beginning of a given name or family name, or enter an exact logical FHIR ID such as Patient/p-104. Leave empty to export every patient.
bulk-export-field-patients-fallback-placeholder = Patient/p-104, Patient/p-205
bulk-export-field-patients-fallback-hint = Enter exact logical FHIR IDs separated by commas or new lines. Leave empty to export every patient.
bulk-export-field-patients-id-only-hint = Search by exact logical FHIR ID, such as Patient/p-104. Leave empty to export every patient.
bulk-export-field-patients-id-only-placeholder = Search exact FHIR ID
bulk-export-patient-options-empty = No matching patients found.
bulk-export-patient-invalid = Enter only valid logical Patient IDs, separated by commas or new lines.
bulk-export-field-name = Name
bulk-export-field-name-placeholder = Diabetes registry 2024
bulk-export-types = Resource types
bulk-export-all-resources = All Resources
bulk-export-narrow = Narrow it down
bulk-export-field-elements = FHIR elements
bulk-export-field-type-filter = Type filter
bulk-export-field-since = Since
bulk-export-since-all = All time
bulk-export-since-day = Last day
bulk-export-since-week = Last 7 days
bulk-export-since-month = Last 4 weeks
bulk-export-since-custom = Custom
bulk-export-field-since-custom = Custom instant
bulk-export-field-since-custom-hint = Used when Since is Custom. RFC 3339, e.g. 2026-08-01T00:00:00Z.
bulk-export-start = Start Export
bulk-export-running = running
bulk-export-clear = Clear
bulk-export-files-word = files
bulk-export-exports-word = exports
bulk-export-none = No exports yet. Use New Export to start one.
bulk-export-status-in-progress = In progress
bulk-export-status-complete = Complete
bulk-export-status-failed = Failed
bulk-export-status-cancelled = Cancelled
bulk-export-progress = Progress
bulk-export-progress-waiting = Waiting for the first status report…
bulk-export-files = Files
bulk-export-finished-in = finished in
bulk-export-error = Error
bulk-export-cancel = Cancel
bulk-export-retry = Retry
bulk-export-download-all = Download All Resources
bulk-export-download-all-aria = Download all resources from { $name }
bulk-export-delete = Delete
bulk-export-delete-aria = Delete export { $name }
bulk-export-delete-warning = Delete { $name } and its output files from the server? This cannot be undone.
bulk-export-delete-confirm = Delete export
bulk-export-delete-cancel = Keep export
bulk-export-delete-error = The export could not be deleted safely. Its card was kept so you can try again.

# CapabilityStatement page (#653)
cap-title = Capability Statement
cap-lede = What this server does right now, for the selected tenant and FHIR version — composed live from /metadata.
cap-summary-heading = Server Summary
cap-summary-description = Description
cap-summary-url = Base URL
cap-summary-fhir-version = FHIR version
cap-summary-status = Status
cap-summary-kind = Kind
cap-summary-date = Date
cap-summary-formats = Formats
cap-interactions-heading = System Interactions
cap-transaction-note = transaction is advertised because the active backend supports atomic transactions; batch is always available.
cap-role-matrix = View the backend role matrix.
cap-operations-heading = Operations
cap-col-operation = Operation
cap-col-definition = Definition
cap-resources-heading = Per-Resource Capabilities
cap-filter-placeholder = Filter types…
cap-col-type = Type
cap-col-interactions = Interactions
cap-col-search-params = Search params
cap-col-includes = Includes
cap-col-revincludes = Revincludes
cap-resources-empty = No resource types match the filter.
cap-raw-toggle = Raw CapabilityStatement (JSON)
cap-raw-load = Open plain JSON
cap-raw-loading = Loading highlighted JSON…
cap-raw-plain = Plain JSON fallback. Highlighting is loaded incrementally when JavaScript is available.
cap-json-load-error = This JSON section could not be loaded. Collapse and reopen it to retry.
cap-json-truncated = display truncated
cap-json-path-too-deep = maximum expansion depth reached
cap-json-pagination-label = JSON items
cap-json-page-prev = Previous
cap-json-page-next = Next
cap-unavailable = The CapabilityStatement could not be fetched from the server — the self-call may need an outbound token when authentication is enabled.

## SQL on FHIR section stubs (#649)


sql-vd-title = View Definitions
sql-vd-lede = Author and manage the ViewDefinitions that SQL on FHIR runs flatten resources with.

sql-queries-title = SQL Queries
sql-queries-lede = Run SQL on FHIR queries against this server.

sql-views-title = SQL Views
sql-views-lede = Reusable SQL views layered over ViewDefinitions.

sql-export-title = SQL Export
sql-export-lede = Long-running SQL on FHIR export jobs.

sql-files-title = Files
sql-files-lede = Manifests and output files produced by SQL exports.

## View Definitions workspace (#649)

vd-new = Create New
vd-new-title = New View Definition
vd-rail-label = View definitions
vd-rail-heading = View Definitions
vd-rail-all-heading = All View Definitions
vd-filter = Filter views
vd-none = No view definitions yet.
vd-empty-lede = Create your first ViewDefinition with Create New.
vd-degraded = The view definition list could not be loaded.
vd-saved = Saved.
vd-run = Run
vd-run-failed = Could not run the view.
vd-save = Save
vd-duplicate = Duplicate
vd-delete = Delete
vd-delete-confirm = Delete view definition "{ $name }"? This cannot be undone.
vd-delete-failed = Could not delete the view definition.
vd-json-heading = Definition (JSON)
vd-run-hint = Runs as you type — results follow the current definition, saved or not
vd-results-heading = Results
vd-results-empty = The view produced no rows.
# #752 ticket 01: the $sql-run preview's row/duration meta, shared by the
# page's own render and the `/run` fragment endpoint.
vd-results-meta = { $rows } rows · { $ms } ms
# #752 ticket 01: the results meta after a failed run — the previous table
# stays on screen, relabelled.
vd-results-stale = last successful run
vd-pagination-label = View definition pages
vd-page-prev = Previous
vd-page-next = Next

## SQL Queries / SQL Views workspaces (#649)

sql-queries-new-title = New SQL Query
sql-views-new-title = New SQL View
sql-queries-rail-all-heading = All SQL Queries
sql-views-rail-all-heading = All SQL Views
lib-filter = Filter libraries
lib-none = No libraries yet.
lib-empty-lede = Create your first library with Create New.
lib-degraded = The library list could not be loaded.
lib-sql-heading = SQL
lib-delete-confirm = Delete "{ $name }"? This cannot be undone.
lib-delete-failed = Could not delete the library.

## SQL Export and Files pages (#649)

export-start-failed = Could not start the export.
export-started = Export started.
export-cancelled = Cancellation requested.
export-job-heading = Export Job
export-job-id = Job id
export-job-state = State
export-state-running = Running
export-state-done = Finished
export-state-unknown = This job is unknown — it may have been cancelled or reclaimed.
export-refresh = Refresh
export-cancel = Cancel Job
export-view-files = View Files
export-new-heading = New Export
export-no-subjects = Nothing to export yet — create a ViewDefinition first.
export-format = Output format
export-start = Start Export
files-job-heading = Export Job
files-load = Load Manifest
files-error = Could not load the manifest.
files-outputs-heading = Outputs
files-col-output = Output
files-col-downloads = Downloads
files-shard = File { $n }
files-empty = The job produced no output files.

## HTS administrative UI (crates/hts-ui) — Phase 1 scaffold stubs
##
## Keys for the HTS UI follow the convention hts-<page>-<role>-<control>. These stubs cover the base layout, sidebar
## nav, and the dashboard scaffold placeholder rendered by the Phase 1 blocker
## slice. They must be kept in parity with es/de/main.ftl.

-hts-app-name = Helios Terminology Server
hts-app-title = { -hts-app-name }

hts-nav-section-work = Terminology
hts-nav-section-tools = Tools
hts-nav-section-server = Server
hts-nav-home = Home
hts-nav-code-systems = Code Systems
hts-nav-value-sets = Value Sets
hts-nav-concept-maps = Concept Maps
hts-nav-operations = Operations
hts-nav-import = Import

hts-fhir-version-heading = FHIR version
hts-fhir-version = FHIR { $version }

hts-home-title = Home
hts-home-subtitle = Terminology server health, catalog inventory, and quick actions.

## Dashboard rows (row headings are visually hidden — they're for screen readers).

hts-home-row-status = Server status

## Dashboard tiles.

hts-home-tile-status = Status
hts-home-tile-uptime = Uptime
hts-home-tile-loaded-systems = Loaded code systems
hts-home-tile-loaded-systems-hint = From TerminologyCapabilities.codeSystem[]
hts-home-tile-requests = Requests
hts-home-tile-metrics-hint = Since server start

## Home request-rate chart (design doc §7.1). Plots a rate differenced from
## the cumulative `/metrics` counters, sampled only while this page is open,
## so several honest "nothing to draw" states need their own copy.

hts-home-chart-title = Requests per minute
hts-home-chart-window = Chart time window
hts-home-chart-series = Status class
hts-home-chart-window-15m = 15m
hts-home-chart-window-1h = 1h
hts-home-chart-window-6h = 6h
hts-home-chart-series-all = All
hts-home-chart-series-2xx = 2xx
hts-home-chart-series-4xx = 4xx
hts-home-chart-series-5xx = 5xx
hts-home-chart-empty-unreachable = /metrics is unreachable — no new samples are arriving.
hts-home-chart-empty-none = No samples collected yet.
hts-home-chart-empty-first = Collecting the first interval — a rate needs two samples.
hts-home-chart-empty-window = No samples in this window. Sampling only runs while this page is open.
hts-home-chart-axis-now = now
hts-home-chart-axis-minutes = -{ $n }m
hts-home-chart-axis-hours = -{ $n }h

## /health `status` values, keyed for translation.

hts-home-status-ok = OK

## Degraded banner (design doc §7 header contract).

hts-degraded-title = Terminology backend not fully available
hts-degraded-body = Some tiles are hidden until HTS becomes reachable again. Interactive controls are disabled on affected pages.
hts-degraded-reason-client-build = Failed to build the upstream HTTP client.
hts-degraded-reason-upstream-down = Could not reach the terminology server.
hts-degraded-reason-upstream-timeout = The terminology server did not respond in time.
hts-degraded-reason-upstream-error = The terminology server returned an error status.
hts-degraded-reason-upstream-shape = The terminology server returned an unexpected response shape.
hts-degraded-reason-bootstrapping = The terminology server is still loading its bootstrap data.
hts-degraded-reason-unknown = The terminology server is temporarily unavailable.

## Dialect chip (topbar, session-wide displayLanguage / Accept-Language per §7.1).


## OperationOutcome partial (shared, design doc §7 / §11).

hts-outcome-severity = Severity: { $severity }
hts-outcome-request-id = Request id: { $id }
hts-outcome-code-not-found = The requested resource was not found.
hts-outcome-code-invalid = The request was rejected as invalid.
hts-outcome-code-too-costly = The requested operation was rejected as too expensive.
hts-outcome-code-unknown = The server returned an issue the UI does not recognise.
hts-degraded-since = Since { $timestamp }

## HTS Slice B — CodeSystem browser + detail with embedded workbench
## (design doc §7.2 + §7.3). Every key here has a peer in es/de/main.ftl.

## CodeSystem status pills (used by browser rows and detail header).

hts-cs-status-draft = draft
hts-cs-status-active = active
hts-cs-status-retired = retired
hts-cs-status-unknown = unknown

## CS browser page.

hts-cs-browser-title = CodeSystems
hts-cs-browser-subtitle = Browse the terminology server's catalog of CodeSystems and open any row to inspect its metadata and workbench.
hts-cs-browser-filter-legend = Filter CodeSystems
hts-cs-browser-filter-url = Canonical URL
hts-cs-browser-filter-version = Version
hts-cs-browser-filter-name = Name
hts-cs-browser-filter-title = Title
hts-cs-browser-filter-status = Status
hts-cs-browser-filter-reset = Reset
hts-cs-browser-empty = No CodeSystems match these filters.
hts-cs-browser-load-more = Load more
hts-cs-browser-showing-count = Showing { $count ->
    [one] { $count } CodeSystem
   *[other] { $count } CodeSystems
}
hts-cs-browser-table-caption = CodeSystems matching the active filters.
hts-cs-browser-column-url = URL
hts-cs-browser-column-version = Version
hts-cs-browser-column-title = Title
hts-cs-browser-column-status = Status
hts-cs-browser-column-name = Name

## Phase 5 — HTS search-form shared strings (used by CS / VS / CM browsers).

hts-search-rail-label = Search filters
hts-search-rail-heading = Filters
hts-facet-status-any = Any status

## CS detail page.

hts-cs-detail-title = { $name } · CodeSystem
hts-cs-detail-title-fallback = CodeSystem
hts-cs-detail-eyebrow = CodeSystem
hts-cs-detail-section-identity = Identity
hts-cs-detail-section-content = Content
hts-cs-detail-content-mode = Content mode
hts-cs-detail-count = Concept count
hts-cs-detail-publisher = Publisher
hts-cs-detail-jurisdiction = Jurisdiction
hts-cs-detail-supersedes = Supersedes
hts-cs-detail-superseded-by = Superseded by
hts-cs-detail-tabs-label = CodeSystem workbench sections
hts-cs-detail-tab-lookup = Lookup
hts-cs-detail-tab-validate = Validate
hts-cs-detail-tab-subsumes = Subsumes
hts-cs-detail-result-empty = Run the operation to see its result here.

## CS $lookup form + result labels.

hts-cs-lookup-heading = Look up a concept
hts-cs-lookup-code = Code
hts-cs-lookup-version = Version
hts-cs-lookup-display-language = Display language
hts-cs-lookup-display-language-placeholder = e.g. en-GB
hts-cs-lookup-properties-legend = Properties
hts-cs-lookup-designations = Designations
hts-cs-lookup-properties = Properties
hts-cs-lookup-no-match = HTS returned no matching concept.

## CS $validate-code form + result labels.

hts-cs-validate-heading = Validate a code
hts-cs-validate-mode-legend = Input mode
hts-cs-validate-mode-code = Bare code
hts-cs-validate-mode-coding = Coding
hts-cs-validate-code = Code
hts-cs-validate-display = Display
hts-cs-validate-coding-legend = Coding
hts-cs-validate-coding-system = system
hts-cs-validate-coding-code = code
hts-cs-validate-coding-display = display
hts-cs-validate-badge-true = valid
hts-cs-validate-badge-false = invalid
hts-cs-validate-message = Message

## CS $subsumes form + result labels.

hts-cs-subsumes-heading = Test subsumption
hts-cs-subsumes-scoped-system = System (fixed)
hts-cs-subsumes-code-a = Code A
hts-cs-subsumes-code-b = Code B
hts-cs-subsumes-outcome-equivalent = Codes are equivalent.
hts-cs-subsumes-outcome-subsumes = Code A subsumes code B.
hts-cs-subsumes-outcome-subsumed-by = Code A is subsumed by code B.
hts-cs-subsumes-outcome-not-subsumed = Neither code subsumes the other.

## Shared workbench chrome (reused by Slice C/D/E workbenches).

hts-workbench-run = Run
hts-workbench-raw-response = Raw request and response
hts-workbench-copy-url = Request URL

## Additional degraded reason for CS-read 404s (design doc §7.3 states matrix).

hts-degraded-reason-upstream-not-found = The terminology server did not find that resource.

## HTS Slice C — ValueSet browser + detail with embedded $expand workbench
## (design doc §7.4 + §7.4.1). Every key here has a peer in es/de/main.ftl.

## ValueSet status pills.

hts-vs-status-draft = draft
hts-vs-status-active = active
hts-vs-status-retired = retired
hts-vs-status-unknown = unknown

## VS browser page.

hts-vs-browser-title = ValueSets
hts-vs-browser-subtitle = Browse the terminology server's catalog of ValueSets and open any row to inspect its metadata or run an expansion.
hts-vs-browser-filter-legend = Filter ValueSets
hts-vs-browser-filter-url = Canonical URL
hts-vs-browser-filter-version = Version
hts-vs-browser-filter-name = Name
hts-vs-browser-filter-title = Title
hts-vs-browser-filter-status = Status
hts-vs-browser-filter-reset = Reset
hts-vs-browser-empty = No ValueSets match these filters.
hts-vs-browser-load-more = Load more
hts-vs-browser-showing-count = Showing { $count ->
    [one] { $count } ValueSet
   *[other] { $count } ValueSets
}
hts-vs-browser-table-caption = ValueSets matching the active filters.
hts-vs-browser-column-url = URL
hts-vs-browser-column-version = Version
hts-vs-browser-column-title = Title
hts-vs-browser-column-status = Status
hts-vs-browser-column-name = Name

## VS detail page.

hts-vs-detail-title = { $name } · ValueSet
hts-vs-detail-title-fallback = ValueSet
hts-vs-detail-eyebrow = ValueSet
hts-vs-detail-section-identity = Identity
hts-vs-detail-section-governance = Governance
hts-vs-detail-publisher = Publisher
hts-vs-detail-jurisdiction = Jurisdiction
hts-vs-detail-immutable = Immutable
hts-vs-detail-immutable-yes = yes
hts-vs-detail-immutable-no = no
hts-vs-detail-purpose = Purpose
hts-vs-detail-copyright = Copyright
hts-vs-detail-tabs-label = ValueSet workbench sections
hts-vs-detail-tab-expand = Expand
hts-vs-detail-result-empty = Run the operation to see its result here.

## VS $expand form + result labels.

hts-vs-expand-heading = Expand this ValueSet
hts-vs-expand-scoped-valueset = ValueSet (fixed)
hts-vs-expand-filter = Filter
hts-vs-expand-filter-placeholder = code or display text
hts-vs-expand-count = count
hts-vs-expand-offset = offset
hts-vs-expand-display-language = Display language
hts-vs-expand-display-language-placeholder = e.g. en-GB
hts-vs-expand-flags-legend = Flags
hts-vs-expand-active-only = Active concepts only
hts-vs-expand-include-designations = Include designations
hts-vs-expand-mode-legend = Result mode
hts-vs-expand-mode-flat = Flat
hts-vs-expand-mode-tree = Tree
hts-vs-expand-use-supplement-legend = Use supplements
hts-vs-expand-use-supplement-placeholder = canonical URL
hts-vs-expand-advanced-summary = Advanced
hts-vs-expand-date = Date
hts-vs-expand-date-placeholder = ISO 8601 (e.g. 2025-06-01)
hts-vs-expand-property-legend = Properties
hts-vs-expand-property-placeholder = property code
hts-vs-expand-tx-resource-legend = tx-resource
hts-vs-expand-tx-resource-placeholder = canonical URL or reference
hts-vs-expand-system-version-legend = system-version
hts-vs-expand-system-version-placeholder = system|version
hts-vs-expand-check-system-version-legend = check-system-version
hts-vs-expand-force-system-version-legend = force-system-version
hts-vs-expand-default-valueset-version = default-valueset-version
hts-vs-expand-threshold = Too-costly threshold
hts-vs-expand-ceiling-tooltip = UI ceiling: { $ceiling } (values above are dropped)
hts-vs-expand-ceiling-note = ceiling: { $ceiling }
hts-vs-expand-ceiling-warning-title = Threshold above the UI ceiling
hts-vs-expand-ceiling-warning-body = You requested threshold { $requested }, which is above the UI ceiling — the header was not attached.
hts-vs-expand-ceiling-value = ceiling: { $ceiling }
hts-vs-expand-too-costly-title = Expansion rejected as too costly
hts-vs-expand-too-costly-body = HTS refused the expansion above the current threshold. Raise it below and re-run, or narrow the filter.
hts-vs-expand-raise-threshold = Raise threshold to
hts-vs-expand-raise-submit = Retry
hts-vs-expand-tree-label = showing full tree { $count ->
    [one] { $count } leaf
   *[other] { $count } leaves
}
hts-vs-expand-total-label = total { $total }
hts-vs-expand-total-unknown = total (unknown)
hts-vs-expand-offset-label = offset { $offset }
hts-vs-expand-filter-no-match = No members match the filter "{ $filter }".
hts-vs-expand-no-members = This expansion contains no members.
hts-vs-expand-column-code = Code
hts-vs-expand-column-display = Display
hts-vs-expand-column-system = System
hts-vs-expand-load-more = Load more
hts-vs-expand-echoed-parameters = Echoed parameters

## HTS Slice D — ConceptMap browser + detail with embedded $translate
## workbench (design doc §7.5). Every key here has a peer in
## es/de/main.ftl.

## ConceptMap status pills.

hts-cm-status-draft = draft
hts-cm-status-active = active
hts-cm-status-retired = retired
hts-cm-status-unknown = unknown

## CM browser page.

hts-cm-browser-title = ConceptMaps
hts-cm-browser-subtitle = Browse the terminology server's catalog of ConceptMaps and open any row to inspect its metadata or run a translation.
hts-cm-browser-filter-legend = Filter ConceptMaps
hts-cm-browser-filter-url = Canonical URL
hts-cm-browser-filter-name = Name
hts-cm-browser-filter-title = Title
hts-cm-browser-filter-status = Status
hts-cm-browser-filter-hint = Source and target canonicals are not offered as filters: HTS accepts only url, version, name, title and status when searching ConceptMaps, and ignores anything else. Filter by URL or name, then read the Mapping column.
hts-cm-browser-filter-reset = Reset
hts-cm-browser-empty = No ConceptMaps match these filters.
hts-cm-browser-load-more = Load more
hts-cm-browser-showing-count = Showing { $count ->
    [one] { $count } ConceptMap
   *[other] { $count } ConceptMaps
}
hts-cm-browser-table-caption = ConceptMaps matching the active filters.
hts-cm-browser-column-url = URL
hts-cm-browser-column-title = Title
hts-cm-browser-column-status = Status
hts-cm-browser-column-name = Name
hts-cm-browser-column-source = Source system
hts-cm-browser-column-target = Target system
hts-cm-browser-column-mapping = Mapping
hts-cm-browser-mapping-source-prefix = S:
hts-cm-browser-mapping-target-prefix = T:

## CM detail page.

hts-cm-detail-title = { $name } · ConceptMap
hts-cm-detail-title-fallback = ConceptMap
hts-cm-detail-eyebrow = ConceptMap
hts-cm-detail-section-identity = Identity
hts-cm-detail-section-mapping = Mapping
hts-cm-detail-publisher = Publisher
hts-cm-detail-jurisdiction = Jurisdiction
hts-cm-detail-purpose = Purpose
hts-cm-detail-source-uri = Source
hts-cm-detail-target-uri = Target
hts-cm-detail-group-count = Groups
hts-cm-detail-tabs-label = ConceptMap workbench sections
hts-cm-detail-tab-translate = Translate
hts-cm-detail-result-empty = Run the operation to see its result here.

## CM $translate form + result labels.

hts-cm-translate-heading = Translate a code
hts-cm-translate-scoped-map = ConceptMap (fixed)
hts-cm-translate-direction-legend = Direction
hts-cm-translate-direction-forward = Forward
hts-cm-translate-direction-reverse = Reverse
hts-cm-translate-source-legend = Source coding
hts-cm-translate-source-system = System
hts-cm-translate-source-system-placeholder = canonical URL
hts-cm-translate-source-code = Code
hts-cm-translate-source-display = Display
hts-cm-translate-source-display-placeholder = optional
hts-cm-translate-reverse-legend = Reverse source
hts-cm-translate-target-code = Target code
hts-cm-translate-target-code-hint = Required in reverse mode.
hts-cm-translate-target-legend = Target constraints
hts-cm-translate-target-system = Target system
hts-cm-translate-target-system-placeholder = canonical URL
hts-cm-translate-source-url = Source ValueSet
hts-cm-translate-source-url-placeholder = canonical URL (optional)
hts-cm-translate-target-url = Target ValueSet
hts-cm-translate-target-url-placeholder = canonical URL (optional)
hts-cm-translate-date = Date
hts-cm-translate-date-placeholder = ISO 8601 (e.g. 2025-06-01)
hts-cm-translate-submit = Translate
hts-cm-translate-matches-count = { $count ->
    [one] { $count } match
   *[other] { $count } matches
}
hts-cm-translate-no-matches = No matches for this source.
hts-cm-translate-column-code = Code
hts-cm-translate-column-system = System
hts-cm-translate-column-display = Display
hts-cm-translate-column-mapping = { $kind ->
    [equivalence] Equivalence
    [relationship] Relationship
   *[other] Mapping
}
hts-cm-translate-column-origin = Origin

## HTS Slice E -- standalone Operations workbench (design doc s7.6).
## Every user-visible string on `/ui/hts/operations` resolves to a key
## in this section. Keys have peers in es/de/main.ftl (parity gated by
## the fluent-key inventory test).

## Shell.

## Op selector labels -- one per OperationKind slug.

## CS $lookup widening (Slice E adds useSupplement to the Slice B set).

## CS $validate-code widening.

## CS $subsumes standalone (heading + outcomes already live in Slice B).

## VS $expand widening (adds designation chip).
hts-vs-expand-advanced = Advanced parameters
hts-vs-expand-total = total { $n }

## VS $validate-code (new op in Slice E).

## CM $translate standalone (base keys already live in Slice D).

## $closure workbench (new op in Slice E).

## batch-validate workbench (new UI-fabricated op in Slice E).

## Slice F — standalone Import page (design doc §7.7).
##
## Shell + upload form + status region for POST /import. All strings
## live under `hts-import-*`; `hts-nav-import` above is the sidebar
## label reused from the Phase 1 stub set.

hts-import-title = Import terminology
hts-import-heading = Import terminology
hts-import-help = Submit a FHIR JSON Bundle. HTS accepts CodeSystem, ValueSet, and ConceptMap resources in one POST.
hts-import-source-legend = Source
hts-import-source-paste = Paste JSON
hts-import-source-file = Upload file
hts-import-bundle-textarea-label = FHIR Bundle (JSON)
hts-import-bundle-file-label = Bundle file (JSON)
hts-import-submit = Import
hts-import-status-empty = No import has been submitted yet.
hts-import-status-success = Import complete
hts-import-status-partial = Import partially succeeded
hts-import-status-rejected = Import rejected
hts-import-status-too-large = Bundle too large
hts-import-counts-heading = Counts by resource
hts-import-counts-created = Created / updated
hts-import-resource-code-system = CodeSystem
hts-import-resource-value-set = ValueSet
hts-import-resource-concept-map = ConceptMap
hts-import-resource-concept = Concepts inserted
hts-import-issues-heading = { $n ->
    [one] { $n } issue
   *[other] { $n } issues
}
hts-import-too-large-hint = The request exceeded the server's payload limit. Split the Bundle into smaller batches and retry.
hts-import-empty-bundle-error = Paste a JSON Bundle before submitting.
hts-import-invalid-json-error = The submitted body is not valid JSON.

# V3 "stepped" Import layout (#551): three numbered steps — choose source,
# review, result. Step 2 deliberately carries no entry counts: HTS reports
# counts only in the `POST /import` response, so a pre-flight number would
# be invented rather than measured.
hts-import-step-source = Choose source
hts-import-step-review = Review
hts-import-step-result = Result
hts-import-file-hint = JSON only. The file is read in your browser and copied into the Bundle field below; nothing is sent until you submit.
hts-import-bundle-hint = The Bundle is posted to POST /import on the terminology server. Existing resources are matched on url + version.
hts-import-review-target = Target server
hts-import-review-request = Request
hts-import-review-accepted = Accepted resources
hts-import-review-accepted-value = CodeSystem, ValueSet, ConceptMap
hts-import-review-existing = Existing resources
hts-import-review-existing-value = Updated in place when url and version match.
hts-import-review-note = Nothing is written until you submit. How many resources were actually created is reported by the server in the result below.
hts-import-counts-resource = Resource
hts-import-raw-toggle = Raw response
hts-import-rejected-note = Nothing was written to the terminology store.
hts-import-tag-success = Success
hts-import-tag-partial = Partial
hts-import-tag-error = Error

## Slice G — standalone Diagnostics page (design doc §7.9).
##
## Stacked-card view over CapabilityStatement, TerminologyCapabilities,
## /health, and /metrics — mirrors HFS's Capability Statement page.
## A failing source renders an `hts-degraded-reason-*` warning notice
## inside its own card; the other cards stay readable.


# Concept information plane (Direction B, "concept-first").
# The concept is a top-level object with its own permalink at
# /ui/hts/concepts?system=...&code=..., rendered as three panels:
# Identity, Mappings (across every stored ConceptMap), and Subsumption.
hts-concept-title = Concept
hts-concept-lede = One code, seen from every angle the terminology server can answer for: what it is, what it maps to, and where it sits in the hierarchy.
hts-concept-open = Open concept
hts-concept-panel-loading = Loading
hts-concept-panel-open = Open this panel

hts-concept-identity-heading = Identity
hts-concept-status-active = Active
hts-concept-status-inactive = Inactive
hts-concept-status-unreported = Activity not reported
hts-concept-field-system = System
hts-concept-field-code = Code
hts-concept-field-display = Display
hts-concept-field-code-system-name = CodeSystem name
hts-concept-field-version = Version
hts-concept-field-selectability = Selectability
hts-concept-selectability-abstract = Abstract (not selectable)
hts-concept-selectability-selectable = Selectable
hts-concept-field-definition = Definition
hts-concept-field-neighbours = Hierarchy neighbours
hts-concept-field-used-supplements = Supplements applied
hts-concept-designations-heading = Designations
hts-concept-designations-value = Designation
hts-concept-designations-language = Language
hts-concept-designations-use = Use
hts-concept-properties-heading = Properties
hts-concept-properties-code = Property
hts-concept-properties-value = Value
hts-concept-raw-response = Raw response

hts-concept-mappings-heading = Mappings
hts-concept-mappings-direction-forward = Mappings where this concept is the source, across every stored ConceptMap.
hts-concept-mappings-direction-reverse = Mappings where this concept is the target, across every stored ConceptMap.
hts-concept-mappings-switch-forward = Show mappings from this concept
hts-concept-mappings-switch-reverse = Show mappings to this concept
hts-concept-mappings-empty = No ConceptMap maps this concept.
hts-concept-mappings-vocabulary = Mapping vocabulary
hts-concept-mappings-vocabulary-equivalence = equivalence (R4 / R4B)
hts-concept-mappings-vocabulary-relationship = relationship (R5 / R6)
hts-concept-mappings-vocabulary-unknown = Not reported
hts-concept-mappings-unattributable = The server does not attribute reverse-mode matches to a source map, so the origin cannot be shown. Switch to the forward direction to see which ConceptMap each mapping came from.
hts-concept-mappings-origin = Origin map
hts-concept-mappings-column-code = Code
hts-concept-mappings-column-system = System
hts-concept-mappings-column-display = Display
hts-concept-mappings-column-mapping = Relationship

hts-concept-relations-heading = Subsumption
hts-concept-relations-lede = Each row is one subsumption check. The ancestor candidate is always sent as code A, so a hierarchy that agrees with itself answers "subsumes" every time.
hts-concept-relation-parent = Parent
hts-concept-relation-child = Child
hts-concept-relation-manual = Compared
hts-concept-relations-column-relation = Relation
hts-concept-relations-column-question = Question asked
hts-concept-relations-column-outcome = Outcome
hts-concept-relations-subsumes-verb = subsumes
hts-concept-subsumes-outcome-equivalent = Equivalent
hts-concept-subsumes-outcome-subsumes = Subsumes
hts-concept-subsumes-outcome-subsumed-by = Subsumed by
hts-concept-subsumes-outcome-not-subsumed = Not subsumed
hts-concept-relations-conflict-caveat = The concept lookup reports this hierarchy link but the subsumption check does not confirm it. That usually means the subsumption closure was not rebuilt after the CodeSystem was re-imported; the hierarchy itself survived.
hts-concept-relations-empty = This concept has no parents or children to compare.
hts-concept-relations-dropped = { $n } further comparators were not checked; this panel runs at most 20 subsumption calls per render.
hts-concept-relations-compare-label = Compare with code
hts-concept-relations-compare-placeholder = Another code in this system
hts-concept-relations-compare-hint = The system is pinned to this concept's, so enter the bare code. The check asks whether that code subsumes this one.
hts-concept-relations-compare-submit = Compare

## HTS detail pages -- V3 compact header (#551 Slice B/C/D layout pass).
## Shared chip-row and disclosure labels for the CodeSystem / ValueSet /
## ConceptMap detail pages, plus the result-panel headings and the two
## honesty footnotes (tree pager, reverse-mode originMap).

hts-detail-facts-label = Facts
hts-detail-canonical-url = Canonical URL
hts-detail-version-label = Version
hts-detail-status-label = Status
hts-cs-detail-facts-summary = All CodeSystem facts
hts-vs-detail-facts-summary = All ValueSet facts
hts-cm-detail-facts-summary = All ConceptMap facts
hts-cs-lookup-definition = Definition
hts-cs-validate-result-heading = Validation result
hts-cs-subsumes-result-heading = Subsumption result
hts-vs-expand-result-heading = Expansion
hts-vs-expand-table-caption = Expansion members returned by the terminology server.
hts-vs-expand-tree-note = Tree mode returns the whole hierarchy; the pager is flat-mode only.
hts-cm-translate-table-caption = Translate matches returned by the terminology server.
hts-cm-translate-origin-reverse-note = In reverse mode HTS omits originMap, so a match cannot be attributed to a specific concept map. Every Origin cell stays an em-dash by design — it is not a missing value.


# Capability & Conformance page (HTS mirror of HFS's page). The shared
# `cap-*` and `nav-capability-conformance` keys carry everything both
# pages say identically; only what is specific to a terminology server
# lives here.
hts-capability-lede = What this terminology server advertises right now — composed live from /metadata.
hts-capability-operations-empty = No operations advertised.
hts-capability-rest-empty = No REST resources advertised.
hts-capability-terminology-heading = Terminology Capabilities
hts-capability-expansion-hierarchical = Hierarchical expansion
hts-capability-expansion-paging = Expansion paging
hts-capability-expansion-incomplete = Incomplete expansions
hts-capability-expansion-parameters = $expand parameters
hts-capability-validate-code-translations = Validate-code translations
hts-capability-translation-needs-map = Translation needs a map
hts-capability-closure = Closure maintenance
hts-capability-code-systems-declared = Code systems declared
hts-capability-flag-true = Yes
hts-capability-flag-false = No
hts-capability-raw-truncated = Truncated to the first { $shown } of { $total } bytes — this server's statement grows with the code systems it loads.
hts-capability-raw-full = View the complete statement

# Home V3 tile sub-lines. The mockup folds Backend, FHIR version,
# Bundled data and Avg latency into the sub-line of the tile each
# qualifies, instead of giving them tiles of their own.
hts-home-tile-status-sub = backend { $backend } · FHIR { $version }
hts-home-tile-uptime-sub = hts v{ $version } · no restarts since { $since } UTC
hts-home-tile-uptime-sub-noclock = hts v{ $version }
hts-home-tile-loaded-systems-sub = { $mib } MiB bundled on disk
hts-home-tile-requests-sub = { $ms } ms average · from /metrics

# Chart caption, composed from the SELECTED window and status class.
# Each locale owns its own word order through the two placeables.
hts-home-chart-hint = { $window }, { $classes }. Sampled while this page is open. Excludes this page's own 15 s refresh and /metrics scrapes.
hts-home-chart-hint-window-15m = Last 15 minutes
hts-home-chart-hint-window-1h = Last hour
hts-home-chart-hint-window-6h = Last 6 hours
hts-home-chart-hint-series-all = all status classes
hts-home-chart-hint-series-2xx = 2xx responses only
hts-home-chart-hint-series-4xx = 4xx responses only
hts-home-chart-hint-series-5xx = 5xx responses only
