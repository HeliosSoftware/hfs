# HFS Swift Client

This package is the Swift client and SwiftUI admin surface for Helios HFS. It is
intentionally kept outside the Rust Cargo workspace so Rust builds remain
unchanged. Because it is a Swift Package, use `swift build` / `swift test` here —
the Rust `cargo fmt` / `clippy` gate does not apply to this directory.

## Goals

- Provide a reusable async Swift client for HFS FHIR REST APIs.
- Keep FHIR payload handling version-agnostic by default.
- Add typed convenience models only where UI workflows need them.
- Keep HTTP, auth, FHIR JSON, service operations, and SwiftUI screens separate.
- Support both a SwiftUI admin app and other Swift integrations from the same
  client modules.

## Package Layout

| Target | Purpose |
|--------|---------|
| `HFSCore` | Shared configuration, tenant context, FHIR versions, and errors. |
| `HFSFHIR` | Lightweight FHIR JSON resource wrappers and future typed summaries. |
| `HFSHTTP` | URLSession-backed transport behind a `HFSHTTPTransport` protocol (mockable in tests). |
| `HFSAuth` | Token provider abstractions and future SMART-on-FHIR support. |
| `HFSClient` | Base HFS API client: tenant-aware URL construction, headers, `/metadata`. |
| `HFSOperations` | Feature-specific API surfaces: resources, bulk data, audit, subscriptions, terminology, FHIRPath, SQL-on-FHIR, and CDS Hooks. |
| `HFSAdminUI` | SwiftUI shell, navigation, reusable components, the `HFSAppModel` state layer, and feature screens. |
| `HFSAdminApp` | macOS SwiftUI executable (`hfs-admin`) using the `App` / `WindowGroup` lifecycle. |

## How the App Works

`HFSAdminApp` is a standard SwiftUI `App` with a single `WindowGroup` and a
unified window toolbar, so the system provides a real `NSToolbar`, stable toolbar
placement, and Liquid Glass chrome.

State is owned by **`HFSAppModel`** (`@MainActor @Observable`), injected into the
view tree with `.environment(_:)` and read with `@Environment(HFSAppModel.self)`.
It holds the editable connection settings, builds the `HFSClient` lazily, tracks
`connectionState`, and exposes async actions (`connect()`, `refreshOverview()`).
Views render live state — there is no hardcoded status.

Connection flow:

1. **Settings** is a `Form` bound to `HFSAppModel` (Base URL, Tenant, FHIR
   version). **Connect** probes `/metadata`; success sets `.connected`, parses an
   Overview summary and the server's resource-type list, and stores the client.
2. The sidebar connection summary and the per-screen status strip reflect the
   live connection/tenant/version.
3. **Overview** shows facts parsed from the CapabilityStatement (resource-type
   count, FHIR version, tenant); **Refresh** re-fetches `/metadata`.
4. **Resources** lists the server's resource types, searches a selected type
   (`GET /[type]?_count=20` plus any editable search parameters), pages through
   results by following the Bundle `next` link, and shows each resource's JSON in
   a system `.inspector` panel.

## UI Layout Structure

`HFSAdminUI` is organized by responsibility:

| Folder | Purpose |
|--------|---------|
| `Navigation/` | Sidebar destinations and section metadata. |
| `Sidebar/` | Sidebar navigation and connection summary views. |
| `Detail/` | Per-destination content: live `HFSSettingsView`, `HFSResourcesView`, the Overview scaffold, and the shared placeholder scaffold for not-yet-built screens. |
| `Components/` | Reusable content primitives: status strip, metric tiles, workspace panel. |
| `Models/` | `HFSAppModel` state layer plus lightweight scaffold view models. |

The shell is composed entirely from first-party SwiftUI components:
`NavigationSplitView`, `List`, `Section`, `Form`, `GroupBox`, `LabeledContent`,
`Label`, `LazyVGrid`, `Menu`, `ToolbarItemGroup`, `.inspector`,
`ContentUnavailableView`, `ProgressView`, and `.alert`. This lets the system
provide window chrome and navigation, keep toolbar placement stable, and adopt
Liquid Glass automatically where supported.

> **Layout safety:** never put a hard `.frame(width:)` on a child of a
> split-managed/resizable container (a `NavigationSplitView` column or an
> `.inspector`). Use flexible `min/ideal/max` widths — a fixed width squeezed
> below its minimum makes AppKit report `minWidth > maxWidth` on a divider drag
> and abort the app.

## Status

| Screen | State |
|--------|-------|
| Settings | ✅ Live — connect/disconnect, validation, status feedback |
| Overview | ✅ Live — capability tiles from `/metadata`, refresh |
| Resources | ✅ Live — type list, search with parameters, pagination (`next`), JSON inspector |
| Search | ⏳ Scaffold placeholder |
| Bulk Jobs | ⏳ Scaffold placeholder |
| Audit | ⏳ Scaffold placeholder |
| Subscriptions | ⏳ Scaffold placeholder |

Other implemented foundations: the `HFSAppModel` observable state layer, the
mockable `HFSHTTPTransport`, and XCTest suites that run without a live server.

## Development Path

Done:

1. ✅ `HFSClient` request building, tenant-aware paths, and `/metadata`.
2. ✅ `HFSAppModel` state layer + functional Settings (connect/disconnect).
3. ✅ Overview wired to the live CapabilityStatement.
4. ✅ Resources browser: type list → search → JSON detail (`.inspector`).
5. ✅ Resource search **pagination** (follows the Bundle `next` link) and
   editable **search parameters**.

Coming next:

6. ⏭️ A dedicated Search screen reusing the parameter/pagination plumbing.
7. ⏭️ Resource **create / update / delete** (read-write; the "New Resource"
   toolbar action is currently a placeholder).
8. ⏭️ Operations screens: Bulk Jobs, Audit, Subscriptions.
9. ⏭️ **Auth** (`HFSAuth`): static bearer token, then SMART-on-FHIR discovery
   (currently `NoAccessTokenProvider`).
10. ⏭️ Settings persistence (`@AppStorage`/`UserDefaults`).
11. ⏭️ An iOS app under `clients/swift/Apps` depending on this package.

## Building

```bash
cd clients/swift
swift build
swift test
swift run hfs-admin
```

`swift run hfs-admin` opens a real SwiftUI window and is enough for development.
To run the app against a local server, start `cargo run --bin hfs` (default
R4 + SQLite on `http://localhost:8080`) and Connect from the Settings screen.

For a proper installed app (Dock entry, stable identity for UI automation), wrap
the release binary in a signed `.app` bundle — see the `run-swift-client` project
skill for the packaging and redeploy steps. An iOS app can be added later as an
Xcode project that depends on this Swift package.
