import HFSAdminUI
import SwiftUI

@main
struct HFSAdminApp: App {
    var body: some Scene {
        WindowGroup {
            HFSAdminRootView()
                .frame(minWidth: 1240, minHeight: 760)
        }
        .defaultSize(width: 1440, height: 860)
        .windowStyle(.titleBar)
        .windowToolbarStyle(.unified)
        .windowResizability(.contentMinSize)
    }
}
