import SwiftUI
import UnderbiteKit

// The trip-day Live Activity's views, shared by the extension and the app's debug preview.
// Every word and every instant is the core's (`ViewModel.TripDayActivity`). The countdowns are
// the system's timer text counting to the core's instants, the one thing drawn here that the
// core did not word. Once the content is stale (the system's `isStale`), the countdowns give
// way to the times as the core words them, under its out-of-date line.

/// One of the times the activity counts down to, with its symbol and colour.
struct TripActivityItem: Identifiable {
    enum Slot: String {
        case tide, slack, sunset
    }

    let slot: Slot
    let target: ViewModel.ActivityTarget

    var id: Slot { slot }

    var symbol: String {
        switch slot {
        case .tide: "water.waves"
        case .slack: "arrow.left.arrow.right"
        case .sunset: "sunset.fill"
        }
    }

    var role: ColorRole {
        switch slot {
        case .tide: .tideHigh
        case .slack: .tideSlack
        case .sunset: .timelineSun
        }
    }

    /// The items of `state`, in the core's order: the tide, the slack, the sunset.
    static func items(_ state: ViewModel.TripDayActivity) -> [TripActivityItem] {
        [
            state.nextTideChange.map { TripActivityItem(slot: .tide, target: $0) },
            state.slack.map { TripActivityItem(slot: .slack, target: $0) },
            state.sunset.map { TripActivityItem(slot: .sunset, target: $0) },
        ].compactMap { $0 }
    }
}

/// The background the Lock Screen view is drawn on, with white words.
enum TripActivityStyle {
    static let tint = Color.black.opacity(0.55)
    static let staleSymbol = "clock.badge.exclamationmark"
}

/// The system's countdown to `target`, from the content's build time on. Before that instant
/// the range would be empty, so it starts no later than the target.
struct TripActivityCountdown: View {
    let target: ViewModel.ActivityTarget
    let generatedAt: Int64

    var body: some View {
        let end = target.date
        let start = min(Date(timeIntervalSince1970: TimeInterval(generatedAt)), end)
        Text(timerInterval: start...end, countsDown: true)
            .monospacedDigit()
    }
}

/// One time: its symbol, then its label and countdown, or once stale the time in words.
struct TripActivityRow: View {
    let item: TripActivityItem
    let generatedAt: Int64
    let stale: Bool
    @Environment(\.colorTheme) private var theme

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Image(systemName: item.symbol)
                .foregroundStyle(theme.color(item.role))
                .frame(width: 22)
            if stale {
                Text(item.target.text)
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
                    .frame(maxWidth: .infinity, alignment: .leading)
            } else {
                Text(item.target.label)
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
                    .layoutPriority(1)
                TripActivityCountdown(target: item.target, generatedAt: generatedAt)
                    .fontWeight(.semibold)
                    .multilineTextAlignment(.trailing)
                    .frame(maxWidth: .infinity, alignment: .trailing)
            }
        }
    }
}

/// The core's out-of-date line, plainly: the times below it are no longer counted down.
struct TripActivityStaleLine: View {
    let text: String
    @Environment(\.colorTheme) private var theme

    var body: some View {
        Label {
            Text(text).fixedSize(horizontal: false, vertical: true)
        } icon: {
            Image(systemName: TripActivityStyle.staleSymbol)
                .foregroundStyle(theme.color(.cardStale))
        }
        .font(.footnote.weight(.semibold))
    }
}

/// The fixed notices ("Not for navigation."), whole, and the sources' credit.
struct TripActivityFooter: View {
    let state: ViewModel.TripDayActivity

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            if !state.notices.isEmpty {
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(Array(state.notices.enumerated()), id: \.offset) { _, notice in
                        Text(notice.text)
                            .fontWeight(.semibold)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
            Spacer(minLength: 0)
            if !state.credit.isEmpty {
                Text(state.credit)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.trailing)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .font(.caption2)
    }
}

/// The trip's name and the place the times are for.
struct TripActivityHeading: View {
    let state: ViewModel.TripDayActivity

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Text(state.title).fontWeight(.semibold).lineLimit(1)
            Text(state.place).foregroundStyle(.secondary).lineLimit(1)
        }
        .font(.footnote)
    }
}

/// The Lock Screen (and banner) view: the trip and place, each time with its countdown, the
/// notices and the credit. VoiceOver reads the core's sentence, and its out-of-date line once
/// stale.
struct TripActivityLockScreen: View {
    let state: ViewModel.TripDayActivity
    let stale: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            TripActivityHeading(state: state)
            if stale { TripActivityStaleLine(text: state.stale) }
            ForEach(TripActivityItem.items(state)) { item in
                TripActivityRow(item: item, generatedAt: state.generatedAt, stale: stale)
                    .font(.subheadline)
            }
            TripActivityFooter(state: state)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(state.accessibility))
        .accessibilityValue(stale ? Text(state.stale) : Text(verbatim: ""))
    }
}

// MARK: - The Dynamic Island

/// The symbol of the first time, or the out-of-date one: compact leading and minimal.
struct TripActivityIslandSymbol: View {
    let state: ViewModel.TripDayActivity
    let stale: Bool
    @Environment(\.colorTheme) private var theme

    var body: some View {
        let first = TripActivityItem.items(state).first
        Group {
            if stale {
                Image(systemName: TripActivityStyle.staleSymbol)
                    .foregroundStyle(theme.color(.cardStale))
            } else {
                Image(systemName: first?.symbol ?? "water.waves")
                    .foregroundStyle(theme.color(first?.role ?? .tideHigh))
            }
        }
        .accessibilityLabel(Text(stale ? state.stale : first?.target.text ?? state.place))
    }
}

/// The countdown to the first time (compact trailing); once stale, nothing is counted.
struct TripActivityIslandTimer: View {
    let state: ViewModel.TripDayActivity
    let stale: Bool

    var body: some View {
        if !stale, let first = TripActivityItem.items(state).first {
            TripActivityCountdown(target: first.target, generatedAt: state.generatedAt)
                .font(.caption.weight(.semibold))
                .multilineTextAlignment(.trailing)
                .frame(maxWidth: 64)
                .accessibilityLabel(Text(first.target.label))
        }
    }
}

/// The expanded view's top leading corner: the trip and place.
struct TripActivityIslandHead: View {
    let state: ViewModel.TripDayActivity

    var body: some View {
        VStack(alignment: .leading, spacing: 1) {
            Text(state.title).font(.caption.weight(.semibold)).lineLimit(1)
            Text(state.place).font(.caption2).foregroundStyle(.secondary).lineLimit(1)
        }
    }
}

/// The expanded view's top trailing corner: the first time's symbol, or the out-of-date one.
struct TripActivityIslandMark: View {
    let state: ViewModel.TripDayActivity
    let stale: Bool

    var body: some View {
        TripActivityIslandSymbol(state: state, stale: stale).font(.title3)
    }
}

/// The expanded view's bottom: every time, then the notices and the credit.
struct TripActivityIslandBody: View {
    let state: ViewModel.TripDayActivity
    let stale: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            if stale { TripActivityStaleLine(text: state.stale) }
            ForEach(TripActivityItem.items(state)) { item in
                TripActivityRow(item: item, generatedAt: state.generatedAt, stale: stale)
                    .font(.footnote)
            }
            TripActivityFooter(state: state)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(state.accessibility))
        .accessibilityValue(stale ? Text(state.stale) : Text(verbatim: ""))
    }
}
