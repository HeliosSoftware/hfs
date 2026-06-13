---
name: run-swift-client
description: Build, run, and debug the HFS Swift admin client (macOS SwiftUI app) under clients/swift. Use for the hfs-admin executable, swift build/test, packaging and redeploying the .app bundle, connecting the app to a local HFS server, the client module layout, and SwiftUI conventions.
---

# HFS Swift Client

Use this when working with the Swift admin client under `clients/swift`. It is a
Swift Package Manager project that is intentionally **outside the Rust Cargo
workspace**, so `cargo` commands and the Rust `cargo fmt` / `clippy` completion
gate do not apply here. Use `swift build` / `swift test` instead.

## Layout

- Package root: `clients/swift` (`Package.swift`)
- Executable: `hfs-admin` — macOS SwiftUI app using the `App` / `WindowGroup`
  lifecycle (not a manual `NSWindow`)
- Library products: `HFSClientKit` (`HFSCore`, `HFSFHIR`, `HFSHTTP`, `HFSAuth`,
  `HFSClient`, `HFSOperations`) and `HFSAdminUI` (SwiftUI shell + screens)
- Minimum platforms: macOS 14, iOS 17

## Build, test, run

```bash
cd clients/swift
swift build            # debug build of all targets
swift test             # run the XCTest suites
swift run hfs-admin    # launch the macOS app from the terminal
```

`swift run hfs-admin` opens a real SwiftUI window (App/WindowGroup) and is enough
for day-to-day development.

## Connect the app to a local HFS server

1. Start a server (see `run-hfs-server`): `cargo run --bin hfs`
   — default is R4 + SQLite on `http://localhost:8080` (the minimal build).
2. Launch the app, open **Settings**, set Base URL (default
   `http://localhost:8080`), an optional Tenant, and FHIR version, then
   **Connect**. Connect probes `/metadata`; on success the Overview and
   Resources screens light up.
3. Seed data so the Resources browser has something to show:
   ```bash
   curl -s -X POST http://localhost:8080/Patient \
     -H "Content-Type: application/fhir+json" \
     -d '{"resourceType":"Patient","name":[{"family":"Smith"}]}'
   ```

## Package a proper .app bundle

`swift run` is fine for dev, but a bare executable has no bundle identity (no
Dock entry, no stable identity for UI automation / computer-use). For a real
installed app, wrap the release binary in a signed `.app`:

```bash
cd clients/swift
swift build -c release
APP="/Applications/HFS Admin.app"
rm -rf "$APP"; mkdir -p "$APP/Contents/MacOS"
cp .build/release/hfs-admin "$APP/Contents/MacOS/HFS Admin"
cat > "$APP/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>HFS Admin</string>
  <key>CFBundleDisplayName</key><string>HFS Admin</string>
  <key>CFBundleIdentifier</key><string>com.helios.hfs.admin</string>
  <key>CFBundleExecutable</key><string>HFS Admin</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>1.0</string>
  <key>CFBundleVersion</key><string>1.0</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSPrincipalClass</key><string>NSApplication</string>
</dict></plist>
PLIST
codesign --force --sign - "$APP"
open "$APP"
```

Redeploy after a code change (rebuild, copy binary, re-sign, relaunch):

```bash
cd clients/swift && swift build -c release
pkill -f "HFS Admin.app"; sleep 1
cp .build/release/hfs-admin "/Applications/HFS Admin.app/Contents/MacOS/HFS Admin"
codesign --force --sign - "/Applications/HFS Admin.app"
open "/Applications/HFS Admin.app"
```

## Conventions

- **Prefer first-party SwiftUI**: `NavigationSplitView`, `List`, `Form`,
  `LabeledContent`, `GroupBox`, `LazyVGrid`, `Label`, `Menu`, `ToolbarItemGroup`,
  `.inspector`, `ContentUnavailableView`, `ProgressView`, `.alert`. Reach for
  custom drawing only for content the framework does not provide.
- **State** lives in `HFSAppModel` (`@MainActor @Observable`), injected with
  `.environment(_:)` and read with `@Environment(HFSAppModel.self)`.
- **Networking** belongs in `HFSOperations`; keep `HFSClient` small (URL/headers/
  encoding/CapabilityStatement only).
- **Layout safety**: never put a hard `.frame(width:)` on a child of a
  split-managed/resizable container (a `NavigationSplitView` column or an
  `.inspector`). Use flexible `min/ideal/max` widths — a fixed width that gets
  squeezed below its minimum makes AppKit report `minWidth > maxWidth` during a
  divider drag and abort the app.
- **Tests** inject a fake `HFSHTTPTransport` (or build an `HFSClient` over a stub
  transport) so they run without a live server.
