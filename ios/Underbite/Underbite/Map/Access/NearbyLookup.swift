import Foundation
import UnderbiteKit

/// Runs a nearby lookup the core planned (`plan_access`, `plan_stocking`, ...): sends each round
/// of its requests through the app's fetch host, hands the answers back, and repeats while the
/// plan holds requests. Requests of one queue go one at a time after their delay; queues run
/// side by side. Nothing here starts a lookup: the map's buttons do.
nonisolated enum NearbyLookup {
    /// Sends one request; nil for no answer (it counts as failed).
    typealias Send = @Sendable (HostRequest) async -> HostResponse?
    /// Waits, as `Task.sleep` does.
    typealias Pause = @Sendable (Duration) async -> Void

    /// The longest wait before a request that is still worth it while someone looks at the
    /// button, milliseconds. A request asked to wait longer (a busy service's back-off) is left
    /// unanswered, which the core counts as failed: it says the service is busy and keeps the
    /// pause for the next press.
    static let longestDelayMs: UInt32 = 30_000
    /// The most rounds one lookup runs (PAD-US pages, Overpass retries).
    static let maxRounds = 12

    /// Runs `plan` to the end, calling `progress` with the results after every round. It runs
    /// off the main actor: the core reads the answers (large public areas) as it goes.
    @concurrent
    static func run(
        _ plan: NearbyPlan, send: @escaping Send, pause: @escaping Pause,
        clock: @escaping @Sendable () -> Int64, deadline: Duration = roundDeadline,
        progress: @escaping @Sendable (NearbyResults) async -> Void = { _ in }
    ) async -> NearbyResults {
        var results = plan.results()
        for _ in 0..<maxRounds {
            let requests = plan.requests()
            guard !requests.isEmpty, !Task.isCancelled else { break }
            let responses = await round(
                requests, send: send, pause: pause, deadline: deadline)
            guard !Task.isCancelled else { break }
            results = plan.read(responses: responses, now: clock())
            await progress(results)
        }
        return results
    }

    /// The requests of a round by queue, each queue in the order given, queues in the order
    /// they first appear.
    static func queues(_ requests: [NearbyRequest]) -> [[NearbyRequest]] {
        var order: [String] = []
        var byQueue: [String: [NearbyRequest]] = [:]
        for request in requests {
            if byQueue[request.queue] == nil { order.append(request.queue) }
            byQueue[request.queue, default: []].append(request)
        }
        return order.compactMap { byQueue[$0] }
    }

    /// Sends one round and gathers the answers that came within `deadline`: a service that
    /// hangs (a busy Overpass queues requests until they time out) does not hold back what the
    /// others answered; what it did not answer counts as failed.
    static func round(
        _ requests: [NearbyRequest], send: @escaping Send, pause: @escaping Pause,
        deadline: Duration = roundDeadline
    ) async -> [NearbyResponse] {
        let queues = queues(requests)
        let answers = Answers()
        await withTaskGroup(of: Bool.self) { group in
            for queue in queues {
                group.addTask {
                    for request in queue {
                        guard !Task.isCancelled else { break }
                        guard request.delayMs <= longestDelayMs else { continue }
                        if request.delayMs > 0 { await pause(.milliseconds(Int(request.delayMs))) }
                        guard !Task.isCancelled else { break }
                        if let response = await send(request.request), !Task.isCancelled {
                            answers.add(NearbyResponse(id: request.id, response: response))
                        }
                    }
                    return true
                }
            }
            group.addTask {
                try? await Task.sleep(for: deadline)
                return false
            }
            var finished = 0
            for await queueDone in group {
                if queueDone { finished += 1 }
                // Every queue is done, or the time is up: stop the rest.
                if !queueDone || finished == queues.count {
                    group.cancelAll()
                    break
                }
            }
        }
        return answers.all
    }

    /// The longest a round may take, waits included.
    static let roundDeadline: Duration = .seconds(60)

    /// The answers of a round as they come, from its queues side by side.
    private final class Answers: @unchecked Sendable {
        private let lock = NSLock()
        private var list: [NearbyResponse] = []
        func add(_ answer: NearbyResponse) { lock.withLock { list.append(answer) } }
        var all: [NearbyResponse] { lock.withLock { list } }
    }
}
