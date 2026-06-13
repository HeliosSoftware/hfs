import SwiftUI

struct HFSWorkspacePanel: View {
    var destination: HFSAdminDestination

    var body: some View {
        GroupBox("\(destination.title) Workspace") {
            VStack(alignment: .leading, spacing: 12) {
                ForEach(destination.workspaceRows) { row in
                    Label {
                        VStack(alignment: .leading, spacing: 3) {
                            Text(row.title)
                                .font(.callout.weight(.medium))
                            Text(row.detail)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    } icon: {
                        Image(systemName: row.systemImage)
                            .foregroundStyle(.secondary)
                    }
                    .padding(.vertical, 4)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}
