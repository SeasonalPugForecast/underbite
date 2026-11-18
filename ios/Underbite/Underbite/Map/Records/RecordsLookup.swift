import Foundation
@preconcurrency import UnderbiteKit

/// Runs the core's nearby lookups (`plan_records`, `plan_blooms`): sends each round of a plan's
/// requests through the app's fetch host, queue by queue with the waits the core asks for, hands
/// the answers back and repeats until the plan holds no more requests. Nothing is asked unless
/// the user pressed a lookup button; the core rounds the box before it leaves the phone.
nonisolated enum RecordsLookup {
    typealias Send = @Sendable (HostRequest) async throws -> HostResponse
    typealias Sleep = @Sendable (Duration) async -> Void

    /// The most rounds one lookup sends (pages and retries), whatever the plan asks.
    static let maxRounds = 12

    /// Waits `duration`, or until the task is cancelled.
    static let pause: Sleep = { duration in
        try? await Task.sleep(for: duration)
    }

    /// The core's view of a region on screen.
    static func area(_ region: MapRegion) -> NearbyArea {
        NearbyArea(west: region.west, south: region.south, east: region.east, north: region.north)
    }

    /// The two-letter US state of an ISO 3166-2 region (`US-NY` gives `NY`), which the USGS
    /// Nonindigenous Aquatic Species search needs; none for any other region.
    static func nasState(region: String) -> String? {
        let code = region.trimmingCharacters(in: .whitespacesAndNewlines).uppercased()
        let parts = code.split(separator: "-", omittingEmptySubsequences: false)
        guard parts.count == 2, parts[0] == "US", parts[1].count == 2,
            parts[1].allSatisfy({ $0.isASCII && $0.isLetter })
        else { return nil }
        return String(parts[1])
    }

    /// The records query for a species' scientific name, from every source; the USGS
    /// Nonindigenous Aquatic Species database is asked when `region` names a US state.
    static func query(scientificName: String, page: UInt32, region: String) -> RecordsQuery {
        RecordsQuery(
            scientificName: scientificName, page: page, sources: [],
            nasState: nasState(region: region), nasHuc8: nil)
    }

    /// Sends the plan's requests round after round, hands the answers to the plan and returns
    /// everything it found. A cancelled task stops sending; what was found so far is returned.
    /// It runs off the main actor: the core reads the answers here.
    @concurrent
    static func run(
        _ plan: NearbyPlan, send: @escaping Send, sleep: @escaping Sleep = RecordsLookup.pause,
        clock: @escaping @Sendable () -> Int64
    ) async -> NearbyResults {
        var results = plan.results()
        for _ in 0..<maxRounds {
            let requests = plan.requests()
            guard !requests.isEmpty, !Task.isCancelled else { break }
            let responses = await round(requests, send: send, sleep: sleep)
            results = plan.read(responses: responses, now: clock())
        }
        return results
    }

    /// Sends one round: the requests of a queue one after another in the order given, each
    /// after its delay, and the queues side by side. A request that fails has no answer, which
    /// the core counts as failed. The answers come back in the requests' order.
    static func round(
        _ requests: [NearbyRequest], send: @escaping Send, sleep: @escaping Sleep
    ) async -> [NearbyResponse] {
        var queues: [String: [(index: Int, request: NearbyRequest)]] = [:]
        var order: [String] = []
        for (index, request) in requests.enumerated() {
            if queues[request.queue] == nil { order.append(request.queue) }
            queues[request.queue, default: []].append((index, request))
        }
        var answers: [Int: NearbyResponse] = [:]
        await withTaskGroup(of: [(Int, NearbyResponse)].self) { group in
            for name in order {
                let queue = queues[name] ?? []
                group.addTask {
                    var out: [(Int, NearbyResponse)] = []
                    for (index, request) in queue {
                        if Task.isCancelled { break }
                        if request.delayMs > 0 {
                            await sleep(.milliseconds(Int(request.delayMs)))
                            if Task.isCancelled { break }
                        }
                        if let response = try? await send(request.request) {
                            out.append((index, NearbyResponse(id: request.id, response: response)))
                        }
                    }
                    return out
                }
            }
            for await done in group {
                for (index, response) in done { answers[index] = response }
            }
        }
        return answers.keys.sorted().compactMap { answers[$0] }
    }
}

/// The lookup state the core keeps between lookups (iNaturalist's pace and daily count, the
/// pauses GBIF and iNaturalist asked for), as the JSON the core gives, in one file on the
/// device. It holds no position and nothing about the user.
nonisolated struct RecordsStateFile: Sendable {
    let files: AppFiles
    static let name = "map/records-lookup-state.json"

    init(files: AppFiles = .standard) {
        self.files = files
    }

    /// The kept state; empty for none, as the core reads it.
    func read() -> String {
        files.text(Self.name) ?? ""
    }

    /// Keeps `json`; nothing when the plan had no state.
    func write(_ json: String?) {
        guard let json, !json.isEmpty else { return }
        files.write(Data(json.utf8), Self.name)
    }
}
