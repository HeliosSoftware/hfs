import SwiftUI

struct HFSConnectionSummary: View {
    @Environment(HFSAppModel.self) private var model

    var body: some View {
        Section {
            LabeledContent("Server") {
                Text(model.serverName ?? "HFS Server")
            }

            LabeledContent("Status") {
                Text(model.statusText)
                    .foregroundStyle(.secondary)
            }

            Text(model.serverURLString)
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
        } header: {
            Text("Connection")
        }
    }
}
