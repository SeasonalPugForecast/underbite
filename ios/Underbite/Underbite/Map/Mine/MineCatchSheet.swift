import MapKit
import SwiftUI
import UIKit
@preconcurrency import UnderbiteKit

/// What a tapped catch mark opens: the catch in full (the log's own detail screen), or for a
/// group of catches their rows, each opening its detail. Every word is the core's, from the log's
/// list and detail views.
struct MineCatchSheet: View {
    @Environment(SpotStore.self) private var spots
    let catches: CatchStore
    /// The catches of the mark, newest first.
    let ids: [String]
    /// The layer's title, the list's title.
    let title: String
    /// Closes the sheet.
    let close: () -> Void
    @State private var rows: [CatchRow] = []

    var body: some View {
        NavigationStack {
            Group {
                if ids.count == 1, let id = ids.first {
                    CatchDetailScreen(catches: catches, id: id)
                } else {
                    List(rows) { row in
                        NavigationLink(value: row.id) {
                            MineCatchRow(row: row)
                        }
                    }
                    .navigationTitle(title)
                    .navigationBarTitleDisplayMode(.inline)
                    .navigationDestination(for: String.self) { id in
                        CatchDetailScreen(catches: catches, id: id)
                    }
                }
            }
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done", action: close)
                }
            }
        }
        // The rows follow the log, the settings (units, voice, clock) and the spots' names.
        .task(id: "\(catches.revision) \(spots.settingsJSON.hashValue)") {
            guard ids.count > 1 else { return }
            let places = spots.spots.map { PlaceName(spotId: $0.id, name: $0.name) }
            let list = CatchList.build(
                book: catches.book, settingsJSON: spots.settingsJSON, places: places)
            let byID = Dictionary(
                (list?.rows ?? []).map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
            rows = ids.compactMap { byID[$0] }
        }
    }
}

/// A catch of a group: species, size, time and fate, as the log's list words them.
private struct MineCatchRow: View {
    let row: CatchRow

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(alignment: .firstTextBaseline) {
                Text(row.species).font(.headline)
                Spacer(minLength: 8)
                CatchFateTag(row: row)
            }
            if let size = row.size {
                Text(size).font(.subheadline)
            }
            Text(row.when).font(.subheadline).foregroundStyle(.readableSecondary)
        }
        .padding(.vertical, 4)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(row.summary))
    }
}

/// Presents a ``MineCatchSheet`` from the map: a popover pointing at the mark where the width is
/// regular (the iPad), a sheet at medium height otherwise. The map's views are UIKit, so it is
/// presented by UIKit from the controller on top of the map's window; the app's store, voice and
/// sun mode are handed to it as the app's root view hands them to its screens.
@MainActor
enum MineCatchPresenter {
    /// The size the popover asks for, in points.
    static let popoverSize = CGSize(width: 380, height: 480)

    /// Presents the sheet for `ids` pointing at `coordinate` on `map`; returns what it presented.
    @discardableResult
    static func present(
        ids: [String], title: String, at coordinate: CLLocationCoordinate2D, on map: MKMapView,
        catches: CatchStore, store: SpotStore = .shared, closed: @escaping () -> Void
    ) -> UIViewController? {
        guard var top = map.window?.rootViewController else { return nil }
        while let next = top.presentedViewController, !next.isBeingDismissed { top = next }
        weak var presented: UIViewController?
        let sheet = MineCatchSheet(
            catches: catches, ids: ids, title: title,
            close: { presented?.dismiss(animated: true) }
        )
        .environment(store)
        .environment(\.voiceStyle, voice(store.settingsJSON))
        .sunMode(store.sunMode || LaunchArguments.sunMode)
        // Gone by a swipe, a tap outside the popover or "Done": the mark is let go.
        .onDisappear(perform: closed)
        let host = UIHostingController(rootView: sheet)
        presented = host
        host.modalPresentationStyle = .popover
        host.preferredContentSize = popoverSize
        if let popover = host.popoverPresentationController {
            popover.sourceView = map
            let at = map.convert(coordinate, toPointTo: map)
            let reach = CatchMarks.reach
            popover.sourceRect = CGRect(
                x: at.x - reach, y: at.y - reach, width: 2 * reach, height: 2 * reach)
            let adapted = popover.adaptiveSheetPresentationController
            adapted.detents = [.medium(), .large()]
            adapted.prefersGrabberVisible = true
        }
        top.present(host, animated: true)
        return host
    }

    /// The voice of the settings (`presentation.voice`); Luna's when they cannot be read.
    private static func voice(_ settingsJSON: String) -> VoiceStyle {
        VoiceStyle(setting: (try? Settings(json: settingsJSON).presentation.voice) ?? "luna")
    }
}
