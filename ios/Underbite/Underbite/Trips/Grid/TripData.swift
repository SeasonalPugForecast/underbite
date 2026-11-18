import Foundation
@preconcurrency import UnderbiteKit

/// What a trip's page keeps between launches besides the spots' snapshots: each place's history
/// (past years' values for the trip's dates) and the tide stations' metadata (harmonic
/// constants, datums, offsets), both as the core gave them. One file per trip,
/// `trip-data/<id>.json` in the app's directory.
nonisolated struct TripData: Codable, Equatable, Sendable {
    /// Per place key, the core's `TripHistory` JSON.
    var histories: [String: String] = [:]
    /// The station metadata to pass back to the core, by cache key.
    var metadata: [Metadata] = []

    /// One kept response.
    struct Metadata: Codable, Equatable, Sendable {
        var key: String
        var body: Data
        var fetchedAt: Int64
    }

    /// The metadata as the core takes it.
    var cachedMetadata: [CachedResponse] {
        metadata.map { CachedResponse(key: $0.key, body: $0.body, fetchedAt: $0.fetchedAt) }
    }

    /// What the core needs of each place of the trip, keyed as its columns are: the place's
    /// snapshot (`snapshot`, by key) and its kept history. `fresh` names the places whose
    /// history is left out, so the core plans it again whole.
    func places(
        _ keys: [String], snapshot: (String) -> String?, fresh: Set<String> = []
    ) -> [TripPlaceData] {
        keys.map { key in
            TripPlaceData(
                key: key, snapshotJson: snapshot(key),
                historyJson: fresh.contains(key) ? nil : histories[key])
        }
    }

    /// Takes in what a plan's responses gave: a place's new history replaces its old one (the
    /// plan asked for all of it), and new station metadata replaces the copy of the same key.
    mutating func merge(_ results: TripResults) {
        for history in results.histories {
            histories[history.key] = history.historyJson
        }
        for entry in results.cacheable {
            let kept = Metadata(key: entry.key, body: entry.body, fetchedAt: entry.fetchedAt)
            if let index = metadata.firstIndex(where: { $0.key == entry.key }) {
                metadata[index] = kept
            } else {
                metadata.append(kept)
            }
        }
    }

    /// The places a plan asks history for: those whose kept history lacks something. The page
    /// plans them again without it, so the history that comes back is whole.
    static func historyPlaces(_ requests: [TripRequest]) -> Set<String> {
        Set(requests.filter { $0.source != "coops" }.map(\.place))
    }

    // MARK: - The file

    /// The file of trip `id`; the id is percent-encoded, so it is one file name.
    static func fileName(_ id: String) -> String {
        let safe = id.addingPercentEncoding(withAllowedCharacters: .alphanumerics) ?? "trip"
        return "trip-data/" + safe + ".json"
    }

    /// What is kept for trip `id`; nothing when there is no file or it cannot be read (the
    /// next refresh fetches it again).
    static func read(_ id: String, files: AppFiles) -> TripData {
        files.read(TripData.self, fileName(id)) ?? TripData()
    }

    func write(_ id: String, files: AppFiles) {
        _ = files.write(self, Self.fileName(id))
    }
}
