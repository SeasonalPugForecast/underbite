import CoreLocation
import MapKit
import Observation

/// The map's "you are here": location access as it stands, the locate button's taps, and whether
/// the map follows the phone's position. Opening the map never asks for access; the button does,
/// "when in use" only. MapKit draws the position (its blue dot, with the circle of an approximate
/// position) on this device: nothing here reads the position or sends it anywhere.
@MainActor
@Observable
final class MapLocator: NSObject, CLLocationManagerDelegate {
    /// The key of the purpose string for precise location, in the app's
    /// `NSLocationTemporaryUsageDescriptionDictionary`.
    static let precisePurposeKey = "MapPosition"

    private(set) var state = LocateState()
    /// Location access is off and a tap asked for the position: the map says where to turn it on.
    var explainsRefused = false

    @ObservationIgnored private let manager = CLLocationManager()

    #if DEBUG
        /// `-fakeLocation lat:lon` (screenshots, where the simulator has no location access): a
        /// fixed position drawn as if access were given; `-fakeLocation off` (or anything else)
        /// stands for refused access.
        let simulated: CLLocationCoordinate2D?
    #endif

    override init() {
        #if DEBUG
            let fake = LaunchArguments.value(after: "-fakeLocation")
            simulated = fake.flatMap(Self.position)
        #endif
        super.init()
        #if DEBUG
            if fake != nil {
                _ = state.accessChanged(
                    to: simulated == nil ? .refused : .allowed, precise: simulated != nil)
                return
            }
        #endif
        // The delegate hears the access as it stands right away, then each change.
        manager.delegate = self
    }

    /// The locate button was tapped.
    func tap() {
        for step in state.tap() {
            switch step {
            case .askAccess:
                manager.requestWhenInUseAuthorization()
            case .explainRefused:
                explainsRefused = true
            case .askPrecise:
                manager.requestTemporaryFullAccuracyAuthorization(
                    withPurposeKey: Self.precisePurposeKey)
            case .follow, .stopFollowing:
                // The map follows `state.following`.
                break
            }
        }
    }

    /// The map stopped following by itself (moved by hand).
    func mapStoppedFollowing() {
        guard state.following else { return }
        state.stoppedFollowing()
    }

    private func accessChanged(_ access: LocateState.Access, precise: Bool) {
        var next = state
        _ = next.accessChanged(to: access, precise: precise)
        if next != state { state = next }
    }

    nonisolated func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
        let access = LocateState.Access(manager.authorizationStatus)
        let precise = manager.accuracyAuthorization == .fullAccuracy
        Task { @MainActor in self.accessChanged(access, precise: precise) }
    }

    #if DEBUG
        private static func position(_ text: String) -> CLLocationCoordinate2D? {
            let parts = text.split(separator: ":").compactMap { Double($0) }
            guard parts.count == 2 else { return nil }
            return CLLocationCoordinate2D(latitude: parts[0], longitude: parts[1])
        }
    #endif
}

nonisolated extension LocateState.Access {
    init(_ status: CLAuthorizationStatus) {
        switch status {
        case .authorizedWhenInUse, .authorizedAlways: self = .allowed
        case .denied, .restricted: self = .refused
        default: self = .undecided
        }
    }
}

#if DEBUG
    /// The fixed position of `-fakeLocation`. MapKit's own user-location view takes only the
    /// map's `MKUserLocation`, so this one is drawn alike: a blue dot in a white ring, in a circle
    /// of the 100 m a fix is good for.
    final class SimulatedPosition: NSObject, MKAnnotation {
        static let reuse = "simulated-position"
        let coordinate: CLLocationCoordinate2D
        let accuracy: MKCircle

        init(_ coordinate: CLLocationCoordinate2D) {
            self.coordinate = coordinate
            accuracy = MKCircle(center: coordinate, radius: 100)
        }

        static func renderer(for circle: MKCircle) -> MKOverlayRenderer {
            let renderer = MKCircleRenderer(circle: circle)
            renderer.fillColor = UIColor.systemBlue.withAlphaComponent(0.15)
            renderer.strokeColor = UIColor.systemBlue.withAlphaComponent(0.3)
            renderer.lineWidth = 1
            return renderer
        }
    }

    /// The dot of ``SimulatedPosition``.
    final class SimulatedPositionView: MKAnnotationView {
        override init(annotation: (any MKAnnotation)?, reuseIdentifier: String?) {
            super.init(annotation: annotation, reuseIdentifier: reuseIdentifier)
            frame = CGRect(x: 0, y: 0, width: 24, height: 24)
            backgroundColor = .clear
            let ring = CALayer()
            ring.frame = bounds
            ring.cornerRadius = 12
            ring.backgroundColor = UIColor.white.cgColor
            ring.shadowColor = UIColor.black.cgColor
            ring.shadowOpacity = 0.3
            ring.shadowRadius = 3
            ring.shadowOffset = .zero
            let dot = CALayer()
            dot.frame = bounds.insetBy(dx: 4, dy: 4)
            dot.cornerRadius = 8
            dot.backgroundColor = UIColor.systemBlue.cgColor
            layer.addSublayer(ring)
            layer.addSublayer(dot)
            zPriority = .max
        }

        required init?(coder: NSCoder) { nil }
    }
#endif
