import Foundation
@preconcurrency import UnderbiteKit

/// Fetches the elevation the depth layers draw for the view on screen into the core's
/// `GridStore`: one area per view and zoom (``DepthArea``), asked through the fetch host and
/// handed to the store with `add_dem`. An area fetched lately that still covers the view is
/// used again while the store holds it (`dem_area_request` says none); one the store has
/// dropped is asked for again. A view too wide for one area (the core refuses it) fetches
/// nothing.
///
/// The latest view decides: an area fetched for an earlier one still lands in the store, but
/// only the one the latest view wants is reported ready. A busy service is asked again once it
/// may be, if the view has not changed by then.
@MainActor
final class DepthLoader {
    /// Where the elevation for the latest view stands.
    nonisolated enum State: Equatable, Sendable {
        /// Nothing asked yet.
        case idle
        /// The area is being fetched.
        case loading(DepthArea)
        /// The store holds the area: draw from it.
        case ready(DepthArea)
        /// The view is too wide for one area.
        case tooWide
        /// The fetch failed; a later view asks again.
        case failed(DepthArea)
    }

    /// One request and the area it is for.
    private nonisolated struct Ask: Sendable {
        let area: DepthArea
        let request: GridRequest
    }

    /// The longest pause a busy service asks for that the loader waits out by itself.
    static let longestWait: UInt32 = 120

    let store: GridStore
    private let host: FetchHost
    private let clock: @Sendable () -> Int64
    private(set) var state: State = .idle {
        didSet { if state != oldValue { onChange(state) } }
    }
    /// The state changed.
    var onChange: (State) -> Void = { _ in }
    private(set) var held = DepthAreas()
    /// The key of the area the latest view wants.
    private var wanted: String?
    private var inFlight: Set<String> = []

    init(host: FetchHost, store: GridStore, clock: @escaping @Sendable () -> Int64) {
        self.host = host
        self.store = store
        self.clock = clock
    }

    /// Brings the elevation for `region` into the store. The task ends when the fetch it started
    /// does (tests wait for it); none when nothing needs fetching.
    @discardableResult
    func load(_ region: MapRegion) -> Task<Void, Never>? {
        if let area = held.covering(region) {
            switch ask(area) {
            case .held:
                wanted = nil
                state = .ready(area)
                return nil
            case .fetch(let ask):
                // The store dropped it: the same area again.
                held.forget(area)
                return fetch(ask, region: region)
            case .refused:
                held.forget(area)
            }
        }
        // A margin when the view leaves room for one, else the view alone.
        for area in [DepthArea.around(region), DepthArea.around(region, margin: 0)] {
            switch ask(area) {
            case .held:
                held.hold(area)
                wanted = nil
                state = .ready(area)
                return nil
            case .fetch(let ask):
                return fetch(ask, region: region)
            case .refused:
                continue
            }
        }
        wanted = nil
        state = .tooWide
        return nil
    }

    private enum Asked {
        case held
        case fetch(Ask)
        case refused
    }

    private func ask(_ area: DepthArea) -> Asked {
        do {
            guard
                let request = try store.demAreaRequest(
                    west: area.west, south: area.south, east: area.east, north: area.north,
                    zoom: area.zoom)
            else { return .held }
            return .fetch(Ask(area: area, request: request))
        } catch {
            return .refused
        }
    }

    private func fetch(_ ask: Ask, region: MapRegion) -> Task<Void, Never>? {
        let key = ask.request.key
        wanted = key
        state = .loading(ask.area)
        guard !inFlight.contains(key) else { return nil }
        inFlight.insert(key)
        let (host, store, clock) = (host, store, clock)
        return Task {
            let outcome = await Self.fetch(ask, host: host, store: store, clock: clock)
            self.landed(ask, outcome: outcome, region: region)
        }
    }

    /// What a fetch came to.
    private nonisolated enum Outcome: Sendable {
        case added
        case busy(after: UInt32)
        case failed
    }

    private nonisolated static func fetch(
        _ ask: Ask, host: FetchHost, store: GridStore, clock: @Sendable () -> Int64
    ) async -> Outcome {
        do {
            let response = try await host.send(request: ask.request.request)
            try store.addDem(key: ask.request.key, response: response, now: clock())
            return .added
        } catch MapError.RetryLater(let after) {
            return .busy(after: after)
        } catch {
            return .failed
        }
    }

    private func landed(_ ask: Ask, outcome: Outcome, region: MapRegion) {
        let key = ask.request.key
        inFlight.remove(key)
        if case .added = outcome { held.hold(ask.area) }
        guard wanted == key else { return }
        switch outcome {
        case .added:
            wanted = nil
            state = .ready(ask.area)
        case .busy(let after):
            state = .failed(ask.area)
            guard after <= Self.longestWait else { return }
            Task {
                try? await Task.sleep(for: .seconds(Int(after) + 1))
                if self.wanted == key { self.load(region) }
            }
        case .failed:
            wanted = nil
            state = .failed(ask.area)
        }
    }
}
