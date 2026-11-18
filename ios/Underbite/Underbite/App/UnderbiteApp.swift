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
                .sunMode(store.sunMode || LaunchArguments.sunMode)
                // While the app is open, due spots are fetched and the page built again every
                // minute; in the background nothing is.
                .onChange(of: phase, initial: true) { _, phase in
                    switch phase {
                    case .active: store.setForeground(true)
                    case .background:
                        store.setForeground(false)
                        store.saveSettingsNow()
                    default: break
                    }
                }
        }
    }

    /// The voice of the settings (`presentation.voice`); Luna's when they cannot be read.
    private var voice: VoiceStyle {
        VoiceStyle(setting: (try? Settings(json: store.settingsJSON).presentation.voice) ?? "luna")
    }

    /// The one store of the process; `-seed` adds the test places, `-seedLimits` a sample limits profile, `-seedTrip` a sample trip,
    /// `-seedCatches` sample catches and `-sample` swaps in the offline sample (debug builds).
    private static func makeStore() -> SpotStore {
        #if DEBUG
            if SampleMode.isOn { return SpotStore.sample() }
            let store = SpotStore.shared
            if ProcessInfo.processInfo.arguments.contains("-seed") { store.addSeedSpots() }
            if ProcessInfo.processInfo.arguments.contains("-seedLimits") { store.addSeedLimits() }
            if ProcessInfo.processInfo.arguments.contains("-seedTrip") { TripSeeds.seed(into: store) }
            if ProcessInfo.processInfo.arguments.contains("-seedCatches") { CatchSeeds.seed(into: store) }
            return store
        #else
            return SpotStore.shared
        #endif
    }
}
