import SwiftUI

/// The app's sections.
enum AppTab: String, CaseIterable, Hashable {
    case map, spots, trips, log, settings
}

/// The five sections: a tab bar on the iPhone (and in a narrow iPad window), a sidebar on the
/// iPad. The welcome comes up on the first launch.
struct RootTabs: View {
    @State private var selection = LaunchArguments.initialTab ?? .map
    @AppStorage("welcomeSeen") private var welcomeSeen = false
    @State private var showsWelcome = false

    var body: some View {
        TabView(selection: $selection) {
            Tab("Map", systemImage: "map", value: AppTab.map) {
                MapTab()
            }
            Tab("Spots", systemImage: "mappin.and.ellipse", value: AppTab.spots) {
                SpotsView()
            }
            Tab("Trips", systemImage: "calendar", value: AppTab.trips) {
                ComingSection(
                    title: "Trips", systemImage: "calendar",
                    text: "Plan a trip's days, with a checklist and maps for offline use.")
            }
            Tab("Log", systemImage: "book.closed", value: AppTab.log) {
                ComingSection(
                    title: "Log", systemImage: "book.closed",
                    text: "Keep your catches, where and when, and see what worked.")
            }
            Tab("Settings", systemImage: "gearshape", value: AppTab.settings) {
                SettingsView()
            }
        }
        .tabViewStyle(.sidebarAdaptable)
        .defaultAdaptableTabBarPlacement(.sidebar)
        .onAppear {
            showsWelcome = LaunchArguments.showsWelcome(seen: welcomeSeen)
        }
        .sheet(isPresented: $showsWelcome, onDismiss: { welcomeSeen = true }) {
            WelcomeView {
                welcomeSeen = true
                showsWelcome = false
            }
        }
    }
}

/// A section that is not there yet.
private struct ComingSection: View {
    let title: String
    let systemImage: String
    let text: String

    var body: some View {
        NavigationStack {
            ContentUnavailableView {
                Label("\(title) is coming", systemImage: systemImage)
            } description: {
                Text(text)
            }
            .navigationTitle(title)
        }
    }
}

/// The launch arguments the app reads, in debug builds only (screenshots, UI tests):
/// `-tab <map|spots|trips|log|settings>` opens that section, `-welcome` shows the welcome,
/// and `-seed` (a screenshot run) skips the first launch's welcome.
enum LaunchArguments {
    private static var arguments: [String] {
        #if DEBUG
            ProcessInfo.processInfo.arguments
        #else
            []
        #endif
    }

    /// The argument after `flag`, if any.
    static func value(after flag: String) -> String? {
        let all = arguments
        guard let index = all.firstIndex(of: flag), index + 1 < all.count else { return nil }
        return all[index + 1]
    }

    /// The section `-tab` asks for.
    static var initialTab: AppTab? {
        value(after: "-tab").flatMap(AppTab.init(rawValue:))
    }

    /// Whether to show the welcome: asked for with `-welcome`, else on the first launch
    /// outside screenshot runs.
    static func showsWelcome(seen: Bool) -> Bool {
        let all = arguments
        if all.contains("-welcome") { return true }
        return !seen && !all.contains("-seed")
    }
}
