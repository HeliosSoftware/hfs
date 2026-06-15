import HFSOperations
import SwiftUI

/// A small colored capsule that conveys a `Subscription.status`.
///
/// Custom because SwiftUI has no first-party status pill; it is composed from
/// first-party primitives (`Label`, `Capsule`, `.tint`) so it still adopts the
/// system palette and Dynamic Type. Sibling of `HFSJobStatusBadge`.
struct HFSSubscriptionStatusBadge: View {
    /// The parsed status, or `nil` when the subscription has an unknown code.
    let status: SubscriptionStatus?

    var body: some View {
        Label {
            Text(title)
                .font(.caption.weight(.medium))
        } icon: {
            Image(systemName: systemImage)
                .symbolRenderingMode(.hierarchical)
                .imageScale(.small)
        }
        .labelStyle(.titleAndIcon)
        .padding(.horizontal, 8)
        .padding(.vertical, 3)
        .background(tint.opacity(0.15), in: Capsule())
        .foregroundStyle(tint)
    }

    private var title: String { status?.label ?? "Unknown" }

    private var systemImage: String {
        switch status {
        case .active: "dot.radiowaves.left.and.right"
        case .requested: "clock"
        case .error: "exclamationmark.triangle.fill"
        case .off: "pause.circle"
        case nil: "questionmark.circle"
        }
    }

    private var tint: Color {
        switch status {
        case .active: .green
        case .requested: .blue
        case .error: .red
        case .off: .secondary
        case nil: .secondary
        }
    }
}

#Preview {
    VStack(alignment: .leading, spacing: 8) {
        HFSSubscriptionStatusBadge(status: .active)
        HFSSubscriptionStatusBadge(status: .requested)
        HFSSubscriptionStatusBadge(status: .error)
        HFSSubscriptionStatusBadge(status: .off)
        HFSSubscriptionStatusBadge(status: nil)
    }
    .padding()
}
