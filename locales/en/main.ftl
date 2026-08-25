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
terminology-invalid-heading = HFS_TERMINOLOGY_SERVER is invalid
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
error-generic = Something went wrong. Please try again.

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
tenants-row-failed = Provisioning failed
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
crud-delete-failed = Delete failed

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
editor-edit-raw = Edit raw
editor-versions = Versions
editor-versions-none = No prior versions.

## Resources workspace (#282)

resources-heading = Resources
resources-lede = Browse, search, create, and edit FHIR resources. Search in natural language or build the query by hand, then open any result to edit it.
resources-create-typed = Create new { $type }
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
batch-response-heading = Per-Action Outcomes
batch-sum-created = created
batch-sum-updated = updated
batch-sum-other = read/other
batch-sum-failed = failed
batch-request-failed = The request failed
batch-back = Back to Bundle
batch-execute-again = Execute again

## Bulk Import workspace (#527)

bulk-import-title = Bulk Import
bulk-import-new = New Submission
bulk-import-create-title = Create Bulk Submission
bulk-import-field-name = Submission name
bulk-import-field-recipient = Recipient base URL
bulk-import-field-recipient-hint = This is the base URL of the server where the data will be submitted.
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
bulk-import-create-submit = Create Submission
bulk-import-unavailable = The storage backend does not host the settings store, so submissions cannot be saved.
bulk-import-submissions = Submissions
bulk-import-records = records
bulk-import-col-name = Name
bulk-import-col-status = Status
bulk-import-col-created = Created
bulk-import-col-manifests = Manifests
bulk-import-col-destination = Destination
bulk-import-empty = No submissions yet. Create one to get started.
bulk-import-all = All Submissions
bulk-import-status-not-started = Not Started
bulk-import-status-in-progress = In Progress
bulk-import-status-stopped = Stopped
bulk-import-status-completed = Completed
bulk-import-detail-recipient = Data Recipient
bulk-import-detail-id = Submission ID
bulk-import-detail-submitter = Submitter
bulk-import-detail-created = Created
bulk-import-detail-status = Status
bulk-import-detail-auth = Authentication
bulk-import-abort = Abort
bulk-import-complete = Complete
bulk-import-delete = Delete
bulk-import-add-manifest = Add Manifest
bulk-import-add-manifest-title = Add Manifest
bulk-import-add-manifest-submit = Add
bulk-import-field-manifest-url = Manifest URL
bulk-import-field-manifest-url-hint = URL pointing to a Bulk Export Manifest with a precoordinated FHIR data set.
bulk-import-field-fhir-base = FHIR base URL
bulk-import-field-fhir-base-hint = Base URL used by the Data Recipient when resolving relative references. Leave empty to use the base URL of the manifest.
bulk-import-field-output-format = Output format
bulk-import-field-output-format-hint = The format for the Bulk Data files in the manifest.
bulk-import-field-headers = File request headers
bulk-import-field-headers-hint = HTTP headers the Data Recipient should use when requesting a data file, one "Name: value" per line.
bulk-import-manifests = Manifests
bulk-import-no-manifests = No manifests yet. Add one to submit data.
bulk-import-submit = Submit
bulk-import-submit-all = Submit All
bulk-import-remove = Remove
bulk-import-log = Submission Log
bulk-import-log-empty = Nothing submitted yet.
bulk-import-field-submitter-system = Submitter system
bulk-import-field-submitter-value = Submitter value
bulk-import-field-submitter-hint = Must match an identifier registered with the Data Recipient (coordinated out-of-band). Leave empty to use the generated defaults.
bulk-import-field-submission-id = Submission ID
bulk-import-field-submission-id-hint = Unique per submitter. Leave empty to generate a UUID.
bulk-import-processing = Processing
bulk-import-processing-waiting = Waiting for the recipient's first status report…
bulk-import-result = Result
bulk-import-result-finished = Processing finished at
bulk-import-result-outputs = Output files
bulk-import-result-errors = Error files
bulk-import-abort-manifest = Abort
ui-cancel = Cancel
ui-close = Close
editor-orphans-title = These issues have no field yet — add the elements to fix them
editor-hint-date = FHIR date: YYYY, YYYY-MM, or YYYY-MM-DD
editor-hint-datetime = FHIR dateTime: YYYY, YYYY-MM, YYYY-MM-DD, or a full timestamp with timezone (2024-05-17T14:30:00+02:00)
editor-hint-time = FHIR time: HH:MM:SS
editor-hint-instant = FHIR instant: full timestamp with timezone, e.g. 2024-05-17T14:30:00.000Z

## Subscriptions operator page (#580)

subs-title = Subscriptions
subs-lede = Read-only view of the subscriptions engine: every registered subscription, its channel, live status, and delivery counters.
subs-unavailable = The subscriptions engine is not enabled on this server.
subs-empty = No subscriptions are registered for this tenant.
subs-card-failing = Failing
subs-card-failing-sub = needs attention
subs-card-idle = Idle
subs-card-idle-sub = no clients
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
subs-state-active = active
subs-state-error = error
subs-state-idle = 0 clients
subs-state-requested = requested
subs-state-off = off

## Bulk Export workspace (#537)

bulk-export-title = Bulk Export
bulk-export-active-title = Active Exports
bulk-export-active-link = Active exports
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
bulk-export-field-name = Name
bulk-export-field-name-placeholder = Diabetes registry 2024
bulk-export-types = Resource types
bulk-export-types-hint = Leave everything unchecked to export every type.
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
bulk-export-none = No exports yet. Start one from the Bulk Export page.
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
cap-unavailable = The CapabilityStatement could not be fetched from the server — the self-call may need an outbound token when authentication is enabled.

## SQL on FHIR section stubs (#649)

sql-stub-heading = Coming Soon
sql-stub-planned = Planned
sql-stub-api = Serving API

sql-vd-title = View Definitions
sql-vd-lede = Author and manage the ViewDefinitions that SQL on FHIR runs flatten resources with.

sql-queries-title = SQL Queries
sql-queries-lede = Run SQL on FHIR queries against this server.
sql-queries-body = Compose a query, run it, and inspect the resulting rows inline.

sql-views-title = SQL Views
sql-views-lede = Reusable SQL views layered over ViewDefinitions.
sql-views-body = SQLView libraries and the views they define, resolved with their dependencies.

sql-export-title = SQL Export
sql-export-lede = Long-running SQL on FHIR export jobs.
sql-export-body = Start an export, follow its progress, and cancel jobs that are no longer needed.

sql-files-title = Files
sql-files-lede = Manifests and output files produced by SQL exports.
sql-files-body = Browse each job's manifest and download its output files.

## View Definitions workspace (#649)

vd-new = Create New
vd-new-title = New View Definition
vd-rail-label = View definitions
vd-rail-heading = View Definitions
vd-filter = Filter views
vd-none = No view definitions yet.
vd-empty-lede = Create your first ViewDefinition with Create New.
vd-degraded = The view definition list could not be loaded.
vd-saved = Saved.
vd-run = Run
vd-run-failed = Running the view failed.
vd-save = Save
vd-duplicate = Duplicate
vd-delete = Delete
vd-delete-confirm = Delete view definition "{ $name }"? This cannot be undone.
vd-delete-failed = Deleting the view definition failed.
vd-json-heading = Definition (JSON)
vd-results-heading = Results
vd-results-empty = The view produced no rows.
