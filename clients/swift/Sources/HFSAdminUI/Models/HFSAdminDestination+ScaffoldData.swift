extension HFSAdminDestination {
    var tiles: [HFSPlaceholderTileModel] {
        switch self {
        case .overview:
            [
                .init(title: "Capability", value: "--", caption: "Awaiting /metadata", systemImage: "doc.text.magnifyingglass"),
                .init(title: "Storage", value: "--", caption: "Backend status", systemImage: "externaldrive"),
                .init(title: "Tenants", value: "1", caption: "Default context", systemImage: "building.2")
            ]
        case .resources:
            [
                .init(title: "Resource Types", value: "--", caption: "From CapabilityStatement", systemImage: "folder"),
                .init(title: "Recent Reads", value: "0", caption: "Session activity", systemImage: "clock"),
                .init(title: "Pending Writes", value: "0", caption: "Draft changes", systemImage: "square.and.pencil")
            ]
        case .search:
            [
                .init(title: "Saved Queries", value: "0", caption: "Local workspace", systemImage: "bookmark"),
                .init(title: "Parameters", value: "--", caption: "Selected resource type", systemImage: "slider.horizontal.3"),
                .init(title: "Results", value: "0", caption: "Current search", systemImage: "list.bullet")
            ]
        case .bulkJobs:
            [
                .init(title: "Exports", value: "0", caption: "Tracked jobs", systemImage: "arrow.up.doc"),
                .init(title: "Submits", value: "0", caption: "Tracked ingestions", systemImage: "arrow.down.doc"),
                .init(title: "Manifests", value: "0", caption: "Available outputs", systemImage: "doc.plaintext")
            ]
        case .audit:
            [
                .init(title: "Events", value: "0", caption: "Loaded AuditEvents", systemImage: "list.bullet.clipboard"),
                .init(title: "Filters", value: "0", caption: "Active filters", systemImage: "line.3.horizontal.decrease.circle"),
                .init(title: "Sources", value: "--", caption: "Observed systems", systemImage: "point.3.connected.trianglepath.dotted")
            ]
        case .subscriptions:
            [
                .init(title: "Active", value: "0", caption: "Running subscriptions", systemImage: "dot.radiowaves.left.and.right"),
                .init(title: "Channels", value: "--", caption: "Configured delivery", systemImage: "network"),
                .init(title: "Events", value: "0", caption: "Recent notifications", systemImage: "bell")
            ]
        case .settings:
            [
                .init(title: "Server", value: "1", caption: "Connection profile", systemImage: "server.rack"),
                .init(title: "Auth", value: "Off", caption: "Token provider", systemImage: "lock"),
                .init(title: "Version", value: "R4", caption: "Default FHIR version", systemImage: "cross.case")
            ]
        }
    }

    var workspaceRows: [HFSWorkspaceRow] {
        switch self {
        case .overview:
            [
                .init(title: "CapabilityStatement", detail: "Server metadata will populate the overview header and capability tiles.", systemImage: "doc.text"),
                .init(title: "Runtime Profile", detail: "Storage backend, tenant mode, terminology, and feature availability belong here.", systemImage: "gauge.with.dots.needle.50percent")
            ]
        case .resources:
            [
                .init(title: "Resource Browser", detail: "A resource-type list, paged Bundle results, and JSON detail panel will share this workspace.", systemImage: "rectangle.split.3x1"),
                .init(title: "Editor Surface", detail: "Create, update, patch, and delete actions can be layered in without changing navigation.", systemImage: "square.and.pencil")
            ]
        case .search:
            [
                .init(title: "Query Builder", detail: "Search parameters, modifiers, includes, and result count controls will live in one aligned form.", systemImage: "slider.horizontal.3"),
                .init(title: "Result Inspector", detail: "Bundle rows and selected resource JSON will use the same detail components as Resources.", systemImage: "doc.richtext")
            ]
        case .bulkJobs:
            [
                .init(title: "Job List", detail: "Export and bulk-submit kickoff, polling, manifest, and download state will be grouped here.", systemImage: "arrow.triangle.2.circlepath"),
                .init(title: "Manifest Detail", detail: "NDJSON outputs, errors, file counts, and completion metadata can render in a table.", systemImage: "tablecells")
            ]
        case .audit:
            [
                .init(title: "AuditEvent Timeline", detail: "Recorded events, actors, entities, outcomes, and request correlation filters belong here.", systemImage: "timeline.selection"),
                .init(title: "Event Detail", detail: "Selected events can expand into structured BALP and raw FHIR views.", systemImage: "doc.badge.ellipsis")
            ]
        case .subscriptions:
            [
                .init(title: "Subscription List", detail: "Topic, channel, status, endpoint, and criteria summaries will align in this workspace.", systemImage: "list.bullet.rectangle"),
                .init(title: "Delivery State", detail: "Status, events, websocket binding, and retry diagnostics can share the detail panel.", systemImage: "antenna.radiowaves.left.and.right")
            ]
        case .settings:
            [
                .init(title: "Connection", detail: "Base URL, tenant, FHIR version, and request defaults should be owned by settings state.", systemImage: "link"),
                .init(title: "Authentication", detail: "Static bearer tokens and SMART discovery can be added behind the auth abstraction.", systemImage: "person.badge.key")
            ]
        }
    }
}
