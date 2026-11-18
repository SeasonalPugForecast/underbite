import Foundation
import XCTest

@testable import UnderbiteKit

/// The one nearby state the access and records lookups share: neither overwrites the other's
/// part, and a state the core cannot read is set aside.
@MainActor
final class NearbyStateStoreTests: XCTestCase {
    /// 2026-11-17 15:00 UTC.
    private let now: Int64 = 1_794_927_600
    /// The Battery and the lower Hudson's mouth.
    private let battery = NearbyArea(west: -74.05, south: 40.68, east: -73.99, north: 40.74)

    private let directory = FileManager.default.temporaryDirectory
        .appending(path: "underbite-nearby-state-\(UUID().uuidString)", directoryHint: .isDirectory)

    override func tearDown() {
        try? FileManager.default.removeItem(at: directory)
    }

    private var files: AppFiles { AppFiles(directory: directory) }
    private func store() -> NearbyStateStore { NearbyStateStore(files: files) }
    private func settingsJSON() throws -> String { try Settings.defaults().json() }

    private func object(_ json: String) throws -> [String: Any] {
        try XCTUnwrap(
            try JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
    }

    /// An access lookup through `store` that found one ramp: its base and its final state.
    private func access(_ store: NearbyStateStore) throws -> (base: String, state: String) {
        let json = try settingsJSON()
        let (plan, base) = try store.plan { kept in
            try planAccess(
                area: battery, includeParking: false, stateJson: kept, settingsJson: json,
                now: now)
        }
        let ramp = #"""
            {"version":0.6,"elements":[{"type":"node","id":1,"lat":40.7006,"lon":-73.9969,"tags":{"leisure":"slipway"}}]}
            """#
        let responses = plan.requests().map { r in
            NearbyResponse(
                id: r.id,
                response: HostResponse(
                    status: 200,
                    body: Data(
                        (r.source == "padus"
                            ? #"{"type":"FeatureCollection","features":[]}"# : ramp).utf8),
                    contentType: "application/json", date: nil, expires: nil, lastModified: nil,
                    etag: nil, retryAfter: nil, rateLimitRemaining: nil))
        }
        return (base, try XCTUnwrap(plan.read(responses: responses, now: now).stateJson))
    }

    /// A records lookup through `store`, which counts one iNaturalist request and gets no
    /// answer: its base and its final state.
    private func records(_ store: NearbyStateStore) throws -> (base: String, state: String) {
        let json = try settingsJSON()
        let (plan, base) = try store.plan { kept in
            try planRecords(
                area: battery,
                query: RecordsQuery(
                    scientificName: "Morone saxatilis", page: 1, sources: ["inaturalist"],
                    nasState: nil, nasHuc8: nil),
                stateJson: kept, settingsJson: json, now: now)
        }
        XCTAssertEqual(plan.requests().map(\.source), ["inaturalist"])
        return (base, try XCTUnwrap(plan.read(responses: [], now: now).stateJson))
    }

    /// Whether `state` holds Overpass cells (an access lookup's part) and counts an iNaturalist
    /// request (a records lookup's part).
    private func parts(_ state: String) throws -> (cells: Bool, paced: Bool) {
        let o = try object(state)
        let cells = (o["overpass_cache"] as? [String: Any]).map { !$0.isEmpty } ?? false
        let used = (o["inaturalist"] as? [String: Any])?["used"] as? Int ?? 0
        return (cells, used > 0)
    }

    func testAnAccessSaveKeepsTheRecordsPartAndTheOtherWayRound() throws {
        // Both lookups plan before either ends, and end in either order.
        for accessFirst in [true, false] {
            try? FileManager.default.removeItem(at: directory)
            let store = store()
            let a = try access(store)
            let r = try records(store)
            XCTAssertTrue(try parts(a.state).cells)
            XCTAssertTrue(try parts(r.state).paced)
            if accessFirst {
                store.save(a.state, base: a.base)
                store.save(r.state, base: r.base)
            } else {
                store.save(r.state, base: r.base)
                store.save(a.state, base: a.base)
            }
            let (cells, paced) = try parts(store.load())
            XCTAssertTrue(cells, "the Overpass cells stay (access first: \(accessFirst))")
            XCTAssertTrue(paced, "the iNaturalist count stays (access first: \(accessFirst))")

            // The core reads the merged state: the cells answer the next access lookup.
            let json = try settingsJSON()
            let (next, _) = try store.plan { kept in
                try planAccess(
                    area: battery, includeParking: false, stateJson: kept, settingsJson: json,
                    now: now + 60)
            }
            XCTAssertEqual(next.requests().map(\.source), ["padus"])
        }
    }

    func testWhatPlanningCountsIsKeptAtOnce() throws {
        let store = store()
        let json = try settingsJSON()
        let (_, base) = try store.plan { kept in
            try planAccess(
                area: battery, includeParking: false, stateJson: kept, settingsJson: json,
                now: now)
        }
        XCTAssertFalse(base.isEmpty)
        XCTAssertEqual(store.load(), base, "the Overpass requests are counted before they go")
    }

    func testAMergeTakesOnlyWhatALookupChanged() throws {
        let base = #"{"a":1,"b":{"x":1},"gone":true,"version":1}"#
        let mine = #"{"a":2,"b":{"x":1},"version":1}"#
        let current = #"{"a":1,"b":{"x":5},"c":3,"gone":true,"version":1}"#
        let merged = try object(
            NearbyStateStore.merge(base: base, mine: mine, current: current))
        XCTAssertEqual(merged["a"] as? Int, 2, "changed by this lookup")
        XCTAssertEqual((merged["b"] as? [String: Any])?["x"] as? Int, 5, "changed by the other")
        XCTAssertEqual(merged["c"] as? Int, 3)
        XCTAssertNil(merged["gone"], "dropped by this lookup")
        // Nothing kept, or text that is not an object: the lookup's state as it is.
        XCTAssertEqual(NearbyStateStore.merge(base: "", mine: mine, current: ""), mine)
        XCTAssertEqual(NearbyStateStore.merge(base: "", mine: mine, current: "[1]"), mine)
        XCTAssertEqual(NearbyStateStore.merge(base: "", mine: "oops", current: current), "oops")
    }

    func testAStateTheCoreCannotReadIsSetAsideAndTheLookupStartsAfresh() throws {
        let json = try settingsJSON()
        XCTAssertTrue(files.write(Data("{ not a state".utf8), NearbyStateFile.name))
        let (plan, base) = try store().plan { kept in
            try planAccess(
                area: battery, includeParking: false, stateJson: kept, settingsJson: json, now: now)
        }
        XCTAssertFalse(plan.requests().isEmpty)
        XCTAssertEqual(try files.utf8(NearbyStateFile.name + ".bad"), "{ not a state")
        XCTAssertEqual(store().load(), base, "a fresh state, kept")
        XCTAssertNotEqual(base, "{ not a state")
        files.remove(NearbyStateFile.name)

        // A file that is not text is set aside too, never written over.
        XCTAssertTrue(files.write(Data([0xFF, 0xFE, 0x00, 0xC3]), NearbyStateFile.name))
        XCTAssertEqual(store().load(), "")
        XCTAssertTrue(files.exists(NearbyStateFile.name + ".2.bad"))
    }

    func testOtherPlanningErrorsAreNotBlamedOnTheState() throws {
        let store = store()
        XCTAssertTrue(store.save("{}", base: ""))
        XCTAssertThrowsError(
            try store.plan { kept in
                try planAccess(
                    area: NearbyArea(west: 0, south: 10, east: 1, north: 5),
                    includeParking: false, stateJson: kept, settingsJson: settingsJSON(), now: now)
            })
        XCTAssertEqual(store.load(), "{}", "kept")
    }
}
