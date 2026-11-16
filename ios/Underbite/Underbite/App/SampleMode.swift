#if DEBUG
    import Foundation
    import UnderbiteKit

    /// `-sample` (debug builds, UI tests): the app shows the sample page of The Battery bundled
    /// with it, from a store of its own in a temporary directory, and never reaches the network.
    /// The page never changes, so the accessibility audits see the same screens on every run.
    nonisolated enum SampleMode {
        static var isOn: Bool { ProcessInfo.processInfo.arguments.contains("-sample") }

        /// The spot of the sample page (its id is the page's).
        static let spot = Spot(
            id: "battery", name: "The Battery", latitude: 40.7033, longitude: -74.0170,
            waterType: "tidal_river_estuary", timeZone: "America/New_York")

        /// The sample page as `view_model` wrote it; empty when the bundle lacks it.
        static let page: String = {
            guard let url = Bundle.main.url(forResource: "SamplePage", withExtension: "json"),
                let text = try? String(contentsOf: url, encoding: .utf8)
            else { return "" }
            return text
        }()
    }

    extension SpotStore {
        /// A store holding the sample spot: every fetch is an empty success, every station
        /// lookup finds nothing, and every page build is the sample page.
        static func sample() -> SpotStore {
            let directory = FileManager.default.temporaryDirectory.appending(
                path: "underbite-sample-\(UUID().uuidString)", directoryHint: .isDirectory)
            let page = SampleMode.page
            let store = SpotStore(
                files: AppFiles(directory: directory),
                keys: { Keys(usgs: nil, copernicusUser: nil, copernicusPassword: nil) },
                fetcher: { _, _, _, _, _, _ in
                    SpotFetchOutcome(snapshotJson: "{}", errors: [], suggestion: nil)
                },
                discoverer: { _, _, _, _, _ in
                    DiscoveryOutcome(stationsJson: "{}", errors: [])
                },
                pageBuilder: { _, _, _, _ in page })
            store.add(SampleMode.spot)
            return store
        }
    }
#endif
