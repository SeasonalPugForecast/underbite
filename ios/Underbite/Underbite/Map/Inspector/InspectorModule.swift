import MapKit
import SwiftUI
@preconcurrency import UnderbiteKit

/// A tapped point and what the inspector shows about it.
struct PointPick: Identifiable, Equatable {
    let id = UUID()
    let coordinate: CLLocationCoordinate2D
    /// Where on the map view, in points, when it was tapped: the popover points there.
    let point: CGPoint
    /// The layers on screen at the point, topmost first.
    var rows: [PointRow] = []
    /// The point's coordinates as the core words them.
    var place = ""
    /// The nearest station on screen and its distance (the core's words).
    var nearest: NearestStation?
    /// The water at the point, from the core's lookup (the point card's "water here").
    var water: WaterHereState = .hidden
    /// The words around that section, in the settings' voice.
    var waterWords: WaterHereTexts?

    static func == (a: PointPick, b: PointPick) -> Bool {
        a.id == b.id && a.rows == b.rows && a.place == b.place && a.nearest == b.nearest
            && a.water == b.water && a.waterWords == b.waterWords
    }
}

/// The station nearest a tapped point.
struct NearestStation: Equatable {
    let pin: MapPin
    /// The distance from the point in the user's unit.
    let distance: String
}

/// The point inspector and the measuring tool. It is the bottom module, so it is asked about a
/// tap last. A long press opens the inspector at the point, with the value of every layer on
/// screen there from the grids already held (a long press never fetches) and "Save as spot"; so
/// does a tap no pin took, but only while pin mode is on (``MapTapPolicy``). A line starts from
/// the point's card, and while one is open, taps move its end. Its overlay is the measured
/// line; its annotations are the marks at the point and at the line's ends. The point card also
/// asks the core for the water at the point: the core sends the 3DHP queries with the point
/// rounded outward (the exact point never leaves the device) and places the water on the phone;
/// the lookup stops when the marker moves or the card closes.
@MainActor
@Observable
final class InspectorModule: MapModule {
    /// The point the inspector is open at.
    private(set) var pick: PointPick?
    /// The line being measured.
    private(set) var line: MeasureLine?
    /// Whether a tap on empty map drops the marker; off at launch and not saved.
    var pinMode = false

    @ObservationIgnored private weak var controller: MapController?
    @ObservationIgnored private var lineOverlay: MKGeodesicPolyline?
    @ObservationIgnored private var marks: [InspectorMark] = []
    @ObservationIgnored private var launchDone = false
    @ObservationIgnored private let host: FetchHost
    @ObservationIgnored private let clock: @Sendable () -> Int64
    @ObservationIgnored private let defaults: UserDefaults
    @ObservationIgnored private var waterTask: Task<Void, Never>?

    /// How long a sheet takes to go before the next one comes up.
    static let handOver: Duration = .milliseconds(450)

    init(
        controller: MapController,
        host: FetchHost = URLSessionHost(storeURL: AppFiles.standard.hostDirectory),
        defaults: UserDefaults = .standard,
        clock: @escaping @Sendable () -> Int64 = { Int64(Date().timeIntervalSince1970) }
    ) {
        self.controller = controller
        self.host = host
        self.defaults = defaults
        self.clock = clock
    }

    var layerIDs: Set<String> { [] }

    // MARK: - Drawing

    func overlays(in context: MapModuleContext) -> [any MKOverlay] {
        lineOverlay.map { [$0] } ?? []
    }

    func renderer(for overlay: any MKOverlay) -> MKOverlayRenderer? {
        guard let lineOverlay, overlay === lineOverlay else { return nil }
        let renderer = MKPolylineRenderer(polyline: lineOverlay)
        renderer.strokeColor = UIColor(Palette.accent)
        renderer.lineWidth = 4
        renderer.lineCap = .round
        return renderer
    }

    func annotations(in context: MapModuleContext) -> [any MKAnnotation] { marks }

    func annotationView(for annotation: any MKAnnotation, on map: MKMapView) -> MKAnnotationView? {
        guard let mark = annotation as? InspectorMark else { return nil }
        let view =
            map.dequeueReusableAnnotationView(withIdentifier: InspectorMark.reuse)
            as? InspectorMarkView
            ?? InspectorMarkView(annotation: mark, reuseIdentifier: InspectorMark.reuse)
        view.annotation = mark
        return view
    }

    // MARK: - Taps

    func handleTap(_ tap: MapTap) -> Bool {
        switch MapTapPolicy.action(measuring: line != nil, pinMode: pinMode) {
        case .extendLine:
            if var measuring = line {
                measuring.tap(tap.coordinate)
                setLine(measuring)
            }
            return true
        case .dropMarker:
            open(at: tap.coordinate, point: tap.point)
            return true
        case .ignore:
            return false
        }
    }

    /// A long press: the marker drops and the inspector opens at the point, in either mode.
    func longPress(_ tap: MapTap) {
        open(at: tap.coordinate, point: tap.point)
    }

    // MARK: - The inspector

    /// Opens the inspector at a point.
    func open(at coordinate: CLLocationCoordinate2D, point: CGPoint) {
        var made = PointPick(coordinate: coordinate, point: point)
        fill(&made)
        // The marker moved: the last point's lookup is of no use.
        waterTask?.cancel()
        pick = made
        placeMarks()
        lookUpWater(for: made.id, at: coordinate)
    }

    /// Words the open point again: blocks or pins arrived, or the settings changed.
    func refresh() {
        guard var current = pick else { return }
        fill(&current)
        if current != pick { pick = current }
    }

    /// The inspector was closed.
    func close() {
        waterTask?.cancel()
        waterTask = nil
        guard pick != nil else { return }
        pick = nil
        placeMarks()
    }

    /// The rows, the coordinates and the nearest station of a point.
    private func fill(_ pick: inout PointPick) {
        guard let controller else { return }
        let json = controller.settingsJSON
        let at = pick.coordinate
        let words = try? pointWords(
            point: MapPoint(lat: at.latitude, lon: at.longitude), settingsJson: json)
        let shown = controller.plan.shown
        let values =
            (try? controller.gridStore.inspect(
                lat: at.latitude, lon: at.longitude,
                layers: PointInspection.valueLayers(shown: shown), settingsJson: json,
                timeZone: TimeZone.current.identifier)) ?? []
        pick.rows = PointInspection.rows(
            shown: shown, values: values, imageryOnly: words?.imageryOnly ?? "")
        pick.place = words?.point ?? ""
        pick.waterWords = try? waterHereWords(settingsJson: json)
        pick.nearest = PointInspection.nearestStation(stationPins(), to: at).flatMap { pin in
            let distance = try? measure(
                from: MapPoint(lat: at.latitude, lon: at.longitude),
                to: MapPoint(lat: pin.latitude, lon: pin.longitude), settingsJson: json)
            return distance.map { NearestStation(pin: pin, distance: $0.distance) }
        }
    }

    /// Asks the core for the water at the point and shows what comes: the section appears with
    /// the core's loading words, then with the water, its stocking lines and links, or with the
    /// core's failure words; with no water within reach it goes again. Nothing is asked without
    /// the core planning it, and the answers of a point the marker has left are dropped.
    private func lookUpWater(for id: UUID, at coordinate: CLLocationCoordinate2D) {
        guard let controller else { return }
        let json = controller.settingsJSON
        let now = clock()
        let plan: WaterHerePlan
        do {
            plan = try planWaterHere(
                point: MapPoint(lat: coordinate.latitude, lon: coordinate.longitude),
                region: WaterHereLookup.region(defaults.string(forKey: RecordsModule.regionKey)),
                tideStations: WaterHereLookup.tideStations(stationPins()),
                settingsJson: json, now: now)
        } catch {
            return
        }
        update(id) { $0.water = .looking }
        let host = host
        let clock = clock
        let apply: @MainActor @Sendable (WaterHereResult) -> Void = { [weak self] result in
            self?.update(id) { $0.water = WaterHereState(result) }
        }
        waterTask = Task {
            let result = await WaterHereLookup.run(
                plan, send: { try await host.send(request: $0) }, clock: clock,
                progress: { result in Task { @MainActor in apply(result) } })
            guard !Task.isCancelled else { return }
            apply(result)
        }
    }

    /// Changes the open point if it is still the one `id` names.
    private func update(_ id: UUID, _ change: (inout PointPick) -> Void) {
        guard var current = pick, current.id == id else { return }
        change(&current)
        if current != pick { pick = current }
    }

    /// The station pins on screen, as the stations module draws them.
    private func stationPins() -> [MapPin] {
        guard let controller,
            let stations = controller.modules.lazy.compactMap({ $0 as? StationsModule }).first
        else { return [] }
        return stations.overlays(in: controller.context(for: stations))
            .compactMap { $0 as? StationPinOverlay }
            .flatMap { $0.clusters(in: .world).flatMap(\.pins) }
    }

    // MARK: - Measuring

    /// Measures from the open point: the inspector goes, and the line starts there.
    func measureFromPick() {
        guard let pick else { return }
        startMeasuring(MeasureLine(from: .point(pick.coordinate)))
    }

    /// Starts `line`, once the inspector's sheet has gone if it was up.
    func startMeasuring(_ start: MeasureLine) {
        guard pick != nil else {
            setLine(start)
            return
        }
        close()
        Task { [weak self] in
            try? await Task.sleep(for: Self.handOver)
            self?.setLine(start)
        }
    }

    /// The line starts at `end` instead.
    func setStart(_ end: MeasureLine.End) {
        guard var measuring = line else { return }
        measuring.from = end
        setLine(measuring)
    }

    /// The line ends at `end`.
    func setEnd(_ end: MeasureLine.End) {
        guard var measuring = line else { return }
        measuring.to = end
        setLine(measuring)
    }

    /// Measuring ends; the line goes.
    func stopMeasuring() {
        guard line != nil else { return }
        setLine(nil)
    }

    private func setLine(_ next: MeasureLine?) {
        guard next != line else { return }
        line = next
        let ends = next?.coordinates ?? []
        lineOverlay = ends.count == 2 ? MKGeodesicPolyline(coordinates: ends, count: ends.count) : nil
        placeMarks()
    }

    /// The marks at the open point and at the line's ends; the same objects while they stay.
    private func placeMarks() {
        var wanted: [CLLocationCoordinate2D] = []
        if let pick { wanted.append(pick.coordinate) }
        if let line {
            wanted.append(line.from.coordinate)
            if let to = line.to { wanted.append(to.coordinate) }
        }
        let kept = marks.filter { mark in wanted.contains { $0.same(as: mark.coordinate) } }
        let added = wanted.filter { c in !kept.contains { $0.coordinate.same(as: c) } }
            .map(InspectorMark.init)
        marks = kept + added
        controller?.moduleDidChange()
    }

    // MARK: - Launch

    func regionDidSettle(_ context: MapModuleContext) {
        #if DEBUG
            if !launchDone, let region = context.region {
                launchDone = true
                Task { [weak self] in await self?.launch(in: region) }
            }
        #endif
    }

    #if DEBUG
        /// `-mapInspect <lat>,<lon>` opens the inspector there once the layers on screen have
        /// their grids and pins (or a while has passed); `-mapMeasure <lat>,<lon>,<lat>,<lon>`
        /// measures that line (debug builds, screenshots).
        private func launch(in region: MapRegion) async {
            if let text = LaunchArguments.value(after: "-mapMeasure"),
                let ends = PointInspection.points(text, count: 2)
            {
                try? await Task.sleep(for: .seconds(1))
                setLine(MeasureLine(from: .point(ends[0]), to: .point(ends[1])))
            }
            guard let text = LaunchArguments.value(after: "-mapInspect"),
                let at = PointInspection.points(text, count: 1)?.first
            else { return }
            for _ in 0..<60 {
                try? await Task.sleep(for: .seconds(2))
                if launchReady(at) { break }
            }
            // The map view's top left is the north-west corner of the region shown now (the
            // first one can be from before the map's last layout).
            let shown = controller?.region ?? region
            let corner = MKMapPoint(
                CLLocationCoordinate2D(latitude: shown.north, longitude: shown.west))
            let perPoint = StationPins.mapPointsPerPoint(zoom: shown.zoom)
            let spot = MKMapPoint(at)
            open(
                at: at,
                point: CGPoint(x: (spot.x - corner.x) / perPoint, y: (spot.y - corner.y) / perPoint))
        }

        /// Whether every gridded layer on screen has a row at `at` and the station pins have
        /// come when a station layer is on.
        private func launchReady(_ at: CLLocationCoordinate2D) -> Bool {
            guard let controller else { return true }
            var probe = PointPick(coordinate: at, point: .zero)
            fill(&probe)
            let grids = controller.plan.layers.filter { $0.kind != .tiles && !$0.info.isBase }
            let stationsOn = controller.plan.shown.contains { $0.draw == .stations }
            return probe.rows.filter { row in grids.contains { $0.id == row.layer } }.count
                >= grids.count && (!stationsOn || probe.nearest != nil)
        }
    #endif
}

/// A mark the inspector puts on the map: the tapped point, or an end of the measured line.
final class InspectorMark: NSObject, MKAnnotation {
    static let reuse = "inspector-mark"
    let coordinate: CLLocationCoordinate2D

    init(_ coordinate: CLLocationCoordinate2D) {
        self.coordinate = coordinate
    }
}

/// A dot in the accent colour with a light rim; it takes no touches.
final class InspectorMarkView: MKAnnotationView {
    private let dot = CAShapeLayer()

    override init(annotation: (any MKAnnotation)?, reuseIdentifier: String?) {
        super.init(annotation: annotation, reuseIdentifier: reuseIdentifier)
        let side = 18.0
        frame = CGRect(x: 0, y: 0, width: side, height: side)
        dot.path = CGPath(
            ellipseIn: CGRect(x: 2, y: 2, width: side - 4, height: side - 4), transform: nil)
        dot.lineWidth = 3
        dot.shadowColor = UIColor.black.cgColor
        dot.shadowOpacity = 0.4
        dot.shadowRadius = 2
        dot.shadowOffset = .zero
        layer.addSublayer(dot)
        isUserInteractionEnabled = false
        canShowCallout = false
        displayPriority = .required
        updateColours()
        registerForTraitChanges([UITraitUserInterfaceStyle.self]) {
            (view: InspectorMarkView, _: UITraitCollection) in
            view.updateColours()
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { nil }

    private func updateColours() {
        dot.fillColor = UIColor(Palette.accent).resolvedColor(with: traitCollection).cgColor
        dot.strokeColor = UIColor.white.cgColor
    }
}

extension CLLocationCoordinate2D {
    /// The same place, to the last digit.
    fileprivate func same(as other: CLLocationCoordinate2D) -> Bool {
        latitude == other.latitude && longitude == other.longitude
    }
}
