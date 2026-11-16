import SwiftUI

/// The preference pages the Settings root links to. Present them from a navigation stack with
/// `NavigationLink(value:)` and `navigationDestination(for: PreferencePageID.self)`, or list
/// them all with `PreferencesLinks`.
enum PreferencePageID: String, CaseIterable, Hashable, Identifiable {
    case units, limits, rivers, fishing, freshness, refresh, voice, display, privacy

    var id: String { rawValue }

    var title: String {
        switch self {
        case .units: "Units"
        case .limits: "Go/no-go limits"
        case .rivers: "River ranges"
        case .fishing: "Trout and fishing windows"
        case .freshness: "Freshness and distance"
        case .refresh: "Refresh"
        case .voice: "Voice"
        case .display: "Sun mode"
        case .privacy: "Privacy"
        }
    }

    var systemImage: String {
        switch self {
        case .units: "ruler"
        case .limits: "gauge.with.dots.needle.33percent"
        case .rivers: "water.waves"
        case .fishing: "fish"
        case .freshness: "clock.arrow.circlepath"
        case .refresh: "arrow.clockwise"
        case .voice: "text.bubble"
        case .display: "sun.max"
        case .privacy: "lock"
        }
    }

    @ViewBuilder
    var destination: some View {
        switch self {
        case .units: UnitsPage()
        case .limits: LimitsPage()
        case .rivers: RiversPage()
        case .fishing: FishingPage()
        case .freshness: FreshnessPage()
        case .refresh: RefreshPage()
        case .voice: VoicePage()
        case .display: SunModePage()
        case .privacy: PrivacyPage()
        }
    }

    /// The page `-preferences <name>` asks for, in debug builds only (screenshots).
    static var launchRequest: PreferencePageID? {
        #if DEBUG
            let all = ProcessInfo.processInfo.arguments
            guard let at = all.firstIndex(of: "-preferences"), at + 1 < all.count else { return nil }
            return PreferencePageID(rawValue: all[at + 1])
        #else
            nil
        #endif
    }
}

/// A row for each preference page, for a list in the Settings root.
struct PreferencesLinks: View {
    var body: some View {
        ForEach(PreferencePageID.allCases) { page in
            NavigationLink(value: page) {
                Label(page.title, systemImage: page.systemImage)
            }
        }
    }
}

extension View {
    /// Makes `PreferencePageID` values navigable in the enclosing stack.
    func preferenceDestinations() -> some View {
        navigationDestination(for: PreferencePageID.self) { $0.destination }
    }
}
