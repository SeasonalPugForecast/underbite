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

extension View {
    /// The connectivity banner over this section while the last fetch found no signal. Each
    /// section carries its own, inside the tab view, so no tab bar covers it; every window reads
    /// the same note.
    func connectivityBanner() -> some View {
        modifier(ConnectivityInset())
    }
}

/// The banner as an inset of a section, from the store every window shares: at the top in a
/// compact width, where the tab bar is at the bottom; at the bottom in a regular width, where the
/// tab bar floats over the top.
private struct ConnectivityInset: ViewModifier {
    @Environment(SpotStore.self) private var store
    @Environment(\.horizontalSizeClass) private var sizeClass

    func body(content: Content) -> some View {
        content.safeAreaInset(edge: sizeClass == .regular ? .bottom : .top, spacing: 0) {
            if let note = store.offlineNote {
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
