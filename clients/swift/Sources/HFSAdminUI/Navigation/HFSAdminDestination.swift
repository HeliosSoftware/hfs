enum HFSAdminDestination: String, CaseIterable, Identifiable, Hashable {
    case overview
    case resources
    case bulkJobs
    case audit
    case subscriptions
    case settings

    var id: String { rawValue }

    var title: String {
        switch self {
        case .overview: "Overview"
        case .resources: "Resources"
        case .bulkJobs: "Bulk Jobs"
        case .audit: "Audit"
        case .subscriptions: "Subscriptions"
        case .settings: "Settings"
        }
    }

    var section: HFSSidebarSection {
        switch self {
        case .overview, .settings:
            .server
        case .resources:
            .fhir
        case .bulkJobs, .audit, .subscriptions:
            .operations
        }
    }

    var systemImage: String {
        switch self {
        case .overview: "server.rack"
        case .resources: "folder"
        case .bulkJobs: "arrow.down.doc"
        case .audit: "list.bullet.clipboard"
        case .subscriptions: "dot.radiowaves.left.and.right"
        case .settings: "gearshape"
        }
    }

    var summary: String {
        switch self {
        case .overview:
            "Server capability, storage, and tenant status"
        case .resources:
            "Browse, inspect, create, and update FHIR resources"
        case .bulkJobs:
            "Monitor export and bulk-submit job lifecycle"
        case .audit:
            "Review AuditEvent records and operational activity"
        case .subscriptions:
            "Manage topic-based subscriptions and delivery state"
        case .settings:
            "Configure server URL, tenant, auth, and defaults"
        }
    }

    var primaryActionTitle: String {
        switch self {
        case .overview: "Refresh"
        case .resources: "New Resource"
        case .bulkJobs: "Refresh Jobs"
        case .audit: "Refresh Events"
        case .subscriptions: "New Subscription"
        case .settings: "Connect"
        }
    }

    var primaryActionIcon: String {
        switch self {
        case .overview, .bulkJobs, .audit:
            "arrow.clockwise"
        case .resources, .subscriptions:
            "plus"
        case .settings:
            "bolt.horizontal"
        }
    }
}
