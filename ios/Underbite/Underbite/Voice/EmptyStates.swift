import SwiftUI

/// What a screen has nothing of yet.
enum EmptyKind {
    case spots, conditions, trips, catches

    func title(_ voice: VoiceStyle) -> String {
        switch self {
        case .spots:
            voice.pick(luna: String(localized: "Luna's waiting at the shore", comment: "Empty spot list title in Luna's voice"), plain: String(localized: "No spots yet", comment: "Empty spot list title"))
        case .conditions:
            voice.pick(luna: String(localized: "Luna found nothing here", comment: "Empty conditions title in Luna's voice"), plain: String(localized: "No conditions to show", comment: "Empty conditions title"))
        case .trips:
            voice.pick(luna: String(localized: "No trips on the calendar", comment: "Empty trip list title in Luna's voice"), plain: String(localized: "No trips yet", comment: "Empty trip list title"))
        case .catches:
            voice.pick(luna: String(localized: "The log is empty, and so is the plate", comment: "Empty catch log title in Luna's voice"), plain: String(localized: "No catches logged yet", comment: "Empty catch log title"))
        }
    }

    func text(_ voice: VoiceStyle) -> String {
        switch self {
        case .spots:
            voice.pick(
                luna: String(localized: "Add the first place you fish and she will paddle out to watch it.", comment: "Empty spot list text in Luna's voice"),
                plain: String(localized: "Add the places you fish to see their conditions.", comment: "Empty spot list text"))
        case .conditions:
            voice.pick(
                luna: String(localized: "No station is close enough yet. Try another spot or loosen the distance limits.", comment: "Empty conditions text in Luna's voice"),
                plain: String(localized: "No station is close enough. Try another spot or loosen the distance limits.", comment: "Empty conditions text"))
        case .trips:
            voice.pick(
                luna: String(localized: "Plan a day on the water and she will start on the snack list.", comment: "Empty trip list text in Luna's voice"),
                plain: String(localized: "Plan a trip's days, with a checklist and maps for offline use.", comment: "Empty trip list text"))
        case .catches:
            voice.pick(
                luna: String(localized: "Keep what you catch, where and when. Sashimi is a fine reason.", comment: "Empty catch log text in Luna's voice"),
                plain: String(localized: "Keep your catches, where and when, and see what worked.", comment: "Empty catch log text"))
        }
    }
}

/// An empty screen: Luna paddling, a title and a line of help, or the plain words.
struct LunaEmptyState: View {
    let kind: EmptyKind
    @Environment(\.voiceStyle) private var voice

    var body: some View {
        VStack(spacing: 12) {
            if voice == .luna {
                LunaArt(picture: .paddling, maxWidth: 280)
            }
            Text(kind.title(voice))
                .font(.title3.bold())
                .multilineTextAlignment(.center)
            Text(kind.text(voice))
                .foregroundStyle(.readableSecondary)
                .multilineTextAlignment(.center)
        }
        .frame(maxWidth: 420)
        .padding()
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .accessibilityElement(children: .combine)
        // VoiceOver reads the plain meaning, not the playful title.
        .accessibilityLabel(Text(verbatim: "\(kind.title(.plain)). \(kind.text(.plain))"))
    }
}

/// A loading screen: Luna paddling in place (still with Reduce Motion), or a plain spinner.
struct LunaLoading: View {
    @Environment(\.voiceStyle) private var voice
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var bobbing = false

    var body: some View {
        VStack(spacing: 12) {
            if voice == .luna {
                LunaArt(picture: .paddling, maxWidth: 220)
                    .offset(y: bobbing ? 4 : -4)
                    .animation(
                        reduceMotion ? nil : .easeInOut(duration: 1.2).repeatForever(autoreverses: true),
                        value: bobbing)
                    .onAppear { bobbing = true }
            } else {
                ProgressView()
            }
            Text(voice.pick(luna: String(localized: "Luna's fetching the water…", comment: "Loading text in Luna's voice"), plain: String(localized: "Loading…", comment: "Loading text")))
                .foregroundStyle(.readableSecondary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(Text("Loading"))
    }
}
