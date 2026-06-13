import SwiftUI

struct HFSStatusStrip: View {
    @Environment(HFSAppModel.self) private var model

    var body: some View {
        GroupBox {
            HStack(spacing: 16) {
                LabeledContent("Connection") {
                    Text(model.statusText)
                        .foregroundStyle(.secondary)
                }

                Divider()
                    .frame(height: 24)

                LabeledContent("Tenant") {
                    Text(model.tenantDisplay)
                        .foregroundStyle(.secondary)
                }

                Divider()
                    .frame(height: 24)

                LabeledContent("FHIR") {
                    Text(model.fhirVersion.rawValue)
                        .foregroundStyle(.secondary)
                }

                Spacer(minLength: 0)
            }
        }
        .font(.callout)
    }
}
