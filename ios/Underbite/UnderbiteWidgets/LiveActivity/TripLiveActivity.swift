import ActivityKit
import SwiftUI
import UnderbiteKit
import WidgetKit

/// The trip-day Live Activity: the next tide change, the next slack (current stations only)
/// and the sunset, each counted down, with the credit and the notices, on the Lock Screen and
/// in the Dynamic Island. The app starts it from a trip's page on one of its days, updates it
/// after each refresh and ends it after sunset. Without an update it goes stale at the core's
/// `staleAt`, and then says so and shows the times in words.
struct TripLiveActivity: Widget {
    /// The app's colours, from the shared preferences, read each time a view is made so a change
    /// in the app shows on the next update.
    private static func theme() -> ColorTheme { ThemeStore.shared.resolved() }

    var body: some WidgetConfiguration {
        ActivityConfiguration(for: TripDayAttributes.self) { context in
            TripActivityLockScreen(state: context.state, stale: context.isStale)
                .foregroundStyle(.white)
                .environment(\.colorScheme, .dark)
                .environment(\.colorTheme, Self.theme())
                .activityBackgroundTint(TripActivityStyle.tint)
                .activitySystemActionForegroundColor(.white)
        } dynamicIsland: { context in
            let theme = Self.theme()
            let state = context.state
            let stale = context.isStale
            return DynamicIsland {
                DynamicIslandExpandedRegion(.leading) {
                    TripActivityIslandHead(state: state)
                }
                DynamicIslandExpandedRegion(.trailing) {
                    TripActivityIslandMark(state: state, stale: stale)
                        .environment(\.colorTheme, theme)
                }
                DynamicIslandExpandedRegion(.bottom) {
                    TripActivityIslandBody(state: state, stale: stale)
                        .environment(\.colorTheme, theme)
                }
            } compactLeading: {
                TripActivityIslandSymbol(state: state, stale: stale)
                    .environment(\.colorTheme, theme)
            } compactTrailing: {
                TripActivityIslandTimer(state: state, stale: stale)
            } minimal: {
                TripActivityIslandSymbol(state: state, stale: stale)
                    .environment(\.colorTheme, theme)
            }
            .keylineTint(theme.color(.tideHigh))
        }
    }
}
