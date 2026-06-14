import HFSAdminUI
import SwiftUI
#if os(macOS)
import AppKit
#endif

@main
struct HFSAdminApp: App {
    var body: some Scene {
        WindowGroup {
            HFSAdminRootView()
                .frame(minWidth: 1240, minHeight: 760)
                .onAppear {
                    activateAppIfNeeded()
                }
        }
        .defaultSize(width: 1440, height: 860)
        .windowStyle(.titleBar)
        .windowToolbarStyle(.unified)
        .windowResizability(.contentMinSize)
    }

    private func activateAppIfNeeded() {
        #if os(macOS)
        NSApplication.shared.setActivationPolicy(.regular)
        NSApplication.shared.activate(ignoringOtherApps: true)
        #endif
    }
}
