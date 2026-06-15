import SwiftUI

/// A small colored capsule that summarizes a job's lifecycle state.
///
/// Custom because SwiftUI has no first-party "status pill"; it is composed from
/// first-party primitives (`Label`, `Capsule`, `.tint`) so it still adopts the
/// system palette and Dynamic Type.
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
        Label {
            Text(label)
                .font(.caption.weight(.medium))
        } icon: {
            Image(systemName: kind.systemImage)
                .symbolRenderingMode(.hierarchical)
                .imageScale(.small)
        }
        .labelStyle(.titleAndIcon)
        .padding(.horizontal, 8)
        .padding(.vertical, 3)
        .background(kind.tint.opacity(0.15), in: Capsule())
        .foregroundStyle(kind.tint)
    }

    private var label: String {
        if let detail, !detail.isEmpty {
            return "\(kind.title) · \(detail)"
        }
        return kind.title
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
