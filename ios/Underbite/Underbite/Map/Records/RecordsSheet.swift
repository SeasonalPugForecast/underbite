import MapKit
import SwiftUI
import UIKit
@preconcurrency import UnderbiteKit

/// What a tapped mark or area shows.
enum RecordsSheetContent {
    /// Species records, with the standing notes of their lookup (what records are, the USGS
    /// disclaimer).
    case records([RecordPin], notes: [String])
    /// Forecast beaches, with the forecast's credit and its safety notes.
    case beaches([BloomPin], credit: NearbyCredit?, notes: [String])
}

/// What a tapped mark or area opens: one record or beach in full, or for a group their rows,
/// each opening its own. Every word is the core's: a record shows its licence, its page and its
/// platform's credit, a photo only when the core gives one (openly licensed), and no observer;
/// a beach shows its level, the forecast's credit and the plain safety note.
struct RecordsSheet: View {
    let content: RecordsSheetContent
    /// The layer's title, the list's title.
    let title: String
    /// Loads the photos: the app's only network client.
    let host: URLSessionHost
    let close: () -> Void

    var body: some View {
        NavigationStack {
            Group {
                switch content {
                case .records(let pins, let notes):
                    if pins.count == 1, let pin = pins.first {
                        RecordDetail(pin: pin, notes: notes, host: host)
                    } else {
                        List(pins, id: \.id) { pin in
                            NavigationLink(value: pin.id) { RecordRow(pin: pin) }
                        }
                        .navigationTitle(title)
                        .navigationBarTitleDisplayMode(.inline)
                        .navigationDestination(for: String.self) { id in
                            if let pin = pins.first(where: { $0.id == id }) {
                                RecordDetail(pin: pin, notes: notes, host: host)
                            }
                        }
                    }
                case .beaches(let pins, let credit, let notes):
                    if pins.count == 1, let pin = pins.first {
                        BeachDetail(pin: pin, credit: credit, notes: notes)
                    } else {
                        List(pins, id: \.id) { pin in
                            NavigationLink(value: pin.id) { BeachRow(pin: pin) }
                        }
                        .navigationTitle(title)
                        .navigationBarTitleDisplayMode(.inline)
                        .navigationDestination(for: String.self) { id in
                            if let pin = pins.first(where: { $0.id == id }) {
                                BeachDetail(pin: pin, credit: credit, notes: notes)
                            }
                        }
                    }
                }
            }
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done", action: close)
                }
            }
        }
    }
}

// MARK: - Records

/// A record of a group: what was seen, when, from which source, under which licence.
private struct RecordRow: View {
    let pin: RecordPin

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            RecordSymbol(kind: pin.source == RecordMarks.invasiveSource ? .invasive : .record)
            VStack(alignment: .leading, spacing: 2) {
                Text(pin.title).font(.headline)
                Text(pin.seen).font(.subheadline)
                Text(pin.sourceName).font(.subheadline).foregroundStyle(.readableSecondary)
                Text(pin.licence).font(.caption).foregroundStyle(.readableSecondary)
            }
        }
        .padding(.vertical, 4)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(pin.accessibility))
        .accessibilityValue(Text(pin.licence))
    }
}

/// One record in full.
private struct RecordDetail: View {
    let pin: RecordPin
    let notes: [String]
    let host: URLSessionHost

    var body: some View {
        List {
            Section {
                HStack(alignment: .top, spacing: 10) {
                    RecordSymbol(
                        kind: pin.source == RecordMarks.invasiveSource ? .invasive : .record)
                    VStack(alignment: .leading, spacing: 3) {
                        Text(pin.title).font(.headline)
                        if pin.commonName != nil {
                            Text(pin.taxon).font(.subheadline).italic()
                        }
                        Text(pin.seen).font(.subheadline)
                        if let position = pin.position {
                            Text(position).font(.subheadline)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        Text(pin.sourceName).font(.subheadline)
                            .foregroundStyle(.readableSecondary)
                    }
                }
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("record-card")
            }
            // The licence, the record's page and the credit before the photo: the attribution
            // the licence asks for is never below the fold.
            Section {
                LinkOrText(text: pin.licence, url: pin.licenceUrl, symbol: "c.circle")
                    .accessibilityIdentifier("record-licence")
                if let link = pin.link {
                    LinkOrText(text: pin.sourceName, url: link, symbol: "arrow.up.right.square")
                }
                LinkOrText(text: pin.credit, url: pin.creditUrl, symbol: nil)
                    .font(.footnote)
            } footer: {
                if pin.photo == nil { NotesFooter(notes: notes) }
            }
            if let photo = pin.photo {
                Section {
                    RecordPhotoView(photo: photo, host: host)
                } footer: {
                    NotesFooter(notes: notes)
                }
            }
        }
        .navigationTitle(pin.title)
        .navigationBarTitleDisplayMode(.inline)
    }
}

/// A record's photo, loaded through the app's fetch host, with its own licence beneath.
private struct RecordPhotoView: View {
    let photo: RecordPhoto
    let host: URLSessionHost
    @State private var image: UIImage?
    @State private var failed = false

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Group {
                if let image {
                    Image(uiImage: image)
                        .resizable()
                        .scaledToFit()
                        .clipShape(RoundedRectangle(cornerRadius: 8))
                } else if !failed {
                    ProgressView()
                        .frame(maxWidth: .infinity, minHeight: 120)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: 260)
            .accessibilityHidden(true)
            Text(photo.caption).font(.caption).foregroundStyle(.readableSecondary)
        }
        .task(id: photo.url) {
            image = await Self.load(photo.url, host: host)
            failed = image == nil
        }
    }

    /// The picture at `url`, through `host`; none when it cannot be fetched or read.
    @concurrent
    nonisolated static func load(_ url: String, host: URLSessionHost) async -> UIImage? {
        guard let address = URL(string: url), address.scheme == "https" else { return nil }
        guard let reply = try? await host.send(URLRequest(url: address, timeoutInterval: 30)),
            reply.1.statusCode == 200
        else { return nil }
        return UIImage(data: reply.0)
    }
}

// MARK: - Beaches

/// A beach of a group: its name and level.
private struct BeachRow: View {
    let pin: BloomPin

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            RecordSymbol(kind: .beach, level: pin.level)
            VStack(alignment: .leading, spacing: 2) {
                Text(pin.name).font(.headline)
                Text(pin.detail).font(.subheadline)
                Text(pin.forecastFor).font(.subheadline).foregroundStyle(.readableSecondary)
            }
        }
        .padding(.vertical, 4)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(pin.accessibility))
    }
}

/// One beach in full, with the forecast's credit and the safety note in full.
private struct BeachDetail: View {
    let pin: BloomPin
    let credit: NearbyCredit?
    let notes: [String]

    var body: some View {
        List {
            Section {
                HStack(alignment: .top, spacing: 10) {
                    RecordSymbol(kind: .beach, level: pin.level)
                    VStack(alignment: .leading, spacing: 3) {
                        Text(pin.name).font(.headline)
                        Text(pin.detail).font(.subheadline)
                            .fixedSize(horizontal: false, vertical: true)
                        Text(pin.forecastFor).font(.subheadline)
                            .foregroundStyle(.readableSecondary)
                    }
                }
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(Text(pin.accessibility))
                .accessibilityIdentifier("beach-card")
            }
            if !notes.isEmpty {
                Section {
                    ForEach(notes, id: \.self) { note in
                        DrawerNotice(text: note)
                    }
                }
            }
            Section {
                if let link = pin.link {
                    LinkOrText(
                        text: credit?.name ?? link, url: link, symbol: "arrow.up.right.square")
                }
                if let credit {
                    LinkOrText(text: credit.text, url: credit.url, symbol: nil)
                        .font(.footnote)
                    if let licence = credit.licence {
                        Text(licence).font(.footnote).foregroundStyle(.readableSecondary)
                    }
                }
            }
        }
        .navigationTitle(pin.name)
        .navigationBarTitleDisplayMode(.inline)
    }
}

// MARK: - Parts

/// A mark as the map draws it, beside a row.
struct RecordSymbol: View {
    let kind: RecordItem.Kind
    var level: String?

    var body: some View {
        Image(uiImage: RecordDrawing.image(kind, level: level))
            .accessibilityHidden(true)
    }
}

/// `text` as a link to `url` when there is a usable one, else as text.
private struct LinkOrText: View {
    let text: String
    let url: String?
    let symbol: String?

    var body: some View {
        if let url, let address = URL(string: url), address.scheme?.hasPrefix("http") == true {
            Link(destination: address) {
                HStack(alignment: .firstTextBaseline) {
                    Text(text).fixedSize(horizontal: false, vertical: true)
                    Spacer(minLength: 8)
                    if let symbol {
                        Image(systemName: symbol).accessibilityHidden(true)
                    }
                }
            }
            .accessibilityAddTraits(.isLink)
        } else {
            Text(text).fixedSize(horizontal: false, vertical: true)
        }
    }
}

/// The standing notes under a record, in full.
private struct NotesFooter: View {
    let notes: [String]

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            ForEach(notes, id: \.self) { note in
                Text(note).fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

// MARK: - Presenting

/// Presents a ``RecordsSheet`` from the map: a popover pointing at the mark or the tap where the
/// width is regular (the iPad), a sheet at medium height otherwise. The map's views are UIKit,
/// so it is presented by UIKit from the controller on top of the map's window.
@MainActor
enum RecordsPresenter {
    /// The size the popover asks for, in points.
    static let popoverSize = CGSize(width: 380, height: 520)

    /// Presents `content` pointing at `coordinate` on `map`; returns what it presented.
    @discardableResult
    static func present(
        _ content: RecordsSheetContent, title: String, at coordinate: CLLocationCoordinate2D,
        on map: MKMapView, host: URLSessionHost, store: SpotStore = .shared,
        closed: @escaping () -> Void
    ) -> UIViewController? {
        guard var top = map.window?.rootViewController else { return nil }
        while let next = top.presentedViewController, !next.isBeingDismissed { top = next }
        weak var presented: UIViewController?
        let sheet = RecordsSheet(
            content: content, title: title, host: host,
            close: { presented?.dismiss(animated: true) }
        )
        .environment(store)
        .sunMode(store.sunMode || LaunchArguments.sunMode)
        // Gone by a swipe, a tap outside the popover or "Done": the mark is let go.
        .onDisappear(perform: closed)
        let hosting = UIHostingController(rootView: sheet)
        presented = hosting
        hosting.modalPresentationStyle = .popover
        hosting.preferredContentSize = popoverSize
        if let popover = hosting.popoverPresentationController {
            popover.sourceView = map
            let at = map.convert(coordinate, toPointTo: map)
            let reach = RecordMarks.reach
            popover.sourceRect = CGRect(
                x: at.x - reach, y: at.y - reach, width: 2 * reach, height: 2 * reach)
            let adapted = popover.adaptiveSheetPresentationController
            adapted.detents = [.medium(), .large()]
            adapted.prefersGrabberVisible = true
        }
        top.present(hosting, animated: true)
        return hosting
    }
}
