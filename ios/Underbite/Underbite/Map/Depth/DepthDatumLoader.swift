import Foundation
@preconcurrency import UnderbiteKit

/// What the depths in the view are measured from, for the depth legend: the core's
/// `depth_datums` for the view, from the elevation models the map's `GridStore` has been told
/// of. Where it knows of none in the view, the models at the view's centre are asked for
/// (`dem_sources_request`, the point rounded by the core to the settings' precision) and the
/// line is worked out again once they are in. Each request is sent once while it succeeds; a
/// failed one is sent again by a later view.
@MainActor
final class DepthDatumLoader {
    let store: GridStore
    private let host: FetchHost
    /// The datums of the latest view; none before the first, or when the core refused the view.
    private(set) var datums: DepthDatums? {
        didSet { if datums != oldValue { onChange(datums) } }
    }
    /// The datums changed.
    var onChange: (DepthDatums?) -> Void = { _ in }
    /// The requests sent or being sent, by URL.
    private var asked: Set<String> = []
    private var latestRegion: MapRegion?
    private var latestJSON = ""

    init(host: FetchHost, store: GridStore) {
        self.host = host
        self.store = store
    }

    /// The datums of `region` with the words of `json`; asks for the models at its centre when
    /// none in it are known. The task ends when that fetch does (tests wait for it); none when
    /// nothing is fetched.
    @discardableResult
    func load(_ region: MapRegion, settingsJSON json: String) -> Task<Void, Never>? {
        latestRegion = region
        latestJSON = json
        let found = read(region, json)
        datums = found
        guard let found, found.datums.isEmpty else { return nil }
        let (lat, lon) = (region.centerLatitude, region.centerLongitude)
        guard let request = try? store.demSourcesRequest(lat: lat, lon: lon, settingsJson: json),
            !asked.contains(request.url)
        else { return nil }
        asked.insert(request.url)
        let (host, store) = (host, store)
        return Task {
            let added = await Self.fetch(request, lat: lat, lon: lon, host: host, store: store)
            guard added else {
                self.asked.remove(request.url)
                return
            }
            self.refresh()
        }
    }

    /// The datums of the latest view again, with the words of `json` (the settings changed).
    func reword(settingsJSON json: String) {
        latestJSON = json
        refresh()
    }

    private func refresh() {
        guard let region = latestRegion else { return }
        datums = read(region, latestJSON)
    }

    private func read(_ region: MapRegion, _ json: String) -> DepthDatums? {
        try? store.depthDatums(
            west: region.west, south: region.south, east: region.east, north: region.north,
            settingsJson: json)
    }

    private nonisolated static func fetch(
        _ request: HostRequest, lat: Double, lon: Double, host: FetchHost, store: GridStore
    ) async -> Bool {
        do {
            let response = try await host.send(request: request)
            try store.addDemSources(lat: lat, lon: lon, response: response)
            return true
        } catch {
            return false
        }
    }
}
