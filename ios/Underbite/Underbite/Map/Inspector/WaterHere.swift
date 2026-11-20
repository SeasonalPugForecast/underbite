import CoreLocation
import Foundation
@preconcurrency import UnderbiteKit

/// Where the "water here" lookup of the open point stands. Every word in it is the core's
/// (``WaterHereCard``, ``WaterHereTexts``); nothing here formats.
nonisolated enum WaterHereState: Equatable {
    /// Nothing is shown: no lookup yet, or no water within reach of the point.
    case hidden
    /// The lookup is running: the section shows the core's loading words.
    case looking
    /// The water at the point.
    case found(WaterHereCard)
    /// The lookup failed: the section shows the core's failure words.
    case failed(String)

    /// The state the core's result stands for. No water within reach leaves no section.
    init(_ result: WaterHereResult) {
        switch result.status {
        case .looking: self = .looking
        case .found:
            if let card = result.card {
                self = .found(card)
            } else {
                self = .hidden
            }
        case .nothing: self = .hidden
        case .failed: self = .failed(result.message ?? "")
        }
    }

    /// Whether the point card has a "water here" section.
    var showsSection: Bool { self != .hidden }

    /// The water, once found.
    var card: WaterHereCard? {
        if case .found(let card) = self { return card }
        return nil
    }
}

/// What "Add as spot" starts the new-spot flow with: the place and the water type the core
/// inferred, and the water's own name when the source gives one.
nonisolated struct SpotPrefill: Identifiable, Equatable, Sendable {
    let id = UUID()
    let latitude: Double
    let longitude: Double
    /// The water's name; empty for unnamed water (never made up).
    let name: String
    /// The water type id (`river_stream`); none when the place's water is not known, and the
    /// core suggests one after the first fetch.
    let waterType: String?

    init(latitude: Double, longitude: Double, name: String, waterType: String?) {
        self.latitude = latitude
        self.longitude = longitude
        self.name = name
        self.waterType = waterType
    }

    /// The prefill for the water the card names at the point, at its exact place.
    init(card: WaterHereCard, at coordinate: CLLocationCoordinate2D) {
        self.init(
            latitude: coordinate.latitude, longitude: coordinate.longitude,
            name: card.spotName ?? "", waterType: card.waterType)
    }

    var coordinate: CLLocationCoordinate2D {
        CLLocationCoordinate2D(latitude: latitude, longitude: longitude)
    }

    static func == (a: SpotPrefill, b: SpotPrefill) -> Bool {
        a.latitude == b.latitude && a.longitude == b.longitude && a.name == b.name
            && a.waterType == b.waterType
    }
}

/// The heights the point card's sheet rests at on the iPhone: a first glance, a middle height
/// that shows the "water here" section with its buttons and credits, and full height. The card
/// rises to the middle height by itself when the section appears, unless the person has already
/// moved the sheet.
nonisolated struct PointSheetStops: Equatable {
    enum Stop: Equatable {
        case glance, water, full
    }

    private(set) var stop: Stop = .glance
    private(set) var moved = false

    /// The "water here" section shows or goes: the sheet rises when it appears and the person
    /// has not touched the sheet; it never moves on its own otherwise.
    mutating func water(shown: Bool) {
        guard shown, !moved, stop == .glance else { return }
        stop = .water
    }

    /// The sheet came to rest at `new`: if that is not where this logic put it, the person
    /// moved it.
    mutating func rested(at new: Stop) {
        guard new != stop else { return }
        stop = new
        moved = true
    }

    /// A different point is opened, or the card is closed: back to the first glance.
    mutating func reset() {
        self = PointSheetStops()
    }
}

/// Runs the core's water-here plan (`plan_water_here`): sends each round of the plan's requests
/// through the app's fetch host, hands the answers back and repeats until the plan holds no more
/// requests. The core rounds the point before it leaves the phone and places the water here, on
/// the phone.
nonisolated enum WaterHereLookup {
    /// Sends the plan's requests round after round and returns what the plan found; `progress`
    /// hears each result as it comes (the water first, then its stocking lines). A cancelled
    /// task stops sending. It runs off the main actor: the core reads the answers here.
    @concurrent
    static func run(
        _ plan: WaterHerePlan, send: @escaping RecordsLookup.Send,
        sleep: @escaping RecordsLookup.Sleep = RecordsLookup.pause,
        clock: @escaping @Sendable () -> Int64,
        progress: @escaping @Sendable (WaterHereResult) -> Void = { _ in }
    ) async -> WaterHereResult {
        var result = plan.result()
        for _ in 0..<RecordsLookup.maxRounds {
            let requests = plan.requests()
            guard !requests.isEmpty, !Task.isCancelled else { break }
            let responses = await RecordsLookup.round(requests, send: send, sleep: sleep)
            guard !Task.isCancelled else { break }
            result = plan.read(responses: responses, now: clock())
            progress(result)
        }
        return result
    }

    /// The places of the tide stations among the pins, for the core to decide whether a river
    /// here has tide in it.
    static func tideStations(_ pins: [MapPin]) -> [MapPoint] {
        pins.filter { $0.kind == .tide }.map { MapPoint(lat: $0.latitude, lon: $0.longitude) }
    }

    /// The region the app knows the user fishes (ISO 3166-2, `US-NY`), for the regulation links
    /// and the stocking lists; none when it is not set.
    static func region(_ stored: String?) -> String? {
        let text = stored?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        return text.isEmpty ? nil : text
    }
}
