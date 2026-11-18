import SwiftUI
import UnderbiteKit

extension View {
    /// Opens the `.underbite` files the app is handed (from Files, Mail, Messages or AirDrop):
    /// the core decodes each and a sheet previews it, to add the trip or spot or leave it.
    func receivesSharedFiles() -> some View {
        modifier(ReceivesSharedFiles())
    }
}

/// The `onOpenURL` of `.underbite` files and the sheet that previews one.
private struct ReceivesSharedFiles: ViewModifier {
    @Environment(SpotStore.self) private var spots
    @State private var received: ReceivedFile?
    @State private var importer = ImportInbox()

    func body(content: Content) -> some View {
        content
            .receivesImports(importer)
            .onOpenURL { url in
                // Waypoints and tracks (GPX, KML, KMZ, CSV) go to the import preview.
                if ImportInbox.accepts(url) { return importer.open(url, store: spots) }
                guard url.isFileURL,
                    url.pathExtension.lowercased() == UnderbiteFile.pathExtension
                else { return }
                received = ReceivedFile.open(
                    url, settingsJSON: spots.settingsJSON, now: Self.now)
            }
            .sheet(item: $received) { file in
                ReceivedFileSheet(file: file, trips: .shared)
                    .environment(spots)
            }
            #if DEBUG
                .task {
                    guard ReceiveFileLaunch.sample() else { return }
                    // Once the tabs have settled: a sheet asked for while the first screen is
                    // still appearing is lost.
                    try? await Task.sleep(for: .seconds(2))
                    received = ReceivedFile.sample(
                        today: TripDates.day(of: Date()), settingsJSON: spots.settingsJSON,
                        now: Self.now)
                }
            #endif
    }

    private static var now: Int64 { Int64(Date().timeIntervalSince1970) }
}

/// A file someone sent: what it holds, every word the core's, and what can be done with it
/// (add, add as a copy, replace this device's trip); a place shared rounded or by area only
/// says its location is approximate. A file this build cannot use shows the core's text.
struct ReceivedFileSheet: View {
    @Environment(SpotStore.self) private var spots
    @Environment(\.dismiss) private var dismiss
    let file: ReceivedFile
    let trips: TripStore

    /// Whether the trip or spot has been added.
    @State private var added = false
    @State private var problem: String?

    private var words: ShareWords? { ShareText.words(settingsJSON: spots.settingsJSON) }

    var body: some View {
        let words = words
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    switch file.content {
                    case .trip(_, let listing):
                        TripPreview(listing: listing, approximate: file.approximate)
                    case .spot(let place):
                        SpotPreview(place: place, approximate: file.approximate)
                    case .problem(let message):
                        Label(message, systemImage: "exclamationmark.triangle")
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    if added, let words {
                        Label(words.added, systemImage: "checkmark.circle.fill")
                            .font(.headline)
                            .foregroundStyle(Palette.accent)
                    } else {
                        actions(words)
                    }
                    if let problem {
                        Label(problem, systemImage: "exclamationmark.triangle")
                            .font(.callout)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                .frame(maxWidth: 560, alignment: .leading)
                .frame(maxWidth: .infinity)
                .padding(20)
            }
            .background(Color(.systemGroupedBackground))
            .readableText()
            .navigationTitle(file.heading ?? "")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: added ? .confirmationAction : .cancellationAction) {
                    if added {
                        Button("Done") { dismiss() }
                    } else {
                        Button("Cancel", role: .cancel) { dismiss() }
                    }
                }
            }
        }
        .presentationDetents([.medium, .large])
    }

    @ViewBuilder
    private func actions(_ words: ShareWords?) -> some View {
        let choices = file.choices(trips: trips.trips, spots: spots.spots)
        if let words, !choices.isEmpty {
            VStack(alignment: .leading, spacing: 10) {
                if file.exists(trips: trips.trips, spots: spots.spots) {
                    Text(isTrip ? words.tripExists : words.spotExists)
                        .font(.callout)
                        .foregroundStyle(.readableSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                ForEach(Array(choices.enumerated()), id: \.offset) { index, choice in
                    let title = Self.title(of: choice, words: words)
                    if index == 0 {
                        Button { accept(choice) } label: { Text(title).frame(maxWidth: .infinity) }
                            .buttonStyle(.borderedProminent)
                            .controlSize(.large)
                    } else {
                        Button { accept(choice) } label: { Text(title).frame(maxWidth: .infinity) }
                            .buttonStyle(.bordered)
                            .controlSize(.large)
                    }
                }
                // A trip is not added while the trips cannot be saved.
                .disabled(isTrip && !trips.canSave)
            }
        }
    }

    private var isTrip: Bool {
        if case .trip = file.content { return true }
        return false
    }

    private static func title(of choice: ReceiveChoice, words: ShareWords) -> String {
        switch choice {
        case .add: words.add
        case .addCopy: words.addCopy
        case .replace: words.replace
        }
    }

    private func accept(_ choice: ReceiveChoice) {
        do {
            try file.accept(choice, trips: trips, spots: spots)
            problem = nil
            added = true
        } catch {
            problem = ShareText.message(of: error)
        }
    }
}

/// A received trip: its name, dates and places as the core's trip view words them.
private struct TripPreview: View {
    let listing: TripListing
    let approximate: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            VStack(alignment: .leading, spacing: 2) {
                Text(listing.title).font(.title3.bold())
                if !listing.dates.isEmpty { Text(listing.dates).font(.subheadline) }
                if !listing.status.isEmpty {
                    Text(listing.status)
                        .font(.subheadline)
                        .foregroundStyle(.readableSecondary)
                }
            }
            .accessibilityElement(children: .combine)
            ApproximateNote(text: approximate)
            ForEach(Array(listing.places.enumerated()), id: \.offset) { _, place in
                PlaceLine(name: place.name, area: place.area, waterType: place.waterTypeLabel)
            }
            if !listing.notes.isEmpty {
                Text(listing.notes)
                    .font(.subheadline)
                    .foregroundStyle(.readableSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let problem = listing.problem {
                Label(problem, systemImage: "exclamationmark.triangle")
                    .font(.callout)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(.secondarySystemGroupedBackground), in: .rect(cornerRadius: 14))
    }
}

/// A received spot: its name, area and kind of water, and what the sender wrote.
private struct SpotPreview: View {
    let place: ReceivedPlace
    let approximate: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            PlaceLine(name: place.name, area: place.area, waterType: place.waterTypeLabel)
            ApproximateNote(text: approximate)
            if !place.notes.isEmpty {
                Text(place.notes)
                    .font(.subheadline)
                    .foregroundStyle(.readableSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(.secondarySystemGroupedBackground), in: .rect(cornerRadius: 14))
    }
}

/// "Approximate location", when the sender rounded the places or shared only their area.
private struct ApproximateNote: View {
    let text: String?

    var body: some View {
        if let text {
            Label(text, systemImage: "location.slash")
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(Palette.accent)
        }
    }
}

/// A place: its name, then its area and kind of water.
private struct PlaceLine: View {
    let name: String
    let area: String?
    let waterType: String

    var body: some View {
        Label {
            VStack(alignment: .leading, spacing: 2) {
                Text(name).font(.headline)
                let detail = [area ?? "", waterType].filter { !$0.isEmpty }
                if !detail.isEmpty {
                    Text(detail.joined(separator: " · "))
                        .font(.subheadline)
                        .foregroundStyle(.readableSecondary)
                }
            }
        } icon: {
            Image(systemName: "mappin.and.ellipse").foregroundStyle(Palette.accent)
        }
        .accessibilityElement(children: .combine)
    }
}

#if DEBUG
    /// Launch argument of the screenshot runs: `-receiveFile sample` previews a sample trip file
    /// shared at about 1 km, as if it had been opened from Files.
    enum ReceiveFileLaunch {
        private static var applied = false

        static func sample() -> Bool {
            guard !applied, LaunchArguments.value(after: "-receiveFile") == "sample" else {
                return false
            }
            applied = true
            return true
        }
    }
#endif
