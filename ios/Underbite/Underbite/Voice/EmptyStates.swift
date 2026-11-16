import SwiftUI

/// What a screen has nothing of yet.
enum EmptyKind {
    case spots, conditions, trips, catches

    func title(_ voice: VoiceStyle) -> String {
        switch self {
        case .spots:
            voice.pick(luna: "Luna's waiting at the shore", plain: "No spots yet")
        case .conditions:
            voice.pick(luna: "Luna found nothing here", plain: "No conditions to show")
        case .trips:
            voice.pick(luna: "No trips on the calendar", plain: "No trips yet")
        case .catches:
            voice.pick(luna: "The log is empty, and so is the plate", plain: "No catches logged yet")
        }
    }

    func text(_ voice: VoiceStyle) -> String {
        switch self {
        case .spots:
            voice.pick(
                luna: "Add the first place you fish and she will paddle out to watch it.",
                plain: "Add the places you fish to see their conditions.")
        case .conditions:
            voice.pick(
                luna: "No station is close enough yet. Try another spot or loosen the distance limits.",
                plain: "No station is close enough. Try another spot or loosen the distance limits.")
        case .trips:
            voice.pick(
                luna: "Plan a day on the water and she will start on the snack list.",
                plain: "Plan a trip's days, with a checklist and maps for offline use.")
        case .catches:
            voice.pick(
                luna: "Keep what you catch, where and when. Sashimi is a fine reason.",
                plain: "Keep your catches, where and when, and see what worked.")
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
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .frame(maxWidth: 420)
        .padding()
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .accessibilityElement(children: .combine)
        // VoiceOver reads the plain meaning, not the playful title.
        .accessibilityLabel(Text("\(kind.title(.plain)). \(kind.text(.plain))"))
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
            Text(voice.pick(luna: "Luna's fetching the water…", plain: "Loading…"))
                .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(Text("Loading"))
    }
}
