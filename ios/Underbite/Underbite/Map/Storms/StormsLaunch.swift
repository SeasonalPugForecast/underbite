import Foundation

/// The storms module's launch arguments, read in debug builds only (screenshots). The layer itself
/// is shown with the map's `-mapLayers storms`.
///
/// - `-mapStormsSample noreaster|josephine` draws a built-in outlook instead of fetching one: the
///   nor'easter of the storm samples, or Josephine as NHC's recorded files give her, each at the
///   time it was made for. Nothing is fetched or saved.
/// - `-mapStormsStep <n>` marks the lows at that time step (0 is now).
/// - `-mapStormsFocus <NHC id>` frames that hurricane alone the first time the layer draws it.
enum StormsLaunch {
    /// A built-in outlook and the time and zone it is drawn at.
    struct Sample {
        let outlookJSON: String
        let now: Int64
        let timeZone: String
    }

    static var sample: Sample? {
        #if DEBUG
            guard let name = LaunchArguments.value(after: "-mapStormsSample"),
                ["noreaster", "josephine"].contains(name),
                let url = Bundle.main.url(
                    forResource: "storms-sample-\(name)", withExtension: "json"),
                let json = try? String(contentsOf: url, encoding: .utf8),
                let head = try? JSONDecoder().decode(SampleHead.self, from: Data(json.utf8))
            else { return nil }
            return Sample(
                outlookJSON: json, now: head.fetchedAt,
                timeZone: head.place.tzName ?? "America/New_York")
        #else
            return nil
        #endif
    }

    static var step: Int? {
        LaunchArguments.value(after: "-mapStormsStep").flatMap { Int($0) }
    }

    static var focus: String? { LaunchArguments.value(after: "-mapStormsFocus") }
}

/// What a sample outlook says of when and where it is.
private struct SampleHead: Decodable {
    struct Place: Decodable {
        let tzName: String?

        enum CodingKeys: String, CodingKey { case tzName = "tz_name" }
    }

    let place: Place
    let fetchedAt: Int64

    enum CodingKeys: String, CodingKey {
        case place
        case fetchedAt = "fetched_at"
    }
}
