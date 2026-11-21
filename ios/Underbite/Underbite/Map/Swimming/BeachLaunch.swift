import Foundation
@preconcurrency import UnderbiteKit

/// The beach forecast's launch arguments, read in debug builds only (screenshots); a release
/// build reads none and nothing is saved. The layer itself is shown with `-mapLayers
/// beach-forecast`, or with `-mapPreset swimming`.
///
/// - `-mapFindBeaches` presses the refresh button once, when the map has settled with the layer on
///   (the layer looks itself up a second after the map settles, so shots need none).
/// - `-mapBeachOpen` then picks out the first zone as if tapped, once.
/// - `-mapBeachSample boston` answers the lookup with a hand-made forecast for the Boston area
///   (`beach-sample-boston.synthetic.json`: a low, a moderate and a high risk and a zone with no
///   forecast issued, the first with the values the office issued on 2026-11-20) at the time it
///   was made for, instead of asking the services.
enum BeachLaunch {
    #if DEBUG
        static let find = ProcessInfo.processInfo.arguments.contains("-mapFindBeaches")
        static let open = ProcessInfo.processInfo.arguments.contains("-mapBeachOpen")
        static var sample: Sample? {
            guard LaunchArguments.value(after: "-mapBeachSample") == "boston",
                let url = Bundle.main.url(
                    forResource: "beach-sample-boston.synthetic", withExtension: "json"),
                let zones = try? Data(contentsOf: url)
            else { return nil }
            return Sample(zones: zones)
        }
    #else
        static let find = false
        static let open = false
        static let sample: Sample? = nil
    #endif

    /// A forecast answered from a file.
    nonisolated struct Sample: Sendable {
        let zones: Data
        /// Two hours after the sample forecast was issued (2026-11-20 21:09 UTC).
        let now: Int64 = 1_795_208_940 + 2 * 3_600

        /// What the services would answer: the office of a point, the layer list or the zones.
        func answer(_ request: HostRequest) -> HostResponse {
            let body: Data
            if request.url.hasPrefix("https://api.weather.gov/points/") {
                body = Data(#"{"properties":{"cwa":"BOX"}}"#.utf8)
            } else if request.url.contains("/query?") {
                body = zones
            } else {
                body = Data(
                    #"{"layers":[{"id":1,"name":"BOX Day 1","type":"Feature Layer"},{"id":2,"name":"BOX Day 2","type":"Feature Layer"}]}"#
                        .utf8)
            }
            return HostResponse(
                status: 200, body: body, contentType: "application/json", date: nil,
                expires: nil, lastModified: nil, etag: nil, retryAfter: nil,
                rateLimitRemaining: nil)
        }
    }
}
