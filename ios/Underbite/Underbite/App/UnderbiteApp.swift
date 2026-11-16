import SwiftUI

@main
struct UnderbiteApp: App {
    @State private var store = UnderbiteApp.makeStore()
    @Environment(\.scenePhase) private var phase

    var body: some Scene {
        WindowGroup {
            RootTabs()
                .environment(store)
                .task { await store.refreshOnOpen() }
                .onChange(of: phase) { _, phase in
                    switch phase {
                    case .active: Task { await store.refreshOnOpen() }
                    case .background: store.saveSettingsNow()
                    default: break
                    }
                }
        }
    }

    /// The one store of the process; `-seed` adds the test places (debug builds).
    private static func makeStore() -> SpotStore {
        let store = SpotStore.shared
        #if DEBUG
            if ProcessInfo.processInfo.arguments.contains("-seed") { store.addSeedSpots() }
        #endif
        return store
    }
}
