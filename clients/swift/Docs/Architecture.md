# HFS Swift Client Architecture

The Swift client follows the server architecture without copying the Rust crate
layout one-to-one. The Rust workspace owns FHIR behavior and persistence; the
Swift package owns user-facing workflows and HTTP integration.

## Module Boundaries

### HFSCore

Shared primitives with no UI or networking dependencies:

- server base URL, tenant context, supported FHIR version
- shared client errors (`HFSClientError`)
- feature flags discovered from `/metadata`

### HFSFHIR

FHIR payload representation:

- generic JSON resource envelopes (`FHIRResource`, `FHIRBundle`,
  `FHIROperationOutcome`)
- typed summary models for list rows and details (added as workflows need them)

The server already owns full FHIR validation and serialization, so the client
does not regenerate the FHIR model surface.

### HFSHTTP

Transport abstraction:

- `HFSHTTPTransport` protocol + `URLSessionHFSHTTPTransport`
- status code validation; retry/timeout policy later

Tests inject a fake transport, so the model and operations run without HFS.

### HFSAuth

Authentication state and token injection:

- `HFSAccessTokenProvider` protocol; `NoAccessTokenProvider` today
- static bearer tokens, then SMART-on-FHIR discovery/launch later

### HFSClient

Base API client — intentionally small:

- tenant-aware URL construction and common headers
- JSON request/response plumbing
- `capabilityStatement()` (`/metadata`)

Feature-specific API methods belong in `HFSOperations`.

### HFSOperations

Feature modules that map to HFS capabilities. `Resources.read/search` is
implemented; the rest are stubs awaiting their UI slices:

- `Resources`: read + paged search (`ResourceListItem`); CRUD/history/transactions later
- `BulkData`: `$export`, `$bulk-submit`, polling, manifests
- `Audit`: AuditEvent browsing and filters
- `Subscriptions`: Subscription CRUD, `$status`, `$events`, websocket binding
- `Terminology`: HTS metadata and terminology operations
- `FHIRPath`: expression evaluation endpoint integration
- `SQLOnFHIR`: ViewDefinition execution workflows
- `CDSHooks`: discovery and service invocation support

### HFSAdminUI

SwiftUI composition: app shell, navigation, the state layer, reusable
components, and feature screens. Folders:

- `Navigation`: destinations and section metadata
- `Sidebar`: system `List` sidebar and live connection summary
- `Detail`: per-destination content (live screens + shared placeholder scaffold)
- `Components`: status strip, metric tiles, workspace panel
- `Models`: `HFSAppModel` state layer + lightweight scaffold view models

## App Lifecycle

`HFSAdminApp` is a SwiftUI `App` with a single `WindowGroup` and a unified
toolbar style (`.windowToolbarStyle(.unified)`), not a hand-built `NSWindow`.
This gives a real `NSToolbar`, stable toolbar item placement across sidebar
toggles, a standard macOS menu bar, and automatic Liquid Glass adoption.

## State Management

`HFSAppModel` (`@MainActor @Observable`) is the single source of UI truth:

- editable connection settings (base URL, tenant, FHIR version)
- live `connectionState` (`disconnected` / `connecting` / `connected` / `failed`)
- the connected `HFSClient`, the parsed Overview summary, and the server's
  resource-type list
- async actions: `connect()` (probe `/metadata`), `refreshOverview()`,
  `disconnect()`, and a `resourceOperations()` accessor

It is injected with `.environment(_:)` at the root and read with
`@Environment(HFSAppModel.self)`. Views render live state; nothing is hardcoded.
Per-screen, transient state (e.g., the Resources list/selection) stays as local
`@State` in the screen view and calls into `HFSOperations`.

## SwiftUI Conventions

Compose from first-party components before writing custom views:
`NavigationSplitView`, `List`, `Section`, `Form`, `GroupBox`, `LabeledContent`,
`Label`, `LazyVGrid`, `Menu`, `ToolbarItemGroup`, `.inspector`,
`ContentUnavailableView`, `ProgressView`, and `.alert`. Add custom drawing only
for content surfaces the framework does not provide. Where the UI needs a value,
prefer service protocols over concrete networking so previews and tests are easy.

**Layout safety:** never put a hard `.frame(width:)` on a child of a
split-managed/resizable container (a `NavigationSplitView` column or an
`.inspector`). Use flexible `min/ideal/max` widths — a fixed width squeezed below
its minimum makes AppKit report `minWidth > maxWidth` during a divider drag and
abort the app.

## Screens

Implemented (live against a server):

1. Connection settings (`HFSSettingsView`)
2. Server overview from `/metadata`
3. Resource type browser -> paged search -> JSON detail (`.inspector`)

Planned (currently placeholder scaffold):

4. Dedicated search with parameters and pagination
5. JSON resource editor (create/update/delete)
6. Bulk job status
7. AuditEvent list
8. Subscription status

## App Project Strategy

Use Swift Package Manager for the reusable client and the macOS iteration shell.
When an iOS app is needed, add an Xcode project under `clients/swift/Apps` that
depends on this package. Keep reusable code in package targets rather than in the
app project.
