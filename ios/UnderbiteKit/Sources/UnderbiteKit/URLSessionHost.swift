import Foundation

/// The app's only network client: performs the core's requests with `URLSession`, and keeps
/// what a fetch stores between fetches (station lists, rankings, the call ledger) in files.
/// The map and the offline packs send their own requests through `send(_:)`.
///
/// A URL or a header may carry a key (USGS, Copernicus): never log either. Responses are kept
/// in memory only (`URLSession.noStore`), so nothing a request returns reaches the disk unless
/// the fetch stores it itself.
public final class URLSessionHost: FetchHost, @unchecked Sendable {
    let session: URLSession
    /// The directory of the stored values: one file per namespace and key.
    public let storeURL: URL
    /// The largest body accepted, in bytes.
    let bodyLimit: Int
    /// One file operation at a time: the spots of a refresh fetch side by side and share the
    /// directory.
    private let lock = NSLock()

    /// The core's cap on a response body (8 MB, decompressed), as the command line's.
    public static let maxBody = Int(clamping: maxBodyBytes())

    /// `session` must not cache on disk (requests that carry a key ask for `no_store`);
    /// `storeURL` is a directory for the values a fetch keeps (created when first written),
    /// which must last: it holds the call ledger.
    public convenience init(session: URLSession = .noStore, storeURL: URL) {
        self.init(session: session, storeURL: storeURL, bodyLimit: Self.maxBody)
    }

    init(session: URLSession, storeURL: URL, bodyLimit: Int) {
        self.session = session
        self.storeURL = storeURL
        self.bodyLimit = bodyLimit
    }

    #if DEBUG
        /// Screenshots and tests: every request fails as when the phone is offline.
        nonisolated(unsafe) public static var simulateOffline = false
        /// Screenshots: each request waits this long first (a slow network).
        nonisolated(unsafe) public static var simulatedDelay: Duration = .zero
    #endif

    // MARK: - HTTP

    /// Sends `request` and returns any status with its body, read up to the body limit. Sets
    /// the core's User-Agent unless the request has one. Throws `HostError` only: `TooLarge`
    /// past the limit, `TimedOut`, or `Failed` when no response came.
    public func send(_ request: URLRequest) async throws -> (Data, HTTPURLResponse) {
        #if DEBUG
            if Self.simulateOffline {
                throw HostError.Failed(message: URLError(.notConnectedToInternet).localizedDescription)
            }
            if Self.simulatedDelay > .zero { try? await Task.sleep(for: Self.simulatedDelay) }
        #endif
        var request = request
        if request.value(forHTTPHeaderField: "User-Agent") == nil {
            request.setValue(Self.userAgent, forHTTPHeaderField: "User-Agent")
        }
        do {
            let (bytes, response) = try await session.bytes(for: request)
            guard let http = response as? HTTPURLResponse else {
                bytes.task.cancel()
                throw HostError.Failed(message: "not an HTTP response")
            }
            // A compressed length over the limit is surely over it once decompressed.
            if http.expectedContentLength > Int64(bodyLimit) {
                bytes.task.cancel()
                throw HostError.TooLarge
            }
            var body: [UInt8] = []
            if http.expectedContentLength > 0 {
                body.reserveCapacity(Int(http.expectedContentLength))
            }
            for try await byte in bytes {
                guard body.count < bodyLimit else {
                    bytes.task.cancel()
                    throw HostError.TooLarge
                }
                body.append(byte)
            }
            return (Data(body), http)
        } catch let error as HostError {
            throw error
        } catch let error as URLError where error.code == .timedOut {
            throw HostError.TimedOut(message: error.localizedDescription)
        } catch {
            throw HostError.Failed(message: error.localizedDescription)
        }
    }

    public func send(request: HostRequest) async throws -> HostResponse {
        guard let url = URL(string: request.url) else {
            throw HostError.Failed(message: "invalid URL")
        }
        let urlRequest = Self.urlRequest(request, url: url)
        let (data, http) = try await send(urlRequest)
        if let maxAge = request.maxAgeS, !request.noStore {
            keep(data, http, for: urlRequest, maxAge: maxAge)
        }
        return Self.response(data, http)
    }

    /// The `URLRequest` for the core's request.
    static func urlRequest(_ request: HostRequest, url: URL) -> URLRequest {
        var urlRequest = URLRequest(url: url, timeoutInterval: 30)
        switch request.method {
        case .get:
            urlRequest.httpMethod = "GET"
        case .post:
            urlRequest.httpMethod = "POST"
            urlRequest.setValue("application/json", forHTTPHeaderField: "Content-Type")
            urlRequest.httpBody = request.body.map { Data($0.utf8) }
        }
        for header in request.headers {
            urlRequest.setValue(header.value, forHTTPHeaderField: header.name)
        }
        urlRequest.cachePolicy = cachePolicy(fresh: request.fresh, headers: request.headers)
        return urlRequest
    }

    /// The session's cache answers as HTTP allows, except for a request that asks for a fresh
    /// copy (the flag, or `Cache-Control: no-cache` among its headers), which always goes to
    /// the server.
    static func cachePolicy(fresh: Bool, headers: [Header]) -> URLRequest.CachePolicy {
        let noCache = headers.contains {
            $0.name.caseInsensitiveCompare("Cache-Control") == .orderedSame
                && $0.value.lowercased().contains("no-cache")
        }
        return fresh || noCache ? .reloadIgnoringLocalCacheData : .useProtocolCachePolicy
    }

    /// A successful GET whose server said nothing about caching is kept in the session's
    /// (memory) cache for the request's own `maxAge`, as if the server had sent
    /// `Cache-Control: max-age`.
    private func keep(_ data: Data, _ http: HTTPURLResponse, for request: URLRequest, maxAge: UInt32) {
        guard request.httpMethod == "GET", (200..<300).contains(http.statusCode),
            http.value(forHTTPHeaderField: "Cache-Control") == nil,
            http.value(forHTTPHeaderField: "Expires") == nil,
            let cache = session.configuration.urlCache, let url = http.url
        else { return }
        var headers: [String: String] = [:]
        for case (let name as String, let value as String) in http.allHeaderFields {
            headers[name] = value
        }
        headers["Cache-Control"] = "max-age=\(maxAge)"
        if headers["Date"] == nil {
            headers["Date"] = Self.httpDate(Date())
        }
        guard
            let kept = HTTPURLResponse(
                url: url, statusCode: http.statusCode, httpVersion: "HTTP/1.1", headerFields: headers)
        else { return }
        cache.storeCachedResponse(
            CachedURLResponse(response: kept, data: data, storagePolicy: .allowedInMemoryOnly), for: request)
    }

    /// The response as the core reads it.
    static func response(_ data: Data, _ http: HTTPURLResponse) -> HostResponse {
        func header(_ name: String) -> String? { http.value(forHTTPHeaderField: name) }
        return HostResponse(
            status: UInt16(clamping: http.statusCode),
            body: data,
            contentType: header("Content-Type"),
            date: unixTime(header("Date")),
            expires: unixTime(header("Expires")),
            lastModified: header("Last-Modified"),
            etag: header("ETag"),
            retryAfter: header("Retry-After"),
            rateLimitRemaining: header("X-RateLimit-Remaining")
        )
    }

    /// The User-Agent the core names (`underbite/<version> (iOS; +<contact>)`).
    static let userAgent = UnderbiteKit.userAgent()

    /// An HTTP date (`Sun, 08 Nov 2026 12:00:00 GMT`) as Unix seconds; `nil` for anything
    /// else (`Expires: 0`).
    static func unixTime(_ header: String?) -> Int64? {
        guard let header else { return nil }
        return httpDateFormatter().date(from: header).map { Int64($0.timeIntervalSince1970) }
    }

    /// `date` as an HTTP date.
    static func httpDate(_ date: Date) -> String {
        httpDateFormatter().string(from: date)
    }

    private static func httpDateFormatter() -> DateFormatter {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = TimeZone(identifier: "GMT")
        formatter.dateFormat = "EEE, dd MMM yyyy HH:mm:ss zzz"
        return formatter
    }

    // MARK: - Stored values and the call ledger

    public func load(namespace: String, key: String) -> String? {
        guard let file = file(namespace, key) else { return nil }
        return lock.withLock { try? String(contentsOf: file, encoding: .utf8) }
    }

    public func store(namespace: String, key: String, value: String) {
        guard let file = file(namespace, key) else { return }
        lock.withLock { try? write(value, to: file) }
    }

    /// Stored where `load(namespace: "usage", key: "ledger")` reads it.
    public func saveUsage(ledgerJson: String) throws {
        guard let file = file("usage", "ledger") else { return }
        try lock.withLock {
            do {
                try write(ledgerJson, to: file)
            } catch {
                throw HostError.Failed(message: error.localizedDescription)
            }
        }
    }

    /// The file of `namespace` and `key`: `<store>/<namespace>/<key>.json`. Both may hold only
    /// letters, digits, `-`, `_` and `.`, and must not start with a dot; `nil` otherwise.
    func file(_ namespace: String, _ key: String) -> URL? {
        guard Self.isName(namespace), Self.isName(key) else { return nil }
        return storeURL.appending(path: namespace, directoryHint: .isDirectory)
            .appending(path: key + ".json", directoryHint: .notDirectory)
    }

    static func isName(_ text: String) -> Bool {
        guard let first = text.unicodeScalars.first, first != "." else { return false }
        return text.unicodeScalars.allSatisfy {
            ("a"..."z").contains($0) || ("A"..."Z").contains($0) || ("0"..."9").contains($0)
                || $0 == "-" || $0 == "_" || $0 == "."
        }
    }

    private func write(_ text: String, to file: URL) throws {
        try FileManager.default.createDirectory(
            at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
        try Data(text.utf8).write(to: file, options: [.atomic])
    }
}

extension URLSession {
    /// The session for every request the core makes. A request may carry a key (in a header or
    /// the URL), so responses are cached in memory only and nothing (cache, cookies,
    /// credentials) is written to disk.
    public static let noStore: URLSession = {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.urlCache = URLCache(memoryCapacity: 16 << 20, diskCapacity: 0)
        configuration.httpCookieStorage = nil
        configuration.httpShouldSetCookies = false
        configuration.urlCredentialStorage = nil
        return URLSession(configuration: configuration)
    }()
}
