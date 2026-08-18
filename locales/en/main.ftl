# Helios FHIR Server â€” UI message catalog
# Locale: English (en) â€” SOURCE LOCALE. Every key defined here is the
# canonical set; other locales are expected to provide the same keys.
#
# Syntax: Project Fluent (https://projectfluent.org/). Terms (prefixed with
# `-`) are reusable snippets; messages (bare identifiers) are what the UI
# looks up. Placeables `{ $var }` are interpolated by the caller. Do NOT put
# markup or logic here â€” translations are data, the template renders them.

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

# Pluralized count â€” every locale must supply the plural categories its
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

## Errors (mirrors OperationOutcome text; see docs/multi-language.md Â§5)

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
nav-import = Import
nav-export = Export
nav-sql-on-fhir = SQL-on-FHIR
nav-capability-conformance = Capability & Conformance
nav-search-parameters = Search Parameters
nav-admin-ops = Admin / Ops
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
tenants-search-placeholder = Search by name or tenant idâ€¦
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

footer-copyright = Â© { $year } { -org-name }

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
history-not-found = No history for that resource â€” check the type and id.
history-diff-heading = { $from }
history-metadata-hidden = { $count ->
    [one] { $count } metadata change hidden
   *[other] { $count } metadata changes hidden
}
history-textual = Show full text diff
history-only-metadata = Only metadata changed between these versions.
history-identical = These two versions are identical.
history-deleted = { $version } is a deletion â€” there is nothing to diff against.
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
sp-lede = Browse the parameters this server resolves searches against, filtered by base resource type. Stored parameters can be created, edited, and deleted; the registry picks changes up per tenant.
sp-version-label = FHIR version
sp-spec-missing = The full spec bundle (search-parameters-*.json) was not found in the data directory â€” only the minimal embedded fallback parameters are shown.
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
sp-detail-readonly = Spec parameter (compiled in from the data file) â€” read-only.
sp-field-url = Canonical URL
sp-field-name = Name
sp-field-status = Status
sp-field-base = Base resource types
sp-field-expression = FHIRPath expression
sp-field-description = Description
sp-field-target = Target types
sp-field-components = Components
sp-status-hint = The loader promotes the spec's draft status to active on load.
sp-note-conflict = Duplicate (base, code) within the same source as { $url } â€” the registry rejects this collision (DuplicateCode).
sp-note-overrides = Overrides { $url } on (base, code): a Stored definition outranks the spec parameter, so this one resolves searches. The registry logs a WARN naming both URLs.
sp-note-shadowed = Shadowed by { $url } on (base, code): a higher-precedence source resolves searches for this slot.
sp-note-empty-expression = Empty expression: the extractor indexes zero rows, so every search on this parameter silently returns empty.
sp-note-no-target = Reference parameter with no target types: chained search cannot resolve the referenced type.
sp-note-choice-type = Choice-type expression: the extractor rewrites ofType(T) / as T to the concrete element (for example valueQuantity) before evaluating against raw stored JSON.
sp-new = New search parameter
sp-edit = Edit
sp-delete = Delete
sp-delete-confirm = Delete this stored search parameter? Searches that use it stop matching once the registry refreshes.
cmp-new = New compartment definition
cmp-edit = Edit
cmp-delete = Delete
cmp-delete-confirm = Delete this compartment definition? Its compartment routes stop resolving.
crud-delete-failed = Delete failed

## Compartment viewer & tester (#237)

cmp-heading = Compartments
cmp-lede = The compartment definitions this server routes /{"{"}compartment{"}"}/{"{"}id{"}"}/{"{"}type{"}"} requests with, and a tester that answers: is this type in this compartment, via which parameters, and what search does the server run?
cmp-rail-label = Compartment definitions
cmp-rail-heading = Compartments
cmp-degraded = Compartment definitions could not be loaded from this server right now â€” the self-call to /CompartmentDefinition failed (with authentication enabled this usually means the outbound service token is missing or invalid). The page retries on the next request.
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
cmp-filter-all = All types
cmp-filter-excluded = Excluded
cmp-member = member
cmp-excluded = excluded
cmp-tester-id = Id
cmp-tester-target = Target type (or *)
cmp-tester-run = Test
cmp-result-member = âœ“ member â€” via { $params }
cmp-result-flat = // equivalent flat search
cmp-result-member-note = The server resolves the compartment route to this search over the type's reference parameters.
cmp-result-self = âœ“ member â€” the compartment resource itself ({"{"}def{"}"})
cmp-result-self-note = The compartment instance is trivially in its own compartment; the route reads the resource directly.
cmp-result-notmember = âœ• { $type } is not a member of this compartment
cmp-result-notmember-note = The server returns 404 with an OperationOutcome for types that are not compartment members.
cmp-result-fanout = Fans out to { $count } member types
cmp-result-fanout-note = Excluded types are skipped, not failed â€” the fan-out drops non-member types rather than erroring.
queries-builder-heading = Search builder
queries-url-label = FHIR search URL
queries-url-placeholder = GET /Patient?name=smith&birthdate=ge1980-01-01
queries-builder-hint = Edit the GET URL directly or through the rows below â€” they stay in sync. Run executes the search here and records it under Recent; give it a name to keep it in the saved list.
queries-recent = Recent
queries-recent-heading = Recent searches
queries-recent-empty = No recent searches yet â€” Run one to record it here.
queries-invalid-url = Enter a search like GET /Patient?name=smith â€” the resource type comes from the path.

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
plain-arrow = â€™s
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
queries-add-has = â§‰ Filter a resource that links here
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

## Search â€” natural language & visual builder (#255)

search-heading = Search
search-lede = Describe what you're looking for, or build the query by hand. Either way you get a FHIR search query you can read, correct, and run.
search-query-tag = QUERY
search-copy = Copy the query

search-mode-label = How to write the query
search-mode-nl = Natural language
search-mode-builder = Visual builder

search-nl-label = Describe the search
search-nl-placeholder = Describe what you're looking for â€” e.g. patients named Smith born after 1980
search-nl-hint = Your text and this server's search parameters go to the language model. Patient data never does. The query it writes is shown below for you to check and run.
search-nl-working = Translatingâ€¦
search-nl-caveats = Worth knowing:
search-nl-unsupported = That isn't a search this server can run. Try describing the records you want to find.

search-nl-example-1 = Female patients over 65 with a diabetes diagnosis
search-nl-example-2 = Observations from the last 30 days, most recent first
search-nl-example-3 = Encounters at Boston General still in progress

search-setup-heading = Natural-language search is available
search-setup-body = Turn plain-language descriptions into FHIR search queries. It needs an API key for a language model â€” the server reads it from the environment, and it never reaches this page. Until one is set, use the visual builder below.
search-setup-key-placeholder = your API key
search-setup-disable = To remove the feature entirely â€” endpoint, page, and this notice â€” set HFS_NL_SEARCH_ENABLED=false.
search-setup-docs = Read the how-to

## Resource editor (#264)

editor-heading = Resource editor
editor-lede = Edit a resource against its schema: add any element the schema allows, at any depth â€” including extensions, on any node that accepts one.
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
editor-binding-hint = Bound to a value set â€” codes come from it; strength shown
editor-legend-live = Checked as you type: structure, cardinality, required bindings
editor-legend-save = Checked on save: constraints and terminology
editor-deferred-badge = on save
editor-deferred-hint = Codes are verified against the value set when you save (and live in the picker where a terminology server is configured)
editor-must-support-hint = Must-support: consumers of this profile are expected to handle this element
editor-add-filter = Filter elements
editor-add-another = add another
editor-pick-type = Pick a typeâ€¦
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
resources-save-invalid = The JSON is not valid â€” fix it before saving.
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
batch-semantics-batch = Batch: entries run independently â€” a failed entry does not stop or undo the others.
batch-semantics-transaction = Transaction: all or nothing â€” if any entry fails, the server rolls the whole bundle back.
batch-tab-actions = Actions
batch-tab-json = Bundle JSON
batch-no-body = (no body â€” this entry only addresses a resource)
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

## Bulk Import workspace (#527)

bulk-import-title = Bulk Import
bulk-import-new = New submission
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
bulk-import-create-submit = Create submission
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
bulk-import-processing-waiting = Waiting for the recipient's first status reportâ€¦
bulk-import-result = Result
bulk-import-result-finished = Processing finished at
bulk-import-result-outputs = Output files
bulk-import-result-errors = Error files
bulk-import-abort-manifest = Abort

## HTS administrative UI (crates/hts-ui) â€” Phase 1 scaffold stubs
##
## The full catalog for the HTS UI is filled in during Phase 1.4 / Phase 2
## slices per `edson/docs/hts-ui-design.md` Â§7 (Fluent convention:
## hts-<page>-<role>-<control>). These stubs cover the base layout, sidebar
## nav, and the dashboard scaffold placeholder rendered by the Phase 1 blocker
## slice. They must be kept in parity with es/de/main.ftl.

-hts-app-name = Helios Terminology Server
hts-app-title = { -hts-app-name }

hts-nav-section-work = Terminology
hts-nav-section-tools = Tools
hts-nav-section-server = Server
hts-nav-dashboard = Dashboard
hts-nav-code-systems = Code Systems
hts-nav-value-sets = Value Sets
hts-nav-concept-maps = Concept Maps
hts-nav-operations = Operations
hts-nav-import = Import
hts-nav-diagnostics = Diagnostics

hts-fhir-version-heading = FHIR version
hts-fhir-version = FHIR { $version }

hts-dashboard-title = Dashboard
hts-dashboard-subtitle = Terminology server health, catalog inventory, and quick actions.

## Dashboard rows (row headings are visually hidden â€” they're for screen readers).

hts-dashboard-row-status = Server status
hts-dashboard-row-inventory = Loaded inventory
hts-dashboard-row-metrics = Traffic metrics
hts-dashboard-quick-links = Quick links

## Dashboard tiles.

hts-dashboard-tile-status = Status
hts-dashboard-tile-backend = Backend
hts-dashboard-tile-uptime = Uptime
hts-dashboard-tile-fhir-version = FHIR version
hts-dashboard-tile-loaded-systems = Loaded systems
hts-dashboard-tile-loaded-systems-hint = From TerminologyCapabilities.codeSystem[]
hts-dashboard-tile-bundled-data = Bundled data
hts-dashboard-tile-bundled-data-value = { $mib } MiB
hts-dashboard-tile-bundled-data-hint = From HTS_BOOTSTRAP_DIR footprint
hts-dashboard-tile-requests = Requests
hts-dashboard-tile-avg-latency = Avg latency
hts-dashboard-tile-metrics-hint = From /metrics â€” Wave 2

## /health `status` values, keyed for translation.

hts-dashboard-status-ok = OK

## Degraded banner (design doc Â§7 header contract).

hts-degraded-title = Terminology backend not fully available
hts-degraded-body = Some tiles are hidden until HTS becomes reachable again. Interactive controls are disabled on affected pages.
hts-degraded-reason-client-build = Failed to build the upstream HTTP client.
hts-degraded-reason-upstream-down = Could not reach the terminology server.
hts-degraded-reason-upstream-timeout = The terminology server did not respond in time.
hts-degraded-reason-upstream-error = The terminology server returned an error status.
hts-degraded-reason-upstream-shape = The terminology server returned an unexpected response shape.
hts-degraded-reason-bootstrapping = The terminology server is still loading its bootstrap data.
hts-degraded-reason-unknown = The terminology server is temporarily unavailable.

## Dialect chip (topbar, session-wide displayLanguage / Accept-Language per Â§7.1).

hts-dialect-label = Dialect
hts-dialect-prefix = dialect:
hts-dialect-heading = Session dialect
hts-dialect-hint = Controls displayLanguage on expansions and Accept-Language on reads. Per-op fields on Operations override this.

## OperationOutcome partial (shared, design doc Â§7 / Â§11).

hts-outcome-severity = Severity: { $severity }
hts-outcome-request-id = Request id: { $id }
hts-outcome-code-not-found = The requested resource was not found.
hts-outcome-code-invalid = The request was rejected as invalid.
hts-outcome-code-too-costly = The requested operation was rejected as too expensive.
hts-outcome-code-unknown = The server returned an issue the UI does not recognise.
hts-degraded-since = Since { $timestamp }

## HTS Slice B â€” CodeSystem browser + detail with embedded workbench
## (design doc Â§7.2 + Â§7.3). Every key here has a peer in es/de/main.ftl.

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
hts-cs-browser-filter-search = Search
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
hts-cs-browser-error-title = CodeSystems could not be listed

## CS detail page.

hts-cs-detail-title = { $name } Â· CodeSystem
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
hts-cs-detail-tab-metadata = Metadata
hts-cs-detail-tab-lookup = Lookup
hts-cs-detail-tab-validate = Validate
hts-cs-detail-tab-subsumes = Subsumes
hts-cs-detail-workbench-hint = Pick an operation to run against this CodeSystem.
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
hts-workbench-format-json = JSON
hts-workbench-format-xml = XML

## Additional degraded reason for CS-read 404s (design doc Â§7.3 states matrix).

hts-degraded-reason-upstream-not-found = The terminology server did not find that resource.

## HTS Slice C â€” ValueSet browser + detail with embedded $expand workbench
## (design doc Â§7.4 + Â§7.4.1). Every key here has a peer in es/de/main.ftl.

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
hts-vs-browser-filter-search = Search
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

## VS detail page.

hts-vs-detail-title = { $name } Â· ValueSet
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
hts-vs-detail-tab-metadata = Metadata
hts-vs-detail-tab-expand = Expand
hts-vs-detail-workbench-hint = Pick an operation to run against this ValueSet.
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
hts-vs-expand-ceiling-warning-body = You requested threshold { $requested }, which is above the UI ceiling â€” the header was not attached.
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

## HTS Slice D â€” ConceptMap browser + detail with embedded $translate
## workbench (design doc Â§7.5). Every key here has a peer in
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
hts-cm-browser-filter-source = Source system
hts-cm-browser-filter-target = Target system
hts-cm-browser-filter-status = Status
hts-cm-browser-filter-search = Search
hts-cm-browser-filter-reset = Reset
hts-cm-browser-empty = No ConceptMaps match these filters.
hts-cm-browser-load-more = Load more
hts-cm-browser-showing-count = Showing { $count ->
    [one] { $count } ConceptMap
   *[other] { $count } ConceptMaps
}
hts-cm-browser-table-caption = ConceptMaps matching the active filters.
hts-cm-browser-column-url = URL
hts-cm-browser-column-version = Version
hts-cm-browser-column-title = Title
hts-cm-browser-column-status = Status

## CM detail page.

hts-cm-detail-title = { $name } Â· ConceptMap
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
hts-cm-detail-tab-metadata = Metadata
hts-cm-detail-tab-translate = Translate
hts-cm-detail-workbench-hint = Pick an operation to run against this ConceptMap.
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
hts-cm-translate-matches-heading = Matches
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
hts-cm-translate-column-mapping-equivalence = Equivalence
hts-cm-translate-column-mapping-relationship = Relationship
hts-cm-translate-validate-forward-missing = Forward translation requires both `code` and `system`.
hts-cm-translate-validate-reverse-missing-target-code = Reverse translation requires `targetCode`.

## HTS Slice E -- standalone Operations workbench (design doc s7.6).
## Every user-visible string on `/ui/hts/operations` resolves to a key
## in this section. Keys have peers in es/de/main.ftl (parity gated by
## the fluent-key inventory test).

## Shell.
hts-operations-title = Operations workbench
hts-operations-eyebrow = Terminology
hts-operations-subtitle = Run terminology operations against the connected server. Every operation is proxied via POST regardless of the input form's verb.
hts-operations-selector-label = Operation
hts-operations-resource-tabs-label = Resource family
hts-operations-resource-code-system = CodeSystem
hts-operations-resource-value-set = ValueSet
hts-operations-result-empty = Run the operation to see its result here.
hts-operations-scope-legend = Scope
hts-operations-scope-system = CodeSystem canonical URL
hts-operations-scope-instance = Instance id
hts-operations-scope-instance-placeholder = instance id
hts-operations-scope-canonical = Canonical URL
hts-operations-not-implemented = This operation ships in Slice E2.
hts-operations-closure-stateless-warning = Closure state lives on the server keyed by the `name` you provide. The UI never persists it across requests.
hts-operations-closure-empty-graph = No closure edges yet -- submit at least one Coding to add nodes to the graph.

## Op selector labels -- one per OperationKind slug.
hts-operations-op-lookup = $lookup
hts-operations-op-validate-code = $validate-code
hts-operations-op-subsumes = $subsumes
hts-operations-op-expand = $expand
hts-operations-op-translate = $translate
hts-operations-op-closure = $closure
hts-operations-op-batch-validate = batch-validate

## CS $lookup widening (Slice E adds useSupplement to the Slice B set).
hts-cs-lookup-useSupplement = Supplement
hts-cs-lookup-useSupplement-hint = Optional canonical URL of a CodeSystem supplement to layer on top of the base.
hts-cs-lookup-result-heading = Lookup result
hts-cs-lookup-fact-name = Name
hts-cs-lookup-fact-version = Version
hts-cs-lookup-fact-display = Display
hts-cs-lookup-fact-definition = Definition

## CS $validate-code widening.
hts-cs-validate-version = CodeSystem version
hts-cs-validate-systemVersion = System version override
hts-cs-validate-mode-CodeableConcept = CodeableConcept
hts-cs-validate-displayLanguage = Display language
hts-cs-validate-advanced = Advanced parameters
hts-cs-validate-date = Date
hts-cs-validate-activeOnly = Active codes only
hts-cs-validate-abstract = Allow abstract codes
hts-cs-validate-lenient-display-validation = Lenient display validation
hts-cs-validate-useSupplement = Supplement URL
hts-cs-validate-system-version = System version pin
hts-cs-validate-check-system-version = Check system version
hts-cs-validate-force-system-version = Force system version
hts-cs-validate-result-heading = Validate result
hts-cs-validate-result-badge-true = Valid
hts-cs-validate-result-badge-false = Not valid
hts-cs-validate-fact-code = Code
hts-cs-validate-fact-display = Display
hts-cs-validate-fact-message = Message

## CS $subsumes standalone (heading + outcomes already live in Slice B).
hts-cs-subsumes-version = Version
hts-cs-subsumes-codeA = Code A
hts-cs-subsumes-codeB = Code B
hts-cs-subsumes-result-heading = Subsumption result

## VS $expand widening (adds designation chip).
hts-vs-expand-displayLanguage = Display language
hts-vs-expand-activeOnly = Active only
hts-vs-expand-includeDesignations = Include designations
hts-vs-expand-designation = Designation
hts-vs-expand-designation-hint = Chip filter -- pass a `use|value` pair per line (repeatable).
hts-vs-expand-advanced = Advanced parameters
hts-vs-expand-threshold-hint = HTS refuses expansions above threshold. UI ceiling: { $ceiling }.
hts-vs-expand-result-heading = Expansion
hts-vs-expand-total = total { $n }
hts-vs-expand-count-shown = showing { $n }

## VS $validate-code (new op in Slice E).
hts-vs-validate-heading = Validate a code against a ValueSet
hts-vs-validate-source-legend = ValueSet source
hts-vs-validate-source-canonical = Canonical URL
hts-vs-validate-source-instance = Instance id
hts-vs-validate-source-inline = Inline JSON
hts-vs-validate-mode-legend = Input shape
hts-vs-validate-mode-code = Code
hts-vs-validate-mode-coding = Coding
hts-vs-validate-mode-CodeableConcept = CodeableConcept
hts-vs-validate-code = Code
hts-vs-validate-system = System
hts-vs-validate-systemVersion = System version
hts-vs-validate-display = Display
hts-vs-validate-coding-legend = Coding
hts-vs-validate-coding-system = System
hts-vs-validate-coding-code = Code
hts-vs-validate-coding-display = Display
hts-vs-validate-displayLanguage = Display language
hts-vs-validate-valueSetVersion = ValueSet version
hts-vs-validate-advanced = Advanced parameters
hts-vs-validate-date = Date
hts-vs-validate-activeOnly = Active only
hts-vs-validate-abstract = Allow abstract codes
hts-vs-validate-lenient-display-validation = Lenient display validation
hts-vs-validate-useSupplement = Supplement URL
hts-vs-validate-tx-resource = Extra tx-resource
hts-vs-validate-default-valueset-version = Default ValueSet version
hts-vs-validate-no-membership = Code is not a member of the ValueSet.
hts-vs-validate-result-heading = Validate result
hts-vs-validate-result-badge-true = Valid
hts-vs-validate-result-badge-false = Not valid
hts-vs-validate-fact-code = Code
hts-vs-validate-fact-system = System
hts-vs-validate-fact-display = Display
hts-vs-validate-fact-message = Message

## CM $translate standalone (base keys already live in Slice D).
hts-cm-translate-code = Code
hts-cm-translate-system = System
hts-cm-translate-display = Display
hts-cm-translate-targetCode = Target code
hts-cm-translate-targetSystem = Target system
hts-cm-translate-result-heading = Translate result
hts-cm-translate-result-badge-true = Matched
hts-cm-translate-result-badge-false = No match

## $closure workbench (new op in Slice E).
hts-cm-closure-heading = Closure graph
hts-cm-closure-name = Closure name
hts-cm-closure-name-hint = Client-provided name that identifies the closure table on the server across requests.
hts-cm-closure-concepts-legend = Concepts
hts-cm-closure-concepts-hint = Add up to three seed codings; each row is a system + code pair.
hts-cm-closure-concept-system = System
hts-cm-closure-concept-code = Code
hts-cm-closure-result-heading = Closure edges
hts-cm-closure-edge-source = Source
hts-cm-closure-edge-equivalence = Equivalence
hts-cm-closure-edge-target = Target

## batch-validate workbench (new UI-fabricated op in Slice E).
hts-vs-batch-heading = Batch validate codes against a ValueSet
hts-vs-batch-target-value-set-label = Target ValueSet
hts-vs-batch-rows-legend = Rows
hts-vs-batch-rows-hint = Enter one code per row; empty rows are dropped.
hts-vs-batch-row-code = Code
hts-vs-batch-row-system = System
hts-vs-batch-row-display = Display
hts-vs-batch-row-timeout = Timed out
hts-vs-batch-row-placeholder = --
hts-vs-batch-result-heading = Batch result
hts-vs-batch-target-hint = Target ValueSet: { $target }
hts-vs-batch-column-code = Code
hts-vs-batch-column-system = System
hts-vs-batch-column-display = Display
hts-vs-batch-column-result = Result
hts-vs-batch-progress = { $n } of { $m } completed
hts-vs-batch-progress-final = { $m } completed

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
hts-import-counts-updated = Updated
hts-import-counts-errors = Errors
hts-import-resource-code-system = CodeSystem
hts-import-resource-value-set = ValueSet
hts-import-resource-concept-map = ConceptMap
hts-import-resource-concept = Concepts inserted
hts-import-duration = { $seconds } s
hts-import-issues-heading = { $n ->
    [one] { $n } issue
   *[other] { $n } issues
}
hts-import-too-large-hint = The request exceeded the server's payload limit. Split the Bundle into smaller batches and retry.
hts-import-empty-bundle-error = Paste a JSON Bundle before submitting.
hts-import-invalid-json-error = The submitted body is not valid JSON.

## Slice G — standalone Diagnostics page (design doc §7.9).
##
## Deep-link friendly, tab-swap view over CapabilityStatement,
## TerminologyCapabilities, /health, and /metrics. Per-tab
## OperationOutcome renders inside the shared `#diag-panel` so a
## failing surface never blanks the tab strip.

hts-diagnostics-title = Diagnostics
hts-diagnostics-heading = Diagnostics
hts-diagnostics-nav-label = Diagnostics
hts-diagnostics-fhir-version-chip = FHIR { $version }
hts-diagnostics-tab-capability = Capability
hts-diagnostics-tab-terminology-capabilities = TerminologyCapabilities
hts-diagnostics-tab-health = /health
hts-diagnostics-tab-metrics = /metrics
hts-diagnostics-capability-heading = CapabilityStatement
hts-diagnostics-terminology-capabilities-heading = TerminologyCapabilities
hts-diagnostics-health-heading = Health
hts-diagnostics-metrics-heading = Prometheus metrics
hts-diagnostics-property-url = URL
hts-diagnostics-property-version = Version
hts-diagnostics-property-name = Name
hts-diagnostics-property-title = Title
hts-diagnostics-property-status = Status
hts-diagnostics-property-date = Date
hts-diagnostics-capability-rest-heading = REST resources
hts-diagnostics-terminology-code-systems-heading = Code Systems
hts-diagnostics-terminology-code-systems-empty = No systems loaded
hts-diagnostics-health-status-label = Status
hts-diagnostics-health-unknown = Unknown
hts-diagnostics-metrics-figcaption = Prometheus text-format metrics
hts-diagnostics-metrics-empty = Metrics endpoint returned no body
hts-diagnostics-error = This diagnostic surface is temporarily unavailable.
