#if DEBUG
    import SwiftUI
    import UIKit
    @preconcurrency import UnderbiteKit

    extension View {
        /// `-liveActivityPreview` (debug builds, screenshots): the trip-day Live Activity's views
        /// over the app, for the sample trip moved to start today, from the test places' last
        /// snapshots.
        func tripActivityPreviewOnLaunch() -> some View {
            modifier(TripActivityPreviewLaunch())
        }
    }

    private struct TripActivityPreviewLaunch: ViewModifier {
        @State private var shown = ProcessInfo.processInfo.arguments.contains("-liveActivityPreview")

        func body(content: Content) -> some View {
            content.fullScreenCover(isPresented: $shown) { TripActivityPreview() }
        }
    }

    /// The Lock Screen view as it is now and once stale and, on the iPhone, the Dynamic Island's
    /// compact and expanded views, drawn as the system frames them, for the sample trip
    /// (`TripSeeds`) moved to start today. Its content is the core's, built the way the trip
    /// page builds it, again whenever The Battery is fetched.
    private struct TripActivityPreview: View {
        @Environment(SpotStore.self) private var spots
        @State private var content: ViewModel.TripDayActivity?

        private let phone = UIDevice.current.userInterfaceIdiom == .phone
        private let theme = ThemeStore.shared.resolved()

        var body: some View {
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    if let content {
                        caption("Lock Screen")
                        lockScreen(content, stale: false)
                        caption("Lock Screen, out of date")
                        lockScreen(content, stale: true)
                        if phone {
                            caption("Dynamic Island")
                            compact(content)
                            expanded(content)
                        }
                    } else {
                        ProgressView().frame(maxWidth: .infinity)
                    }
                }
                .frame(maxWidth: 420)
                .padding(16)
                .frame(maxWidth: .infinity)
            }
            .background(wallpaper.ignoresSafeArea())
            .environment(\.colorTheme, theme)
            .task(id: spots.listing(for: SeedSpots.battery.id)?.status.lastSuccess) { build() }
        }

        private var wallpaper: LinearGradient {
            LinearGradient(
                colors: [Color(red: 0.16, green: 0.30, blue: 0.42), Color(red: 0.55, green: 0.42, blue: 0.36)],
                startPoint: .top, endPoint: .bottom)
        }

        private func caption(_ text: String) -> some View {
            Text(text).font(.footnote.weight(.semibold)).foregroundStyle(.white.opacity(0.85))
        }

        /// As the extension draws it on the Lock Screen: its tint, white words, dark colours.
        private func lockScreen(_ state: ViewModel.TripDayActivity, stale: Bool) -> some View {
            TripActivityLockScreen(state: state, stale: stale)
                .foregroundStyle(.white)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(TripActivityStyle.tint, in: .rect(cornerRadius: 22))
                .environment(\.colorScheme, .dark)
        }

        private func compact(_ state: ViewModel.TripDayActivity) -> some View {
            HStack {
                TripActivityIslandSymbol(state: state, stale: false)
                Spacer(minLength: 126)
                TripActivityIslandTimer(state: state, stale: false)
            }
            .foregroundStyle(.white)
            .padding(.horizontal, 14)
            .frame(width: 250, height: 37)
            .background(.black, in: .capsule)
            .frame(maxWidth: .infinity)
            .environment(\.colorScheme, .dark)
        }

        private func expanded(_ state: ViewModel.TripDayActivity) -> some View {
            VStack(alignment: .leading, spacing: 8) {
                HStack(alignment: .top) {
                    TripActivityIslandHead(state: state)
                    Spacer()
                    TripActivityIslandMark(state: state, stale: false)
                }
                TripActivityIslandBody(state: state, stale: false)
            }
            .foregroundStyle(.white)
            .padding(.horizontal, 22)
            .padding(.vertical, 16)
            .background(.black, in: .rect(cornerRadius: 44))
            .environment(\.colorScheme, .dark)
        }

        private func build() {
            let now = Int64(Date().timeIntervalSince1970)
            let today = TripDates.day(of: Date())
            // The sample trip starts three days after the day it is given.
            let json = TripSeeds.trip(
                today: TripDates.adding(-3, to: today) ?? today, spots: spots.spots, areas: spots.areas)
            let trip = SavedTrip(id: TripSeeds.id, json: json)
            let held = TripActivityController.snapshots(of: trip, spots: spots, now: now)
            content = TripActivityContent.activity(
                trip, settingsJSON: spots.settingsJSON, snapshot: { held[$0] }, files: .standard, now: now)
        }
    }
#endif
