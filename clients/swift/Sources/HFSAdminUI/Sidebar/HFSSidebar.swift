import SwiftUI

struct HFSSidebar: View {
    @Binding var selection: HFSAdminDestination

    var body: some View {
        List(selection: $selection) {
            HFSConnectionSummary()

            ForEach(HFSSidebarSection.allCases) { section in
                Section(section.rawValue) {
                    ForEach(section.destinations) { destination in
                        Label(destination.title, systemImage: destination.systemImage)
                            .tag(destination)
                    }
                }
            }
        }
        .listStyle(.sidebar)
    }
}
