import SwiftUI

/// The app's sections.
enum AppTab: String, CaseIterable, Hashable {
    case map, spots, trips, log, settings
}

/// The five sections: a tab bar on the iPhone and the iPad (the iPad can still switch to the sidebar
/// from it, so the Spots list is not covered by default). The welcome comes up on the first launch.
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
                TripsTab()
            }
            Tab("Log", systemImage: "book.closed", value: AppTab.log) {
                LogTab()
            }
            Tab("Settings", systemImage: "gearshape", value: AppTab.settings) {
                SettingsView()
            }
        }
        .tabViewStyle(.sidebarAdaptable)
        .defaultAdaptableTabBarPlacement(.tabBar)
        .onAppear {
            showsWelcome = LaunchArguments.showsWelcome(seen: welcomeSeen)
        }
        .sheet(isPresented: $showsWelcome, onDismiss: { welcomeSeen = true }) {
            WelcomeView {
                welcomeSeen = true
                showsWelcome = false
            }
        }
        // Buttons and links in a blue that small text can be read in, the welcome's too.
        .tint(Palette.accent)
        #if DEBUG
            .widgetGalleryOnLaunch()
        #endif
    }
}

/// A section that is not there yet.
private struct ComingSection: View {
    let title: String
    let systemImage: String
    let text: String

    var body: some View {
        NavigationStack {
            VStack(spacing: 12) {
                Image(systemName: systemImage)
                    .font(.largeTitle)
                    .foregroundStyle(.readableSecondary)
                    .accessibilityHidden(true)
                Text("\(title) is coming")
                    .font(.title2.bold())
                    .multilineTextAlignment(.center)
                Text(text)
                    .foregroundStyle(.readableSecondary)
                    .multilineTextAlignment(.center)
            }
            .padding()
            .frame(maxWidth: 420)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .accessibilityElement(children: .combine)
            .navigationTitle(title)
        }
    }
}

/// The launch arguments the app reads, in debug builds only (screenshots, UI tests):
/// `-open-spot <id>` opens that spot, `-tab <map|spots|trips|log|settings>` opens that section, `-welcome` shows the welcome,
/// `-seed` (a screenshot run) or `-sample` (a UI test) skips the first launch's welcome, and
/// `-sunMode` shows sun mode for that launch only.
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

    /// `-sunMode` shows sun mode for that launch, without saving it.
    static var sunMode: Bool { arguments.contains("-sunMode") }

    /// Whether to show the welcome: asked for with `-welcome`, else on the first launch
    /// outside screenshot runs.
    static func showsWelcome(seen: Bool) -> Bool {
        let all = arguments
        if all.contains("-welcome") { return true }
        return !seen && !all.contains("-seed") && !all.contains("-sample")
    }
}
