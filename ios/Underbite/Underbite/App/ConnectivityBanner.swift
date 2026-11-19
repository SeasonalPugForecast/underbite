import SwiftUI

/// The line over every section while the last fetch found no signal: the core's offline note
/// ("offline: computed from harmonic constants, other values from Nov 18, 2:40 PM"), whole. It
/// comes and goes with fetch outcomes only, never with the route alone.
struct ConnectivityBanner: View {
    let text: String

    var body: some View {
        Label {
            Text(text)
                .fixedSize(horizontal: false, vertical: true)
        } icon: {
            Image(systemName: "wifi.slash")
                .foregroundStyle(Palette.warning)
        }
        .font(.footnote)
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .background(.bar)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("connectivity-banner")
    }
}

/// Which note a section's banner follows.
enum ConnectivityScope {
    /// The conditions of the shown spot: that spot's own note.
    case shownSpot
    /// A section that is not one spot's: a note only when no spot's fetch got an answer.
    case general
}

extension View {
    /// The connectivity banner over this section while the last fetch found no signal. Each
    /// section carries its own, inside the tab view, so no tab bar covers it; every window reads
    /// the same note.
    func connectivityBanner(_ scope: ConnectivityScope = .general) -> some View {
        modifier(ConnectivityInset(scope: scope))
    }
}

/// The banner as an inset at the bottom of a section, from the store every window shares. At the
/// top it would take the place of the section's navigation bar on the iPhone, and sit under the
/// floating tab bar on the iPad.
private struct ConnectivityInset: ViewModifier {
    @Environment(SpotStore.self) private var store
    let scope: ConnectivityScope

    func body(content: Content) -> some View {
        content.safeAreaInset(edge: .bottom, spacing: 0) {
            if let note = scope == .shownSpot ? store.offlineNote : store.generalOfflineNote {
                ConnectivityBanner(text: note)
            }
        }
    }
}

/// The app's one watch of the route: once it comes back and holds, the spots whose last fetch
/// found no signal are fetched again, so the banner goes as soon as one answers.
@MainActor
enum ConnectivityWatch {
    private static var watch: RouteReturn?

    /// Starts the watch for `store`, once per process (every window shares it). Not in a debug
    /// launch with `-offline`, whose fetches never get an answer.
    static func start(_ store: SpotStore) {
        guard watch == nil, !NetworkRoute.launchOffline else { return }
        let watch = RouteReturn { await store.refreshWaiting() }
        watch.start()
        self.watch = watch
    }
}
