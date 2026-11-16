import SwiftUI
import UnderbiteKit

/// The frame every card shares: its title, the content, the reason when the value is greyed,
/// the fixed notices, the provenance footnote and the credits. Every word comes from the view
/// model; notices and reasons are safety text and are never shortened.
struct CardFrame<Content: View>: View {
    let chrome: ViewModel.CardChrome
    var showsChevron = true
    @ViewBuilder var content: Content
    @Environment(\.sunMode) private var sunMode

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text(chrome.title)
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(.readableSecondary)
                Spacer(minLength: 0)
                if showsChevron {
                    Image(systemName: "chevron.right")
                        .font(.footnote.weight(.semibold))
                        .foregroundStyle(.readableTertiary)
                        .accessibilityHidden(true)
                }
            }
            .accessibilityHidden(true)
            content
                .greyed(CardStyle.isGreyed(chrome.state))
                .accessibilityElement(children: .combine)
                .accessibilityLabel(Text(chrome.accessibility))
            StateReason(state: chrome.state)
            NoticeList(notices: chrome.notices)
            if let footnote = chrome.footnote {
                Text(footnote)
                    .font(.caption)
                    .foregroundStyle(.readableSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            CreditsLine(credits: chrome.credits)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(14)
        .background(
            Color(.secondarySystemGroupedBackground),
            in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .overlay {
            if sunMode {
                RoundedRectangle(cornerRadius: 16, style: .continuous)
                    .strokeBorder(Palette.cardStroke(sunMode), lineWidth: 2)
            }
        }
        .contentShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
    }
}

/// What a card's state does to its look; the reason itself is the view model's text.
enum CardStyle {
    /// A value that is too old, from too far away or missing is drawn greyed.
    static func isGreyed(_ state: ViewModel.CardState) -> Bool {
        switch state {
        case .stale, .far, .unavailable: true
        case .fresh, .unknown: false
        }
    }
}

extension View {
    /// A greyed value: drawn in the secondary colour and without its colours (a sparkline goes
    /// grey), never faded, so its text keeps a readable contrast; `StateReason` says why.
    @ViewBuilder func greyed(_ on: Bool) -> some View {
        if on {
            foregroundStyle(.readableSecondary).saturation(0)
        } else {
            self
        }
    }
}

/// Why a value is greyed ("older than 3 h", "12 mi away"), in the view model's words.
struct StateReason: View {
    let state: ViewModel.CardState

    var body: some View {
        if let reason = state.reason, !reason.isEmpty {
            Label(reason, systemImage: "exclamationmark.circle")
                .font(.caption)
                .foregroundStyle(Palette.warning)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

/// The fixed notices ("Not for navigation.", "Provisional data, subject to revision."): plain
/// text, whole, never in Luna's voice.
struct NoticeList: View {
    let notices: [ViewModel.Notice]
    /// Primary rather than secondary text, on a tinted background.
    var onTint = false

    var body: some View {
        ForEach(Array(notices.enumerated()), id: \.offset) { _, notice in
            Text(notice.text)
                .font(.caption.weight(.medium))
                .foregroundStyle(onTint ? AnyShapeStyle(Color.primary) : AnyShapeStyle(.readableSecondary))
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

/// The sources a card owes credit to, one line.
struct CreditsLine: View {
    let credits: [ViewModel.Credit]
    /// Primary rather than secondary text, on a tinted background.
    var onTint = false

    var body: some View {
        if !credits.isEmpty {
            Text(Self.joined(credits))
                .font(.caption)
                .foregroundStyle(onTint ? AnyShapeStyle(Color.primary) : AnyShapeStyle(.readableSecondary))
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    /// The credits' own texts, each once, in order.
    static func joined(_ credits: [ViewModel.Credit]) -> String {
        var seen = Set<String>()
        return credits.map(\.text).filter { seen.insert($0).inserted }.joined(separator: " · ")
    }
}
