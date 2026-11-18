import SwiftUI
import UnderbiteKit

/// Shares a trip as a `.underbite` file, from the trip's toolbar: first how much of each place's
/// location the file holds (about 1 km unless changed), then the system's share sheet. A
/// popover anchored to the button on a regular width, a sheet on a compact one. The file is
/// made by the core; it never holds the catch log or photos.
struct TripShareButton: View {
    @Environment(SpotStore.self) private var spots
    /// The trip's JSON, as the trip store keeps it.
    let tripJSON: String

    @State private var choosing = false

    var body: some View {
        Button("Share", systemImage: "square.and.arrow.up") { choosing = true }
            .popover(isPresented: $choosing) {
                ShareFuzzChoice(tripJSON: tripJSON, settingsJSON: spots.settingsJSON)
                    .frame(idealWidth: 380)
                    .presentationDetents([.medium, .large])
                    .presentationDragIndicator(.visible)
                    .presentationCompactAdaptation(.sheet)
            }
            #if DEBUG
                .task {
                    // Once the trip's page has settled: a popover asked for while the split
                    // view is still pushing it is lost.
                    guard TripShareLaunch.opens() else { return }
                    try? await Task.sleep(for: .seconds(3))
                    choosing = true
                }
            #endif
    }
}

/// The location choice and the share button for the file it makes. Every word is the core's.
private struct ShareFuzzChoice: View {
    let tripJSON: String
    let settingsJSON: String

    @State private var fuzz: ShareFuzz?

    private var words: ShareWords? { ShareText.words(settingsJSON: settingsJSON) }

    var body: some View {
        let words = words
        let chosen = fuzz ?? words?.defaultFuzz ?? .rounded1Km
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                if let words {
                    Text(words.fuzzTitle)
                        .font(.headline)
                        .accessibilityAddTraits(.isHeader)
                    VStack(spacing: 0) {
                        ForEach(Array(words.fuzz.enumerated()), id: \.offset) { index, choice in
                            if index > 0 { Divider().padding(.leading, 44) }
                            FuzzRow(choice: choice, selected: choice.fuzz == chosen) {
                                fuzz = choice.fuzz
                            }
                        }
                    }
                    .background(
                        Color(.secondarySystemGroupedBackground), in: .rect(cornerRadius: 12))
                }
                ShareFileLink(tripJSON: tripJSON, fuzz: chosen)
            }
            .padding(20)
        }
        .background(Color(.systemGroupedBackground))
        .readableText()
    }
}

/// One level of location: its name and what it reveals, a check on the chosen one.
private struct FuzzRow: View {
    let choice: FuzzChoice
    let selected: Bool
    let choose: () -> Void

    var body: some View {
        Button(action: choose) {
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                Image(systemName: selected ? "checkmark.circle.fill" : "circle")
                    .foregroundStyle(selected ? AnyShapeStyle(Palette.accent) : AnyShapeStyle(.readableSecondary))
                    .frame(width: 20)
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 2) {
                    Text(choice.label).foregroundStyle(.primary)
                    Text(choice.detail)
                        .font(.footnote)
                        .foregroundStyle(.readableSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}

/// The system's share button for the trip's file at `fuzz`, or the core's text when the trip
/// cannot be written as a file.
private struct ShareFileLink: View {
    let tripJSON: String
    let fuzz: ShareFuzz

    var body: some View {
        switch Result(catching: { try UnderbiteFile.trip(tripJSON, fuzz: fuzz) }) {
        case .success(let file):
            let name = UnderbiteFile.name(of: tripJSON)
            ShareLink(item: file, preview: SharePreview(name.isEmpty ? file.fileName : name))
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
                .frame(maxWidth: .infinity)
        case .failure(let error):
            Label(ShareText.message(of: error), systemImage: "exclamationmark.triangle")
                .font(.callout)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

#if DEBUG
    /// Launch argument of the screenshot runs: `-shareTrip` opens the location choice of the
    /// trip that is open (`-openTrip <id>`).
    enum TripShareLaunch {
        private static var applied = false

        static func opens() -> Bool {
            guard !applied, ProcessInfo.processInfo.arguments.contains("-shareTrip") else {
                return false
            }
            applied = true
            return true
        }
    }
#endif
