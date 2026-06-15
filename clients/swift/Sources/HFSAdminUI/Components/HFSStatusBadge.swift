import SwiftUI

/// A small colored capsule "status pill" — SwiftUI has no first-party
/// equivalent, so this composes one from first-party primitives (`Label`,
/// `Capsule`, `.tint`) and adopts the system palette and Dynamic Type.
///
/// Domain badges (`HFSJobStatusBadge`, `HFSSubscriptionStatusBadge`) map their
/// state onto this shared appearance.
struct HFSStatusBadge: View {
    let title: String
    let systemImage: String
    let tint: Color
    /// Optional trailing detail, e.g. a progress string.
    var detail: String?

    var body: some View {
        Label {
            Text(label)
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

    private var label: String {
        if let detail, !detail.isEmpty {
            return "\(title) · \(detail)"
        }
        return title
    }
}

#Preview {
    VStack(alignment: .leading, spacing: 8) {
        HFSStatusBadge(title: "Active", systemImage: "checkmark.circle.fill", tint: .green)
        HFSStatusBadge(title: "Running", systemImage: "arrow.triangle.2.circlepath", tint: .blue, detail: "45%")
        HFSStatusBadge(title: "Error", systemImage: "exclamationmark.triangle.fill", tint: .red)
    }
    .padding()
}
