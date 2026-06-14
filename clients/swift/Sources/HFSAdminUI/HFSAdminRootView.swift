import SwiftUI
#if canImport(AppKit)
import AppKit
#elseif canImport(UIKit)
import UIKit
#endif

@MainActor
public struct HFSAdminRootView: View {
    @State private var model = HFSAppModel()
    @State private var selection: HFSAdminDestination = .overview
    @State private var columnVisibility: NavigationSplitViewVisibility = .all
    @State private var pendingAction: HFSAdminDestination?
    @State private var newResourceRequested = false

    public init() {}

    public var body: some View {
        NavigationSplitView(columnVisibility: $columnVisibility) {
            HFSSidebar(selection: $selection)
                .navigationTitle("HFS Admin")
                .navigationSplitViewColumnWidth(min: 260, ideal: 280, max: 320)
        } detail: {
            HFSDetailContent(destination: selection, newResourceRequested: $newResourceRequested)
                .navigationTitle(selection.title)
                .toolbar {
                    ToolbarItemGroup(placement: .primaryAction) {
                        Button {
                            performPrimaryAction()
                        } label: {
                            Label(selection.primaryActionTitle, systemImage: selection.primaryActionIcon)
                        }

                        Menu {
                            Button("Connection Settings") {
                                selection = .settings
                            }
                            Button("Copy Current View Link") {
                                copyCurrentViewLink()
                            }
                        } label: {
                            Label("More", systemImage: "ellipsis.circle")
                        }
                    }
                }
        }
        .navigationSplitViewStyle(.balanced)
        .frame(minWidth: 1240, minHeight: 760)
        .environment(model)
        .task {
            if model.autoConnect, model.connectionState == .disconnected {
                await model.connect()
            }
        }
        .alert(
            pendingAction?.primaryActionTitle ?? "",
            isPresented: Binding(
                get: { pendingAction != nil },
                set: { if !$0 { pendingAction = nil } }
            ),
            presenting: pendingAction
        ) { _ in
            Button("OK", role: .cancel) {}
        } message: { action in
            Text("\u{201C}\(action.primaryActionTitle)\u{201D} requires a connected HFS server. Configure a connection in Settings to enable this action.")
        }
    }

    private func performPrimaryAction() {
        switch selection {
        case .settings:
            Task { await model.connect() }
        case .overview:
            Task { await model.refreshOverview() }
        case .resources:
            if model.isConnected {
                newResourceRequested = true
            } else {
                pendingAction = selection
            }
        default:
            pendingAction = selection
        }
    }

    private func copyCurrentViewLink() {
        let link = "helios-hfs-admin://\(selection.rawValue)"
        #if canImport(AppKit)
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(link, forType: .string)
        #elseif canImport(UIKit)
        UIPasteboard.general.string = link
        #endif
    }
}

#Preview {
    HFSAdminRootView()
}
