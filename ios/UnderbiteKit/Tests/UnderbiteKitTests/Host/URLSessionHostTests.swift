import Foundation
import XCTest

@testable import UnderbiteKit

/// The app's HTTP host against a stubbed `URLSession`: what a request sends and what comes
/// back, the body cap, errors, and the values a fetch stores. No network.
final class URLSessionHostTests: XCTestCase {
    private let tides =
        "https://api.tidesandcurrents.noaa.gov/api/prod/datagetter?station=8518750&product=predictions"
    private var directory: URL!

    override func setUp() {
        directory = FileManager.default.temporaryDirectory
            .appending(path: "underbite-host-\(UUID().uuidString)", directoryHint: .isDirectory)
        StubProtocol.answer(.failure(URLError(.notConnectedToInternet)))
    }

    override func tearDown() {
        try? FileManager.default.removeItem(at: directory)
        #if DEBUG
            URLSessionHost.simulateOffline = false
        #endif
    }

    private func host(bodyLimit: Int = URLSessionHost.maxBody) -> URLSessionHost {
        URLSessionHost(session: StubProtocol.session(), storeURL: directory, bodyLimit: bodyLimit)
    }

    private func get(_ url: String, headers: [Header] = [], maxAge: UInt32? = nil, noStore: Bool = false)
        -> HostRequest
    {
        HostRequest(
            url: url, headers: headers, method: .get, body: nil, fresh: false, noStore: noStore,
            maxAgeS: maxAge)
    }

    // MARK: - Responses

    func testAResponseBringsBackStatusBodyAndHeaders() async throws {
        let body = Data(#"{"predictions":[]}"#.utf8)
        StubProtocol.answer(
            .response(
                status: 200,
                headers: [
                    "Content-Type": "application/json; charset=utf-8",
                    "Date": "Sun, 08 Nov 2026 12:00:00 GMT",
                    "Expires": "Sun, 08 Nov 2026 13:00:00 GMT",
                    "Last-Modified": "Sat, 07 Nov 2026 09:30:00 GMT",
                    "ETag": #""abc123""#,
                    "Retry-After": "120",
                    "X-RateLimit-Remaining": "998",
                ], body: body))
        let response = try await host().send(request: get(tides))
        XCTAssertEqual(response.status, 200)
        XCTAssertEqual(response.body, body)
        XCTAssertEqual(response.contentType, "application/json; charset=utf-8")
        XCTAssertEqual(response.date, 1_794_139_200)
        XCTAssertEqual(response.expires, 1_794_142_800)
        XCTAssertEqual(response.lastModified, "Sat, 07 Nov 2026 09:30:00 GMT")
        XCTAssertEqual(response.etag, #""abc123""#)
        XCTAssertEqual(response.retryAfter, "120")
        XCTAssertEqual(response.rateLimitRemaining, "998")
    }

    func testAnErrorStatusIsAResponseWithItsBody() async throws {
        let body = Data("<error>not found</error>".utf8)
        StubProtocol.answer(.response(status: 404, headers: ["Content-Type": "text/xml"], body: body))
        let response = try await host().send(request: get(tides))
        XCTAssertEqual(response.status, 404)
        XCTAssertEqual(response.body, body)
        XCTAssertEqual(response.contentType, "text/xml")
        XCTAssertNil(response.date)
        XCTAssertNil(response.etag)
    }

    func testHTTPDatesAreUnixSecondsAndAnythingElseIsNone() {
        XCTAssertEqual(URLSessionHost.unixTime("Sun, 08 Nov 2026 12:00:00 GMT"), 1_794_139_200)
        XCTAssertNil(URLSessionHost.unixTime("0"))
        XCTAssertNil(URLSessionHost.unixTime("-1"))
        XCTAssertNil(URLSessionHost.unixTime(nil))
        XCTAssertEqual(URLSessionHost.httpDate(Date(timeIntervalSince1970: 1_794_139_200)), "Sun, 08 Nov 2026 12:00:00 GMT")
    }

    // MARK: - Requests

    func testAGetSendsItsHeadersAndTheCoresUserAgent() async throws {
        StubProtocol.answer(.response(status: 200, headers: [:], body: Data()))
        let key = Header(name: "X-Api-Key", value: "test-key")
        _ = try await host().send(request: get(tides, headers: [key]))
        let sent = try XCTUnwrap(StubProtocol.received.first)
        XCTAssertEqual(sent.httpMethod, "GET")
        XCTAssertEqual(sent.url?.absoluteString, tides)
        XCTAssertEqual(sent.value(forHTTPHeaderField: "X-Api-Key"), "test-key")
        let agent = try XCTUnwrap(sent.value(forHTTPHeaderField: "User-Agent"))
        XCTAssertTrue(agent.hasPrefix("underbite/"), agent)
    }

    func testARequestsOwnUserAgentIsKept() async throws {
        StubProtocol.answer(.response(status: 200, headers: [:], body: Data()))
        let agent = Header(name: "User-Agent", value: "underbite/test (+https://example.org)")
        _ = try await host().send(request: get(tides, headers: [agent]))
        XCTAssertEqual(
            StubProtocol.received.first?.value(forHTTPHeaderField: "User-Agent"), agent.value)
    }

    func testAPostSendsItsJSONBody() async throws {
        StubProtocol.answer(.response(status: 200, headers: [:], body: Data("{}".utf8)))
        let json = #"{"lat":40.7,"lon":-74.02}"#
        let request = HostRequest(
            url: "https://example.org/query", headers: [], method: .post, body: json, fresh: false,
            noStore: false, maxAgeS: nil)
        _ = try await host().send(request: request)
        let sent = try XCTUnwrap(StubProtocol.received.first)
        XCTAssertEqual(sent.httpMethod, "POST")
        XCTAssertEqual(sent.value(forHTTPHeaderField: "Content-Type"), "application/json")
        XCTAssertEqual(sent.httpBody.map { String(decoding: $0, as: UTF8.self) }, json)
    }

    func testAFreshCopyBypassesTheCache() {
        XCTAssertEqual(URLSessionHost.cachePolicy(fresh: true, headers: []), .reloadIgnoringLocalCacheData)
        XCTAssertEqual(
            URLSessionHost.cachePolicy(fresh: false, headers: [Header(name: "cache-control", value: "No-Cache")]),
            .reloadIgnoringLocalCacheData)
        XCTAssertEqual(URLSessionHost.cachePolicy(fresh: false, headers: []), .useProtocolCachePolicy)
        let request = HostRequest(
            url: tides, headers: [], method: .get, body: nil, fresh: true, noStore: false, maxAgeS: nil)
        let url = URL(string: tides)!
        XCTAssertEqual(URLSessionHost.urlRequest(request, url: url).cachePolicy, .reloadIgnoringLocalCacheData)
    }

    // MARK: - Cache lifetime

    func testAnUncachedAnswerIsKeptForTheRequestsMaxAge() async throws {
        StubProtocol.answer(.response(status: 200, headers: ["Content-Type": "application/json"], body: Data("[]".utf8)))
        let host = host()
        let request = get(tides, maxAge: 600)
        _ = try await host.send(request: request)
        let urlRequest = URLSessionHost.urlRequest(request, url: URL(string: tides)!)
        let kept = try XCTUnwrap(host.session.configuration.urlCache?.cachedResponse(for: urlRequest))
        let response = try XCTUnwrap(kept.response as? HTTPURLResponse)
        XCTAssertEqual(response.value(forHTTPHeaderField: "Cache-Control"), "max-age=600")
        XCTAssertNotNil(response.value(forHTTPHeaderField: "Date"))
        XCTAssertEqual(kept.data, Data("[]".utf8))
        XCTAssertEqual(kept.storagePolicy, .allowedInMemoryOnly)
    }

    // MARK: - Cache-served responses

    func testOnlyAResponseEndingInTheLocalCacheIsFromCache() {
        typealias Fetch = URLSessionTaskMetrics.ResourceFetchType
        XCTAssertTrue(URLSessionHost.servedFromCache([.localCache]))
        XCTAssertFalse(URLSessionHost.servedFromCache([.networkLoad]))
        XCTAssertFalse(URLSessionHost.servedFromCache([.serverPush, .unknown]))
        // A redirect whose last hop went to the network is an answer.
        XCTAssertFalse(URLSessionHost.servedFromCache([Fetch.localCache, .networkLoad]))
        XCTAssertFalse(URLSessionHost.servedFromCache([]))
    }

    func testTheCacheFlagPassesThroughToTheCore() throws {
        let http = try XCTUnwrap(
            HTTPURLResponse(
                url: URL(string: tides)!, statusCode: 200, httpVersion: "HTTP/1.1", headerFields: [:]))
        XCTAssertFalse(URLSessionHost.response(Data(), http).fromCache)
        XCTAssertTrue(URLSessionHost.response(Data(), http, fromCache: true).fromCache)
    }

    func testAnAnswerFromTheNetworkIsNotFlagged() async throws {
        StubProtocol.answer(.response(status: 200, headers: ["Content-Type": "application/json"], body: Data("[]".utf8)))
        let response = try await host().send(request: get(tides, maxAge: 600))
        XCTAssertFalse(response.fromCache)
    }

    func testTheServersCacheHeadersAndNoStoreRequestsAreLeftAlone() async throws {
        StubProtocol.answer(.response(status: 200, headers: ["Cache-Control": "max-age=60"], body: Data("[]".utf8)))
        let host = host()
        let told = get(tides, maxAge: 600)
        _ = try await host.send(request: told)
        let cache = try XCTUnwrap(host.session.configuration.urlCache)
        XCTAssertNil(cache.cachedResponse(for: URLSessionHost.urlRequest(told, url: URL(string: tides)!)))

        StubProtocol.answer(.response(status: 200, headers: [:], body: Data("[]".utf8)))
        let keyed = "https://example.org/keyed"
        let secret = get(keyed, maxAge: 600, noStore: true)
        _ = try await host.send(request: secret)
        XCTAssertNil(cache.cachedResponse(for: URLSessionHost.urlRequest(secret, url: URL(string: keyed)!)))
    }

    // MARK: - Limits and errors

    func testTheCapIsTheCoresEightMegabytes() {
        XCTAssertEqual(URLSessionHost.maxBody, 8 * 1024 * 1024)
    }

    func testABodyOverTheCapIsRefused() async throws {
        StubProtocol.answer(.response(status: 200, headers: [:], body: Data(count: 17)))
        do {
            _ = try await host(bodyLimit: 16).send(request: get(tides))
            XCTFail("a 17-byte body passed a 16-byte cap")
        } catch HostError.TooLarge {
        } catch {
            XCTFail("unexpected \(error)")
        }
        StubProtocol.answer(.response(status: 200, headers: [:], body: Data(count: 16)))
        let response = try await host(bodyLimit: 16).send(request: get(tides))
        XCTAssertEqual(response.body.count, 16)
    }

    func testADeclaredLengthOverTheCapIsRefusedAtOnce() async throws {
        StubProtocol.answer(.response(status: 200, headers: ["Content-Length": "4096"], body: Data(count: 4096)))
        do {
            _ = try await host(bodyLimit: 1024).send(request: get(tides))
            XCTFail("a 4 KB body passed a 1 KB cap")
        } catch HostError.TooLarge {
        } catch {
            XCTFail("unexpected \(error)")
        }
    }

    func testATimeoutIsTimedOutAndNoAnswerIsAFailure() async {
        StubProtocol.answer(.failure(URLError(.timedOut)))
        do {
            _ = try await host().send(request: get(tides))
            XCTFail("a timeout passed")
        } catch HostError.TimedOut {
        } catch {
            XCTFail("unexpected \(error)")
        }
        StubProtocol.answer(.failure(URLError(.notConnectedToInternet)))
        do {
            _ = try await host().send(request: get(tides))
            XCTFail("no connection passed")
        } catch HostError.Failed {
        } catch {
            XCTFail("unexpected \(error)")
        }
    }

    func testAnInvalidURLFails() async {
        do {
            _ = try await host().send(request: get(""))
            XCTFail("an empty URL passed")
        } catch HostError.Failed(let message) {
            XCTAssertEqual(message, "invalid URL")
        } catch {
            XCTFail("unexpected \(error)")
        }
    }

    #if DEBUG
        func testSimulatedOfflineFailsWithoutAsking() async {
            StubProtocol.answer(.response(status: 200, headers: [:], body: Data()))
            URLSessionHost.simulateOffline = true
            do {
                _ = try await host().send(request: get(tides))
                XCTFail("an offline request passed")
            } catch HostError.Failed {
            } catch {
                XCTFail("unexpected \(error)")
            }
            XCTAssertTrue(StubProtocol.received.isEmpty)
        }
    #endif

    // MARK: - Stored values and the ledger

    func testStoredValuesComeBackAndMissingOnesAreNone() {
        let host = host()
        XCTAssertNil(host.load(namespace: "stations", key: "coops-water-levels"))
        host.store(namespace: "stations", key: "coops-water-levels", value: #"{"stations":[]}"#)
        XCTAssertEqual(host.load(namespace: "stations", key: "coops-water-levels"), #"{"stations":[]}"#)
        host.store(namespace: "stations", key: "coops-water-levels", value: "[]")
        XCTAssertEqual(host.load(namespace: "stations", key: "coops-water-levels"), "[]")
        XCTAssertNil(host.load(namespace: "rankings", key: "coops-water-levels"))
    }

    func testAnEmptyValueDeletesItsFile() throws {
        let host = host()
        host.store(namespace: "snapshots", key: "spot-1", value: #"{"lat":40.7033}"#)
        let file = try XCTUnwrap(host.file("snapshots", "spot-1"))
        XCTAssertTrue(FileManager.default.fileExists(atPath: file.path))
        // How the core forgets a deleted spot's values.
        host.store(namespace: "snapshots", key: "spot-1", value: "")
        XCTAssertFalse(FileManager.default.fileExists(atPath: file.path))
        XCTAssertNil(host.load(namespace: "snapshots", key: "spot-1"))
        // Forgetting what was never stored leaves no file either.
        host.store(namespace: "snapshots", key: "spot-2", value: "")
        XCTAssertFalse(FileManager.default.fileExists(atPath: try XCTUnwrap(host.file("snapshots", "spot-2")).path))
    }

    func testTheLedgerIsReadBackAsUsage() throws {
        let host = host()
        try host.saveUsage(ledgerJson: #"{"open-meteo":{"day":3}}"#)
        XCTAssertEqual(host.load(namespace: "usage", key: "ledger"), #"{"open-meteo":{"day":3}}"#)
        let again = URLSessionHost(session: StubProtocol.session(), storeURL: directory)
        XCTAssertEqual(again.load(namespace: "usage", key: "ledger"), #"{"open-meteo":{"day":3}}"#)
    }

    func testNamesThatCouldLeaveTheDirectoryAreRefused() {
        let host = host()
        for bad in ["", ".", "..", "../x", "a/b", ".hidden", "a b", "40.70,-74.02"] {
            host.store(namespace: bad, key: "k", value: "v")
            host.store(namespace: "n", key: bad, value: "v")
            XCTAssertNil(host.load(namespace: bad, key: "k"), bad)
            XCTAssertNil(host.file(bad, "k"), bad)
            XCTAssertNil(host.file("n", bad), bad)
        }
        XCTAssertNotNil(host.file("stations.v2", "usgs_sites-1"))
        let written = (try? FileManager.default.subpathsOfDirectory(atPath: directory.path())) ?? []
        XCTAssertTrue(written.isEmpty, "\(written)")
    }

    func testALedgerThatCannotBeWrittenFails() throws {
        // A file where the store's directory should be.
        try FileManager.default.createDirectory(
            at: directory.deletingLastPathComponent(), withIntermediateDirectories: true)
        try Data().write(to: directory)
        XCTAssertThrowsError(try host().saveUsage(ledgerJson: "{}")) { error in
            guard case HostError.Failed = error else { return XCTFail("unexpected \(error)") }
        }
    }
}
