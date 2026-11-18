import MapKit
import UIKit

/// Every charted point and reef site as one overlay with one renderer, however many there are:
/// the reef areas under the marks, the marks grouped by the ``StructureModule`` for the zoom
/// shown.
nonisolated final class StructurePinOverlay: NSObject, MKOverlay {
    let coordinate = CLLocationCoordinate2D(latitude: 0, longitude: 0)
    let boundingMapRect = MKMapRect.world
    private let lock = NSLock()
    private var clusters: [StructureCluster] = []
    private var areas: [StructureArea] = []
    private var opacity: [String: Double] = [:]
    private var picked: String?

    /// Replaces the marks, the areas, the layers' opacities and the area picked out; the
    /// renderer is told to draw again by its owner.
    func set(
        _ clusters: [StructureCluster], areas: [StructureArea], opacity: [String: Double],
        picked: String?
    ) {
        lock.withLock {
            self.clusters = clusters
            self.areas = areas
            self.opacity = opacity
            self.picked = picked
        }
    }

    /// The marks whose centre lies in `rect`, with the opacity of the layer of each one's lead.
    func clusters(in rect: MKMapRect) -> [(StructureCluster, Double)] {
        lock.withLock {
            clusters.compactMap { cluster in
                guard rect.contains(MKMapPoint(x: cluster.x, y: cluster.y)),
                    let lead = cluster.marks.first
                else { return nil }
                return (cluster, opacity[lead.layer] ?? 1)
            }
        }
    }

    /// The areas that meet `rect`, with their layer's opacity and whether each is picked out.
    func areas(in rect: MKMapRect) -> [(StructureArea, Double, Bool)] {
        lock.withLock {
            areas.filter { $0.bounds.intersects(rect) }.map {
                ($0, opacity[$0.layer] ?? 1, $0.id == picked)
            }
        }
    }
}

/// Draws a ``StructurePinOverlay``: each reef area lightly filled with a dashed edge, then each
/// mark as its kind's outline in its colour with its symbol, a group as the mark that leads it
/// with their number beside.
nonisolated final class StructurePinRenderer: MKOverlayRenderer {
    /// The largest mark's half size with its number, in points.
    static let margin = 30.0

    private let pins: StructurePinOverlay

    init(_ overlay: StructurePinOverlay) {
        pins = overlay
        super.init(overlay: overlay)
    }

    override func draw(_ mapRect: MKMapRect, zoomScale: MKZoomScale, in context: CGContext) {
        let perPoint = 1 / Double(zoomScale)
        let margin = Self.margin * perPoint
        let grown = mapRect.insetBy(dx: -margin, dy: -margin)
        for (area, opacity, picked) in pins.areas(in: grown) {
            drawArea(area, opacity: opacity, picked: picked, perPoint: perPoint, in: context)
        }
        let marks = pins.clusters(in: grown)
        guard !marks.isEmpty else { return }
        UIGraphicsPushContext(context)
        defer { UIGraphicsPopContext() }
        for (mark, opacity) in marks {
            guard let lead = mark.marks.first else { continue }
            let at = point(for: MKMapPoint(x: mark.x, y: mark.y))
            context.saveGState()
            // From here on, one unit is one point on screen, centred on the mark.
            context.translateBy(x: at.x, y: at.y)
            context.scaleBy(x: CGFloat(perPoint), y: CGFloat(perPoint))
            context.setAlpha(CGFloat(opacity))
            StructureDrawing.mark(lead.kind, count: mark.marks.count, in: context)
            context.restoreGState()
        }
    }

    private func drawArea(
        _ area: StructureArea, opacity: Double, picked: Bool, perPoint: Double,
        in context: CGContext
    ) {
        guard let first = area.corners.first else { return }
        let colour = StructureDrawing.uiColor(StructureMarkKind.reef.colour)
        let alpha = CGFloat(opacity)
        context.saveGState()
        context.beginPath()
        context.move(to: point(for: first))
        for corner in area.corners.dropFirst() {
            context.addLine(to: point(for: corner))
        }
        context.closePath()
        let path = context.path
        context.setFillColor(colour.withAlphaComponent((picked ? 0.32 : 0.16) * alpha).cgColor)
        context.fillPath()
        if let path { context.addPath(path) }
        context.setStrokeColor(colour.withAlphaComponent(0.9 * alpha).cgColor)
        context.setLineWidth(CGFloat((picked ? 3 : 2) * perPoint))
        context.setLineDash(phase: 0, lengths: [CGFloat(5 * perPoint), CGFloat(3 * perPoint)])
        context.strokePath()
        context.restoreGState()
    }
}

/// The drawing of one mark or group, centred on the origin, in points. Used by the renderer (on
/// MapKit's drawing threads) and for the symbols in the card.
nonisolated enum StructureDrawing {
    /// A marker's size on screen, in points.
    static let size = 24.0

    private static let cache = SymbolCache()

    static func uiColor(_ rgb: PinRGB, alpha: CGFloat = 1) -> UIColor {
        UIColor(
            red: CGFloat(rgb.red) / 255, green: CGFloat(rgb.green) / 255,
            blue: CGFloat(rgb.blue) / 255, alpha: alpha)
    }

    /// One mark: its kind's outline with a dark rim and a white edge, so it holds on imagery,
    /// on the chart and on the light map, filled in its colour, its kind's symbol, and with
    /// `count` above one, the number of marks it leads.
    static func mark(_ kind: StructureMarkKind, count: Int = 1, in context: CGContext) {
        let path = outline(kind.outline, size: size)
        context.addPath(path)
        context.setStrokeColor(UIColor.black.withAlphaComponent(0.55).cgColor)
        context.setLineWidth(4.5)
        context.setLineJoin(.round)
        context.strokePath()
        context.addPath(path)
        context.setFillColor(uiColor(kind.colour).cgColor)
        context.fillPath()
        context.addPath(path)
        context.setStrokeColor(UIColor.white.cgColor)
        context.setLineWidth(2)
        context.strokePath()
        if let image = cache.image(kind.symbol, tint: uiColor(kind.glyph), pointSize: 10) {
            let size = image.size
            image.draw(
                in: CGRect(
                    x: -size.width / 2, y: -size.height / 2, width: size.width,
                    height: size.height))
        }
        if count > 1 { number(count, corner: size / 2, in: context) }
    }

    /// An outline `size` points across, centred on the origin.
    static func outline(_ shape: StructureOutline, size: Double) -> CGPath {
        let h = size / 2
        switch shape {
        case .circle:
            return CGPath(
                ellipseIn: CGRect(x: -h, y: -h, width: size, height: size), transform: nil)
        case .square:
            return CGPath(
                roundedRect: CGRect(x: -h, y: -h, width: size, height: size), cornerWidth: 5,
                cornerHeight: 5, transform: nil)
        case .diamond:
            // A little larger, so that it holds a symbol as the square does.
            let d = h * 1.25
            let path = CGMutablePath()
            path.move(to: CGPoint(x: 0, y: -d))
            path.addLine(to: CGPoint(x: d, y: 0))
            path.addLine(to: CGPoint(x: 0, y: d))
            path.addLine(to: CGPoint(x: -d, y: 0))
            path.closeSubpath()
            return path
        }
    }

    /// The number of marks in a group, on an ink capsule at the lead mark's top right.
    private static func number(_ count: Int, corner: Double, in context: CGContext) {
        // A count of marks on screen, not a measured value: nothing for the core to word.
        let text = NSAttributedString(
            string: String(count),
            attributes: [
                .font: UIFont.systemFont(ofSize: 10, weight: .bold),
                .foregroundColor: UIColor.white,
            ])
        let size = text.size()
        let width = max(size.width + 7, 16), height = 16.0
        let capsule = CGRect(
            x: corner + 2 - width / 2, y: -corner - 3 - height / 2, width: width, height: height)
        let path = CGPath(
            roundedRect: capsule, cornerWidth: height / 2, cornerHeight: height / 2, transform: nil)
        context.addPath(path)
        context.setFillColor(uiColor(.ink).cgColor)
        context.fillPath()
        context.addPath(path)
        context.setStrokeColor(UIColor.white.cgColor)
        context.setLineWidth(1.5)
        context.strokePath()
        text.draw(at: CGPoint(x: capsule.midX - size.width / 2, y: capsule.midY - size.height / 2))
    }

    /// SF Symbol images by name, tint and size, made once; MapKit draws on several threads.
    private final class SymbolCache: @unchecked Sendable {
        private let lock = NSLock()
        private var images: [String: UIImage] = [:]

        func image(_ name: String, tint: UIColor, pointSize: CGFloat) -> UIImage? {
            let key = "\(name)|\(tint.description)|\(pointSize)"
            if let held = lock.withLock({ images[key] }) { return held }
            let configuration = UIImage.SymbolConfiguration(pointSize: pointSize, weight: .bold)
            guard
                let made = UIImage(systemName: name, withConfiguration: configuration)?
                    .withTintColor(tint, renderingMode: .alwaysOriginal)
            else { return nil }
            lock.withLock { images[key] = made }
            return made
        }
    }
}

/// The lake contours of the state layers shown as one overlay with one renderer, however many
/// lines there are. The lines change in place (``set(_:opacity:)``), so the overlay keeps its
/// place under the marks.
nonisolated final class LakeContourOverlay: NSObject, MKOverlay {
    let coordinate = CLLocationCoordinate2D(latitude: 0, longitude: 0)
    let boundingMapRect = MKMapRect.world
    private let lock = NSLock()
    private var lines = ContourSet.empty
    private var opacity = 1.0

    /// Replaces the lines and their opacity; the renderer is told to draw again by its owner.
    func set(_ lines: ContourSet, opacity: Double) {
        lock.withLock {
            self.lines = lines
            self.opacity = opacity
        }
    }

    var drawn: (lines: ContourSet, opacity: Double) {
        lock.withLock { (lines, opacity) }
    }
}

/// Draws a ``LakeContourOverlay``: each line a fixed width on screen in a blue apart from the
/// elevation model's contours, index lines heavier, and each label the core placed ("10 ft"),
/// turned along its line on a light halo.
nonisolated final class LakeContourRenderer: MKOverlayRenderer {
    /// Line widths in points.
    static let minorWidth = 1.0
    static let majorWidth = 2.0
    static let labelSize = 11.0
    /// How far from its anchor a label's text may reach, in points.
    static let labelReach = 60.0

    private let contours: LakeContourOverlay
    private let line = UIColor(red: 0.0, green: 0.30, blue: 0.62, alpha: 0.8)
    private let text: [NSAttributedString.Key: Any]
    private let halo: [NSAttributedString.Key: Any]

    init(_ overlay: LakeContourOverlay) {
        contours = overlay
        let font = UIFont.systemFont(ofSize: Self.labelSize, weight: .semibold)
        text = [.font: font, .foregroundColor: UIColor(red: 0.0, green: 0.2, blue: 0.45, alpha: 1)]
        halo = [
            .font: font, .foregroundColor: UIColor.white,
            .strokeColor: UIColor(white: 1, alpha: 0.9), .strokeWidth: 6.0,
        ]
        super.init(overlay: overlay)
    }

    override func draw(_ mapRect: MKMapRect, zoomScale: MKZoomScale, in context: CGContext) {
        let (set, opacity) = contours.drawn
        guard !set.lines.isEmpty else { return }
        let perPoint = 1 / Double(zoomScale)
        context.setAlpha(CGFloat(opacity))
        context.setLineCap(.round)
        context.setLineJoin(.round)
        let lines = set.lines(in: mapRect, margin: Self.majorWidth * perPoint)
        for major in [false, true] {
            context.setLineWidth(CGFloat((major ? Self.majorWidth : Self.minorWidth) * perPoint))
            context.setStrokeColor(line.cgColor)
            for contour in lines where contour.major == major {
                let points = contour.points
                context.move(to: point(for: points[0]))
                for p in points.dropFirst() {
                    context.addLine(to: point(for: p))
                }
            }
            context.strokePath()
        }
        let labels = set.labels(near: mapRect, reach: Self.labelReach * perPoint)
        guard !labels.isEmpty else { return }
        UIGraphicsPushContext(context)
        defer { UIGraphicsPopContext() }
        for label in labels {
            let at = point(for: label.at)
            let size = (label.text as NSString).size(withAttributes: text)
            let origin = CGPoint(x: -size.width / 2, y: -size.height / 2)
            context.saveGState()
            context.translateBy(x: at.x, y: at.y)
            context.rotate(by: CGFloat(label.rotation))
            context.scaleBy(x: CGFloat(perPoint), y: CGFloat(perPoint))
            (label.text as NSString).draw(at: origin, withAttributes: halo)
            (label.text as NSString).draw(at: origin, withAttributes: text)
            context.restoreGState()
        }
    }
}
