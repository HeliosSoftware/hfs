import HFSOperations
import SwiftUI

/// A subscription status pill, mapping `Subscription.status` onto `HFSStatusBadge`.
struct HFSSubscriptionStatusBadge: View {
    /// The parsed status, or `nil` when the subscription has an unknown code.
    let status: SubscriptionStatus?

    var body: some View {
        HFSStatusBadge(title: title, systemImage: systemImage, tint: tint)
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
