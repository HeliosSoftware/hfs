import SwiftUI

/// An export-job status pill, mapping `BulkExportJob` state onto `HFSStatusBadge`.
struct HFSJobStatusBadge: View {
    enum Kind {
        case running
        case completed
        case failed
        case cancelled

        var title: String {
            switch self {
            case .running: "Running"
            case .completed: "Completed"
            case .failed: "Failed"
            case .cancelled: "Cancelled"
            }
        }

        var systemImage: String {
            switch self {
            case .running: "arrow.triangle.2.circlepath"
            case .completed: "checkmark.circle.fill"
            case .failed: "exclamationmark.triangle.fill"
            case .cancelled: "slash.circle"
            }
        }

        var tint: Color {
            switch self {
            case .running: .blue
            case .completed: .green
            case .failed: .red
            case .cancelled: .secondary
            }
        }
    }

    let kind: Kind
    /// Optional trailing detail, e.g. the server's progress string.
    var detail: String?

    var body: some View {
        HFSStatusBadge(
            title: kind.title,
            systemImage: kind.systemImage,
            tint: kind.tint,
            detail: detail
        )
    }
}

#Preview {
    VStack(alignment: .leading, spacing: 8) {
        HFSJobStatusBadge(kind: .running, detail: "45%")
        HFSJobStatusBadge(kind: .completed)
        HFSJobStatusBadge(kind: .failed)
        HFSJobStatusBadge(kind: .cancelled)
    }
    .padding()
}
