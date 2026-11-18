import Foundation
import XCTest

@testable import UnderbiteKit

/// An offline store in a directory of its own, deleted with the test.
final class ScratchStore {
    let directory = FileManager.default.temporaryDirectory
        .appending(path: "underbite-map-offline-\(UUID().uuidString)", directoryHint: .isDirectory)
    lazy var store = OfflineStore(directory: directory)

    deinit {
        try? FileManager.default.removeItem(at: directory)
    }
}

/// A fetch host that answers from a list, records the URLs asked and keeps its store in a
/// dictionary; no network. An empty list answers with a transport failure.
final class OfflineHost: FetchHost, @unchecked Sendable {
    private let lock = NSLock()
    private var answers: [Result<HostResponse, HostError>]
    private var asked: [String] = []
    private var values: [String: String] = [:]

    init(_ answers: [Result<HostResponse, HostError>] = []) {
        self.answers = answers
    }

    var urls: [String] { lock.withLock { asked } }

    func answer(_ more: [Result<HostResponse, HostError>]) {
        lock.withLock { answers += more }
    }

    func value(_ namespace: String, _ key: String) -> String? {
        lock.withLock { values["\(namespace)/\(key)"] }
    }

    func send(request: HostRequest) async throws -> HostResponse {
        let answer: Result<HostResponse, HostError> = lock.withLock {
            asked.append(request.url)
            return answers.isEmpty
                ? .failure(.Failed(message: "The Internet connection appears to be offline."))
                : answers.removeFirst()
        }
        return try answer.get()
    }

    func load(namespace: String, key: String) -> String? { value(namespace, key) }

    func store(namespace: String, key: String, value: String) {
        lock.withLock { values["\(namespace)/\(key)"] = value.isEmpty ? nil : value }
    }

    func saveUsage(ledgerJson: String) throws {}
}

/// A switch the tests flip: whether the device has no route.
final class RouteSwitch: @unchecked Sendable {
    private let lock = NSLock()
    private var off = false
    init(offline: Bool = false) { off = offline }
    var isOffline: Bool { lock.withLock { off } }
    func set(offline: Bool) { lock.withLock { off = offline } }
}

/// A clock the tests move, in seconds.
final class MovingClock: @unchecked Sendable {
    private let lock = NSLock()
    private var now: Int64
    init(_ now: Int64) { self.now = now }
    var seconds: Int64 { lock.withLock { now } }
    var date: Date { Date(timeIntervalSince1970: TimeInterval(seconds)) }
    func advance(_ by: Int64) { lock.withLock { now += by } }
}

func hostResponse(
    _ status: UInt16, _ contentType: String?, _ body: Data, etag: String? = nil
) -> Result<HostResponse, HostError> {
    .success(
        HostResponse(
            status: status, body: body, contentType: contentType, date: nil, expires: nil,
            lastModified: nil, etag: etag, retryAfter: nil, rateLimitRemaining: nil))
}

/// A hand-made ERDDAP `.json` table of MUR SST: two by two cells from `south`, `west`.
func murTable(south: Double = 40, west: Double = -74, value: Double = 16) -> Data {
    var rows: [String] = []
    for i in 0..<2 {
        for j in 0..<2 {
            let (lat, lon) = (south + 0.5 * Double(i), west + 0.5 * Double(j))
            rows.append("[\"2026-11-07T09:00:00Z\", \(lat), \(lon), \(value), 1]")
        }
    }
    let table = """
        {"table": {"columnNames": ["time", "latitude", "longitude", "analysed_sst", "mask"],
        "columnTypes": ["String", "float", "float", "double", "byte"],
        "columnUnits": ["UTC", "degrees_north", "degrees_east", "degree_C", null],
        "rows": [\(rows.joined(separator: ", "))]}}
        """
    return Data(table.utf8)
}

/// Waits, a little at a time, until `condition` holds on the main actor (the badges are kept
/// there, reported from the loaders' threads); fails after two seconds.
@MainActor
func eventually(
    _ condition: @MainActor () -> Bool, file: StaticString = #filePath, line: UInt = #line
) async {
    for _ in 0..<200 {
        if condition() { return }
        try? await Task.sleep(for: .milliseconds(10))
    }
    XCTFail("the condition never held", file: file, line: line)
}
