import CoreLocation
import Foundation

/// The phone's location, "when in use" only and asked once; never "always". A fix is at about
/// 100 m, which is enough: a place moves only when the phone is 1 km away.
enum CurrentLocation {
    enum Failure: LocalizedError {
        case denied, unavailable

        var errorDescription: String? {
            switch self {
            case .denied: "Location access is off for Underbite (Settings, Privacy)."
            case .unavailable: "The location could not be found."
            }
        }
    }

    /// A fix: where, how well, and when it was taken.
    struct Fix: Sendable {
        var latitude: Double
        var longitude: Double
        var accuracy: Double
        var taken: Date
    }

    private static var last: Fix?

    /// One fix. With `reuseRecent` one under ten minutes old answers (coming back to the app);
    /// asked for (pull to refresh, "where am I") a new one is read. Gives up after `timeout`
    /// seconds (no signal, or a permission prompt left alone).
    static func fix(timeout: Double = 12, reuseRecent: Bool = false) async throws -> Fix {
        #if DEBUG
            if let fake = debugFix() { return try fake.get() }
        #endif
        if reuseRecent, let last, Date().timeIntervalSince(last.taken) < RefreshPolicy.fixLasts {
            let status = CLLocationManager().authorizationStatus
            if status == .denied || status == .restricted {
                self.last = nil
                throw Failure.denied
            }
            return last
        }
        let location = try await withThrowingTaskGroup(of: CLLocation.self) { group in
            group.addTask { try await liveFix() }
            group.addTask {
                try await Task.sleep(for: .seconds(timeout))
                throw Failure.unavailable
            }
            defer { group.cancelAll() }
            // INVARIANT: the group has two tasks and one of them always finishes.
            return try await group.next()!
        }
        let fix = Fix(
            latitude: location.coordinate.latitude, longitude: location.coordinate.longitude,
            accuracy: location.horizontalAccuracy, taken: location.timestamp)
        last = fix
        return fix
    }

    private static func liveFix() async throws -> CLLocation {
        let session = CLServiceSession(authorization: .whenInUse)
        defer { withExtendedLifetime(session) {} }
        switch CLLocationManager().authorizationStatus {
        case .authorizedWhenInUse, .authorizedAlways:
            return try await CoarseFix().location()
        case .denied, .restricted:
            throw Failure.denied
        default:
            break
        }
        for try await update in CLLocationUpdate.liveUpdates() {
            if update.authorizationDenied || update.authorizationDeniedGlobally {
                throw Failure.denied
            }
            if let location = update.location { return location }
        }
        throw Failure.unavailable
    }

    #if DEBUG
        /// The simulator has no location: `-fakeLocation lat:lon` gives the fix, `-fakeLocation off`
        /// refuses access.
        private static func debugFix() -> Result<Fix, Failure>? {
            let args = ProcessInfo.processInfo.arguments
            guard let i = args.firstIndex(of: "-fakeLocation"), i + 1 < args.count else { return nil }
            let parts = args[i + 1].split(separator: ":")
            if parts.first == "off" { return .failure(.denied) }
            guard parts.count >= 2, let lat = Double(parts[0]), let lon = Double(parts[1]) else {
                return .failure(.unavailable)
            }
            return .success(Fix(latitude: lat, longitude: lon, accuracy: 100, taken: Date()))
        }
    #endif
}

/// One fix at about a hundred metres, once location access is given (Wi-Fi where it answers,
/// rather than the best accuracy, which keeps GPS on).
@MainActor
private final class CoarseFix: NSObject, CLLocationManagerDelegate {
    private let manager = CLLocationManager()
    private var continuation: CheckedContinuation<CLLocation, any Error>?

    func location() async throws -> CLLocation {
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                guard !Task.isCancelled else {
                    continuation.resume(throwing: CancellationError())
                    return
                }
                self.continuation = continuation
                manager.delegate = self
                manager.desiredAccuracy = kCLLocationAccuracyHundredMeters
                manager.requestLocation()
            }
        } onCancel: {
            Task { @MainActor in self.finish(.failure(CancellationError())) }
        }
    }

    private func finish(_ result: Result<CLLocation, any Error>) {
        guard let continuation else { return }
        self.continuation = nil
        continuation.resume(with: result)
    }

    nonisolated func locationManager(_ manager: CLLocationManager, didUpdateLocations locations: [CLLocation]) {
        guard let location = locations.last else { return }
        Task { @MainActor in self.finish(.success(location)) }
    }

    nonisolated func locationManager(_ manager: CLLocationManager, didFailWithError error: any Error) {
        let denied = (error as? CLError)?.code == .denied
        Task { @MainActor in
            self.finish(.failure(denied ? CurrentLocation.Failure.denied : CurrentLocation.Failure.unavailable))
        }
    }
}

extension SpotStore {
    /// Reads where the phone is and notes it in the store (memory only); returns whether the
    /// phone moved 1 km or more since the last fix.
    @discardableResult
    func locate(reuseRecent: Bool = false) async throws -> Bool {
        let fix = try await CurrentLocation.fix(reuseRecent: reuseRecent)
        return noteFix(latitude: fix.latitude, longitude: fix.longitude, accuracy: fix.accuracy)
    }
}
