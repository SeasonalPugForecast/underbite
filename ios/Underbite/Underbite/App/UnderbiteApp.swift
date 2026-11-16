import SwiftUI
import UnderbiteKit

@main
struct UnderbiteApp: App {
    @State private var store = UnderbiteApp.makeStore()
    @Environment(\.scenePhase) private var phase

    var body: some Scene {
        WindowGroup {
            RootTabs()
                .environment(store)
                .environment(\.voiceStyle, voice)
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

    /// The voice of the settings (`presentation.voice`); Luna's when they cannot be read.
    private var voice: VoiceStyle {
        VoiceStyle(setting: (try? Settings(json: store.settingsJSON).presentation.voice) ?? "luna")
    }

    /// The one store of the process; `-seed` adds the test places and `-sample` swaps in the
    /// offline sample (debug builds).
    private static func makeStore() -> SpotStore {
        #if DEBUG
            if SampleMode.isOn { return SpotStore.sample() }
            let store = SpotStore.shared
            if ProcessInfo.processInfo.arguments.contains("-seed") { store.addSeedSpots() }
            return store
        #else
            return SpotStore.shared
        #endif
    }
}
