import MapKit
import UIKit

/// What a ``MineOverlay`` holds, swapped whole under a lock: MapKit draws on several threads.
nonisolated private final class Locked<Value>: @unchecked Sendable {
    private let lock = NSLock()
    private var value: Value

    init(_ value: Value) { self.value = value }

    func get() -> Value { lock.withLock { value } }
    func set(_ new: Value) { lock.withLock { value = new } }
    func update<T>(_ body: (inout Value) -> T) -> T { lock.withLock { body(&value) } }
}

/// One of the "Mine" layers as one overlay over the whole world, however much it holds; its
/// renderer draws what lies in each tile. An empty one draws nothing, so the three overlays stay
/// on the map in their order (heat map, tracks, catches) while their layers come and go.
nonisolated class MineOverlay: NSObject, MKOverlay {
    let coordinate = CLLocationCoordinate2D(latitude: 0, longitude: 0)
    let boundingMapRect = MKMapRect.world
}

// MARK: - Catches

/// Every catch mark on the map. The marks (``CatchMark``) are set by the ``MineModule`` for the
/// zoom shown; the renderer draws them a fixed size on screen.
nonisolated final class CatchPinOverlay: MineOverlay {
    private let marks = Locked<[CatchMark]>([])

    func set(_ new: [CatchMark]) { marks.set(new) }

    /// The marks whose centre lies in `rect`.
    func marks(in rect: MKMapRect) -> [CatchMark] {
        marks.get().filter { rect.contains(MKMapPoint(x: $0.at.x, y: $0.at.y)) }
    }
}

/// Draws a ``CatchPinOverlay``: each catch as a disc with a fish, a group of catches as one disc
/// with their number beside.
nonisolated final class CatchPinRenderer: MKOverlayRenderer {
    /// The marks' size on screen, in points.
    static let size = 24.0
    /// The largest mark's half size with its number, in points.
    static let margin = 32.0

    private let pins: CatchPinOverlay

    init(_ overlay: CatchPinOverlay) {
        pins = overlay
        super.init(overlay: overlay)
    }

    override func draw(_ mapRect: MKMapRect, zoomScale: MKZoomScale, in context: CGContext) {
        let perPoint = 1 / Double(zoomScale)
        let margin = Self.margin * perPoint
        let marks = pins.marks(in: mapRect.insetBy(dx: -margin, dy: -margin))
        guard !marks.isEmpty else { return }
        UIGraphicsPushContext(context)
        defer { UIGraphicsPopContext() }
        for mark in marks {
            let at = point(for: MKMapPoint(x: mark.at.x, y: mark.at.y))
            context.saveGState()
            // From here on, one unit is one point on screen, centred on the mark.
            context.translateBy(x: at.x, y: at.y)
            context.scaleBy(x: CGFloat(perPoint), y: CGFloat(perPoint))
            CatchDrawing.mark(count: mark.catches.count, in: context)
            context.restoreGState()
        }
    }
}

/// The drawing of a catch mark, centred on the origin, in points.
nonisolated enum CatchDrawing {
    /// The disc's fill: a deep raspberry that holds on the light map and on imagery, apart from
    /// the stations' colours; the white fish on it has a contrast above 7:1.
    static let fill = UIColor(red: 0xA0 / 255.0, green: 0x19 / 255.0, blue: 0x5A / 255.0, alpha: 1)
    /// The number's capsule.
    static let ink = UIColor(red: 0x1B / 255.0, green: 0x1B / 255.0, blue: 0x1F / 255.0, alpha: 1)
    /// The symbol drawn on the disc.
    static let symbol = "fish.fill"

    private static let cache = Locked<UIImage?>(nil)

    static func mark(count: Int, in context: CGContext) {
        let side = CatchPinRenderer.size
        let disc = CGRect(x: -side / 2, y: -side / 2, width: side, height: side)
        context.setStrokeColor(UIColor.black.withAlphaComponent(0.55).cgColor)
        context.setLineWidth(4.5)
        context.strokeEllipse(in: disc)
        context.setFillColor(fill.cgColor)
        context.fillEllipse(in: disc)
        context.setStrokeColor(UIColor.white.cgColor)
        context.setLineWidth(2)
        context.strokeEllipse(in: disc)
        if let image = glyph() {
            let size = image.size
            image.draw(
                in: CGRect(
                    x: -size.width / 2, y: -size.height / 2, width: size.width,
                    height: size.height))
        }
        if count > 1 { number(count, in: context) }
    }

    /// The number of catches in a group, on an ink capsule at the mark's top right.
    private static func number(_ count: Int, in context: CGContext) {
        // A count of marks on screen, not a measured value: nothing for the core to word.
        let text = NSAttributedString(
            string: String(count),
            attributes: [
                .font: UIFont.systemFont(ofSize: 10, weight: .bold),
                .foregroundColor: UIColor.white,
            ])
        let size = text.size()
        let width = max(size.width + 7, 16), height = 16.0
        let capsule = CGRect(x: 14 - width / 2, y: -15 - height / 2, width: width, height: height)
        let path = CGPath(
            roundedRect: capsule, cornerWidth: height / 2, cornerHeight: height / 2, transform: nil)
        context.addPath(path)
        context.setFillColor(ink.cgColor)
        context.fillPath()
        context.addPath(path)
        context.setStrokeColor(UIColor.white.cgColor)
        context.setLineWidth(1.5)
        context.strokePath()
        text.draw(at: CGPoint(x: capsule.midX - size.width / 2, y: capsule.midY - size.height / 2))
    }

    /// The white fish, made once.
    private static func glyph() -> UIImage? {
        cache.update { held in
            if let held { return held }
            let configuration = UIImage.SymbolConfiguration(pointSize: 11, weight: .bold)
            held = UIImage(systemName: symbol, withConfiguration: configuration)?
                .withTintColor(.white, renderingMode: .alwaysOriginal)
            return held
        }
    }
}

// MARK: - Tracks

/// Every imported track as one overlay: its segments in map points, simplified per zoom level
/// the first time that level is drawn.
nonisolated final class TrackOverlay: MineOverlay {
    private struct State {
        var lines: [TrackLine] = []
        var simplified: [Int: [TrackLine]] = [:]
    }

    private let state = Locked(State())

    /// Replaces the lines; the renderer is told to draw again by its owner.
    func set(_ lines: [TrackLine]) {
        state.set(State(lines: lines))
    }

    var isEmpty: Bool { state.get().lines.isEmpty }

    /// The lines of zoom level `level` (``TrackLines/level(zoomScale:)``) whose box meets `rect`.
    func lines(level: Int, meeting rect: MKMapRect) -> [TrackLine] {
        let all = state.update { state -> [TrackLine] in
            if let held = state.simplified[level] { return held }
            let tolerance = TrackLines.tolerance(level: level)
            let made = state.lines.map { TrackLines.simplify($0, tolerance: tolerance) }
            state.simplified[level] = made
            return made
        }
        return all.filter {
            $0.meets(minX: rect.minX, minY: rect.minY, maxX: rect.maxX, maxY: rect.maxY)
        }
    }
}

/// Draws a ``TrackOverlay``: each line in the tracks' colour on a white casing, so it holds on
/// the map and on imagery. All of them are one path.
nonisolated final class TrackRenderer: MKOverlayRenderer {
    /// The line's width on screen, in points, and its casing's.
    static let width = 3.5
    static let casing = 6.5
    /// An indigo apart from water, the heat map and the catches.
    static let colour = UIColor(red: 0x3F / 255.0, green: 0x51 / 255.0, blue: 0xB5 / 255.0, alpha: 1)

    private let tracks: TrackOverlay

    init(_ overlay: TrackOverlay) {
        tracks = overlay
        super.init(overlay: overlay)
    }

    override func draw(_ mapRect: MKMapRect, zoomScale: MKZoomScale, in context: CGContext) {
        let perPoint = 1 / Double(zoomScale)
        let margin = Self.casing * perPoint
        let lines = tracks.lines(
            level: TrackLines.level(zoomScale: Double(zoomScale)),
            meeting: mapRect.insetBy(dx: -margin, dy: -margin))
        guard !lines.isEmpty else { return }
        let path = CGMutablePath()
        for line in lines {
            guard let first = line.points.first else { continue }
            path.move(to: point(for: MKMapPoint(x: first.x, y: first.y)))
            for p in line.points.dropFirst() {
                path.addLine(to: point(for: MKMapPoint(x: p.x, y: p.y)))
            }
        }
        context.setLineJoin(.round)
        context.setLineCap(.round)
        context.addPath(path)
        context.setStrokeColor(UIColor.white.withAlphaComponent(0.9).cgColor)
        context.setLineWidth(CGFloat(Self.casing * perPoint))
        context.strokePath()
        context.addPath(path)
        context.setStrokeColor(Self.colour.cgColor)
        context.setLineWidth(CGFloat(Self.width * perPoint))
        context.strokePath()
    }
}

// MARK: - The heat map

/// The catch heat map's squares.
nonisolated final class HeatOverlay: MineOverlay {
    private let squares = Locked<[HeatSquare]>([])

    func set(_ new: [HeatSquare]) { squares.set(new) }

    /// The squares that reach into `rect`.
    func squares(meeting rect: MKMapRect) -> [HeatSquare] {
        squares.get().filter {
            rect.intersects(
                MKMapRect(
                    x: $0.at.x - $0.side / 2, y: $0.at.y - $0.side / 2, width: $0.side,
                    height: $0.side))
        }
    }
}

/// Draws a ``HeatOverlay``: each square in the ramp's colour for its share of the catches, the
/// fuller ones more opaque.
nonisolated final class HeatRenderer: MKOverlayRenderer {
    private let heat: HeatOverlay

    init(_ overlay: HeatOverlay) {
        heat = overlay
        super.init(overlay: overlay)
    }

    override func draw(_ mapRect: MKMapRect, zoomScale: MKZoomScale, in context: CGContext) {
        for square in heat.squares(meeting: mapRect) {
            let corner = point(
                for: MKMapPoint(x: square.at.x - square.side / 2, y: square.at.y - square.side / 2))
            let rect = CGRect(
                x: corner.x, y: corner.y, width: CGFloat(square.side), height: CGFloat(square.side))
            let (r, g, b) = CatchHeat.colour(square.fraction)
            let radius = rect.width * 0.2
            context.addPath(
                CGPath(roundedRect: rect, cornerWidth: radius, cornerHeight: radius, transform: nil))
            context.setFillColor(
                UIColor(
                    red: CGFloat(r), green: CGFloat(g), blue: CGFloat(b),
                    alpha: CGFloat(CatchHeat.alpha(square.fraction))
                ).cgColor)
            context.fillPath()
        }
    }
}
