import Foundation

/// Answers every request of a session made with `StubProtocol.session()` from a handler,
/// without the network, and records what was asked.
final class StubProtocol: URLProtocol, @unchecked Sendable {
    /// What to answer: a response and its body, or an error.
    enum Answer {
        case response(status: Int, headers: [String: String], body: Data)
        case failure(URLError)
    }

    private static let lock = NSLock()
    nonisolated(unsafe) private static var answer: Answer = .failure(URLError(.notConnectedToInternet))
    nonisolated(unsafe) private static var requests: [URLRequest] = []

    /// Answers the next requests with `answer` and forgets earlier ones.
    static func answer(_ answer: Answer) {
        lock.withLock {
            self.answer = answer
            requests = []
        }
    }

    /// The requests answered since the last `answer(_:)`, with their bodies read from the
    /// stream `URLSession` turns them into.
    static var received: [URLRequest] { lock.withLock { requests } }

    /// An ephemeral session that only this stub answers, with a memory cache.
    static func session() -> URLSession {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [StubProtocol.self]
        configuration.urlCache = URLCache(memoryCapacity: 1 << 20, diskCapacity: 0)
        return URLSession(configuration: configuration)
    }

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        var seen = request
        if seen.httpBody == nil, let stream = seen.httpBodyStream {
            seen.httpBody = Self.read(stream)
        }
        let answer = Self.lock.withLock {
            Self.requests.append(seen)
            return Self.answer
        }
        switch answer {
        case let .response(status, headers, body):
            guard let url = request.url,
                let response = HTTPURLResponse(
                    url: url, statusCode: status, httpVersion: "HTTP/1.1", headerFields: headers)
            else {
                client?.urlProtocol(self, didFailWithError: URLError(.badURL))
                return
            }
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: body)
            client?.urlProtocolDidFinishLoading(self)
        case let .failure(error):
            client?.urlProtocol(self, didFailWithError: error)
        }
    }

    override func stopLoading() {}

    private static func read(_ stream: InputStream) -> Data {
        stream.open()
        defer { stream.close() }
        var data = Data()
        var buffer = [UInt8](repeating: 0, count: 4096)
        while stream.hasBytesAvailable {
            let count = stream.read(&buffer, maxLength: buffer.count)
            if count <= 0 { break }
            data.append(buffer, count: count)
        }
        return data
    }
}
