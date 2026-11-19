import SwiftUI
@preconcurrency import UnderbiteKit

extension View {
    /// Puts the trip's Live Activity under the view (the trip page's header) on one of the
    /// trip's days: the button that starts it, or the one that stops it while it runs. Where
    /// the system does not let the app start one, nothing is shown.
    func tripActivityOffer(trips: TripStore, tripID: String, view: ViewModel.TripView) -> some View {
        modifier(TripActivityOffer(trips: trips, tripID: tripID, view: view))
    }
}

/// The trip page's Live Activity entry. Built again with the page's view (each fetch, a new
/// day) and once the day's activity is over; a running activity is updated with it.
private struct TripActivityOffer: ViewModifier {
    @Environment(SpotStore.self) private var spots
    let trips: TripStore
    let tripID: String
    let view: ViewModel.TripView

    @State private var offer: TripActivityMatch.Offer = .hidden
    @State private var content: ViewModel.TripDayActivity?
    /// Raised when the content's day is over, to look again.
    @State private var checks = 0

    func body(content page: Content) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            page
            switch offer {
            case .hidden:
                EmptyView()
            case .offer:
                Button(action: start) {
                    Label("Show today's times on the Lock Screen", systemImage: "timer")
                }
                .buttonStyle(.bordered)
            case .running:
                HStack(alignment: .firstTextBaseline) {
                    Label("Today's times are on the Lock Screen", systemImage: "timer")
                        .foregroundStyle(.readableSecondary)
                    Spacer(minLength: 8)
                    Button("Stop", action: stop)
                }
                .font(.subheadline)
            }
        }
        .task(id: "\(view.hashValue) \(checks)") { await reconsider() }
    }

    /// Builds today's content off the main actor and updates the offer, and the running
    /// activity with it (or ends it when the day has none); then waits for the day's end.
    private func reconsider() async {
        guard TripActivityController.enabled, TripActivityContent.isTripDay(view),
            let trip = trips.trip(tripID)
        else {
            offer = .hidden
            if TripActivityController.isRunning(tripID) { await TripActivityController.stop(tripID) }
            return
        }
        let built = await build(trip)
        guard !Task.isCancelled else { return }
        content = built
        let running = TripActivityController.isRunning(tripID)
        offer = TripActivityMatch.offer(enabled: true, content: built, running: running, now: .now)
        if running {
            if offer == .running, let built {
                await TripActivityController.show(tripID, content: built)
            } else {
                await TripActivityController.stop(tripID)
            }
        }
        // Look again once the day's activity is over (after sunset).
        guard let built, offer != .hidden else { return }
        let left = built.endDate.timeIntervalSinceNow
        guard (try? await Task.sleep(for: .seconds(max(left, 0) + 1))) != nil else { return }
        checks += 1
    }

    /// Today's content for `trip` now, built off the main actor from the places' last snapshots.
    private func build(_ trip: SavedTrip) async -> ViewModel.TripDayActivity? {
        let now = Int64(Date().timeIntervalSince1970)
        let snapshots = TripActivityController.snapshots(of: trip, spots: spots, now: now)
        let settings = spots.settingsJSON
        return await Task.detached(priority: .userInitiated) {
            TripActivityContent.activity(
                trip, settingsJSON: settings, snapshot: { snapshots[$0] }, files: .standard, now: now)
        }.value
    }

    /// Starts the activity with the content as it is now (the page may have been open a while).
    private func start() {
        guard let trip = trips.trip(tripID) else { return }
        Task {
            guard let fresh = await build(trip), TripActivityMatch.shows(fresh, at: .now) else {
                offer = .hidden
                return
            }
            content = fresh
            if await TripActivityController.show(tripID, content: fresh) { offer = .running }
        }
    }

    private func stop() {
        Task {
            await TripActivityController.stop(tripID)
            offer = content.map {
                TripActivityMatch.offer(enabled: TripActivityController.enabled, content: $0, running: false, now: .now)
            } ?? .hidden
        }
    }
}
