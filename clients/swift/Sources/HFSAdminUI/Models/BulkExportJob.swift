import Foundation
import HFSOperations

/// A tracked Bulk Data `$export` job, owned by `HFSAppModel` so it survives
/// navigating away from the Bulk Jobs screen and keeps polling in the background.
struct BulkExportJob: Identifiable {
    enum State {
        case running(progress: String?)
        case completed(BulkExportManifest)
        case failed(String)
        case cancelled
    }

    let id = UUID()
    let level: BulkExportLevel
    let groupID: String?
    let types: [String]
    let since: String?
    let statusURL: URL
    let startedAt: Date
    var state: State

    var isRunning: Bool {
        if case .running = state { return true }
        return false
    }

    /// A one-line description of what was requested.
    var requestSummary: String {
        var parts = [level.label]
        if level == .group, let groupID { parts.append("Group/\(groupID)") }
        if !types.isEmpty { parts.append("_type=\(types.joined(separator: ","))") }
        if let since, !since.isEmpty { parts.append("_since=\(since)") }
        return parts.joined(separator: " · ")
    }

    var badgeKind: HFSJobStatusBadge.Kind {
        switch state {
        case .running: .running
        case .completed: .completed
        case .failed: .failed
        case .cancelled: .cancelled
        }
    }

    var badgeDetail: String? {
        switch state {
        case .running(let progress): progress
        default: nil
        }
    }
}
