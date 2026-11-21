import MapKit
import UIKit

/// The storms layer as one overlay over the whole world, holding a ``StormDrawing`` swapped whole
/// under a lock (MapKit draws on several threads); its renderer draws every band, line, cone and
/// mark, so the layer is one overlay with one renderer however many tracks it has.
nonisolated final class StormOverlay: NSObject, MKOverlay, @unchecked Sendable {
    let coordinate = CLLocationCoordinate2D(latitude: 0, longitude: 0)
    let boundingMapRect = MKMapRect.world

    private let lock = NSLock()
    private var drawing = StormDrawing()

    func set(_ new: StormDrawing) { lock.withLock { drawing = new } }
    func get() -> StormDrawing { lock.withLock { drawing } }
}

/// Draws a ``StormOverlay``: shapes in map points, widths and marks a fixed size on screen. A
/// shape that passes the world's edge (across the antimeridian) is drawn again one world over, so
/// it shows on both sides.
nonisolated final class StormRenderer: MKOverlayRenderer {
    /// How far a mark's plate reaches from its point, in points: marks this close to a tile are
    /// drawn in it too.
    static let markMargin = 220.0

    private let storms: StormOverlay

    init(_ overlay: StormOverlay) {
        storms = overlay
        super.init(overlay: overlay)
    }

    override func draw(_ mapRect: MKMapRect, zoomScale: MKZoomScale, in context: CGContext) {
        let drawing = storms.get()
        guard !drawing.isEmpty else { return }
        let perPoint = 1 / Double(zoomScale)
        let world = MKMapSize.world.width
        let reach = 16 * perPoint
        for shape in drawing.shapes {
            for shift in [-world, 0, world] {
                let bounds = shape.bounds.offsetBy(dx: shift, dy: 0).insetBy(dx: -reach, dy: -reach)
                guard bounds.intersects(mapRect) else { continue }
                draw(shape, shift: shift, perPoint: perPoint, in: context)
            }
        }
        let margin = Self.markMargin * perPoint
        UIGraphicsPushContext(context)
        defer { UIGraphicsPopContext() }
        let kept = StormMarkDrawing.keptLabels(drawing.marks, perPoint: perPoint)
        for (index, mark) in drawing.marks.enumerated() {
            for shift in [-world, 0, world] {
                let at = MKMapPoint(x: mark.at.x + shift, y: mark.at.y)
                guard mapRect.insetBy(dx: -margin, dy: -margin).contains(at) else { continue }
                let p = point(for: at)
                context.saveGState()
                // From here on, one unit is one point on screen, centred on the mark.
                context.translateBy(x: p.x, y: p.y)
                context.scaleBy(x: CGFloat(perPoint), y: CGFloat(perPoint))
                StormMarkDrawing.draw(mark, labelled: kept[index], in: context)
                context.restoreGState()
            }
        }
    }

    private func path(_ shape: StormDrawing.Shape, shift: Double, closed: Bool) -> CGPath {
        let path = CGMutablePath()
        for (i, p) in shape.points.enumerated() {
            let at = point(for: MKMapPoint(x: p.x + shift, y: p.y))
            if i == 0 { path.move(to: at) } else { path.addLine(to: at) }
        }
        if closed { path.closeSubpath() }
        return path
    }

    private func draw(
        _ shape: StormDrawing.Shape, shift: Double, perPoint: Double, in context: CGContext
    ) {
        let colour = UIColor(shape.colour)
        let scale = CGFloat(perPoint)
        context.saveGState()
        defer { context.restoreGState() }
        context.setLineCap(.round)
        context.setLineJoin(.round)
        switch shape.style {
        case .cone:
            let ring = path(shape, shift: shift, closed: true)
            context.addPath(ring)
            context.setFillColor(colour.withAlphaComponent(StormMapRules.coneFillAlpha).cgColor)
            context.fillPath()
            context.addPath(ring)
            context.setStrokeColor(colour.cgColor)
            context.setLineWidth(StormMapRules.coneEdgeWidth * scale)
            context.strokePath()
        case .band(let width, let dash):
            // The outline of the line stroked at the band's width, overlaps merged so the fill
            // is even, filled translucent and edged in the full colour.
            let line = path(shape, shift: shift, closed: false)
            let outline = line.copy(
                strokingWithWidth: width * scale, lineCap: .round, lineJoin: .round,
                miterLimit: 10
            ).normalized()
            context.addPath(outline)
            context.setFillColor(colour.withAlphaComponent(StormMapRules.fillAlpha).cgColor)
            context.fillPath()
            context.addPath(outline)
            context.setStrokeColor(colour.cgColor)
            context.setLineWidth(StormMapRules.edgeWidth * scale)
            if !dash.isEmpty {
                context.setLineDash(phase: 0, lengths: dash.map { CGFloat($0) * scale })
            }
            context.strokePath()
        case .line(let width):
            let line = path(shape, shift: shift, closed: false)
            context.addPath(line)
            context.setStrokeColor(UIColor.white.withAlphaComponent(0.85).cgColor)
            context.setLineWidth((width + 2 * StormMapRules.casingWidth) * scale)
            context.strokePath()
            context.addPath(line)
            context.setStrokeColor(colour.cgColor)
            context.setLineWidth(width * scale)
            context.strokePath()
        case .dotted(let width, let dash):
            let line = path(shape, shift: shift, closed: false)
            context.addPath(line)
            context.setStrokeColor(colour.withAlphaComponent(0.85).cgColor)
            context.setLineWidth(width * scale)
            context.setLineDash(phase: 0, lengths: dash.map { CGFloat($0) * scale })
            context.strokePath()
        }
    }
}

/// The marks, drawn centred on the origin in points: a low's dot with its model and pressure on
/// a plate beside it, a day's small dot with its time, and a cyclone's symbol with its name.
/// Plates are white with dark text, edged in the mark's colour, so they read on every base map.
nonisolated enum StormMarkDrawing {
    /// The labels' font: bold, a fixed size on the map.
    private static func labelFont(_ size: CGFloat) -> UIFont {
        UIFont.systemFont(ofSize: size, weight: .bold)
    }

    /// A label on its plate with its top left corner at `origin`; returns the plate's size.
    @discardableResult
    private static func plate(
        _ text: String, colour: UIColor, font: UIFont, origin: CGPoint, in context: CGContext
    ) -> CGSize {
        let attributes: [NSAttributedString.Key: Any] = [
            .font: font, .foregroundColor: UIColor.black,
        ]
        let string = NSAttributedString(string: text, attributes: attributes)
        let size = string.size()
        let rect = CGRect(
            x: origin.x, y: origin.y, width: ceil(size.width) + 12, height: ceil(size.height) + 4)
        let shape = UIBezierPath(roundedRect: rect, cornerRadius: 6)
        UIColor.white.withAlphaComponent(0.92).setFill()
        shape.fill()
        colour.setStroke()
        shape.lineWidth = 2
        shape.stroke()
        string.draw(at: CGPoint(x: rect.minX + 6, y: rect.minY + 2))
        return rect.size
    }

    /// The size a label's plate takes.
    private static func plateSize(_ text: String, font: UIFont) -> CGSize {
        let size = NSAttributedString(string: text, attributes: [.font: font]).size()
        return CGSize(width: ceil(size.width) + 12, height: ceil(size.height) + 4)
    }

    private static func dot(radius: CGFloat, colour: UIColor, edge: UIColor, in context: CGContext) {
        let rect = CGRect(x: -radius, y: -radius, width: 2 * radius, height: 2 * radius)
        context.setFillColor(colour.cgColor)
        context.fillEllipse(in: rect)
        context.setStrokeColor(edge.cgColor)
        context.setLineWidth(2)
        context.strokeEllipse(in: rect)
    }

    /// Where a mark's label plate lies, in points from the mark's own point.
    static func labelRect(_ mark: StormDrawing.Mark) -> CGRect {
        switch mark.kind {
        case .low(let storm, let right, let up):
            let radius: CGFloat = storm ? 7 : 5
            let size = plateSize(mark.text, font: labelFont(12))
            let gap: CGFloat = 4
            let x = right ? radius + gap : -radius - gap - size.width
            let y = up ? -size.height - 1 : 1
            return CGRect(origin: CGPoint(x: x, y: y), size: size)
        case .day:
            return CGRect(
                origin: CGPoint(x: 7, y: 2), size: plateSize(mark.text, font: labelFont(11)))
        case .hurricane:
            return CGRect(
                origin: CGPoint(x: CGFloat(StormMapRules.symbolSize) / 2 + 4, y: 2),
                size: plateSize(mark.text, font: labelFont(13)))
        }
    }

    /// The marks' labels to draw at `perPoint` map points per screen point: those that do not
    /// overlap a label of a higher rank (see ``StormMapRules/keptLabels(_:ranks:gap:)``).
    static func keptLabels(_ marks: [StormDrawing.Mark], perPoint: Double) -> [Bool] {
        let rects = marks.map { mark -> CGRect in
            labelRect(mark).offsetBy(
                dx: CGFloat(mark.at.x / perPoint), dy: CGFloat(mark.at.y / perPoint))
        }
        return StormMapRules.keptLabels(rects, ranks: marks.map(\.rank))
    }

    /// Draws a mark; its label only when `labelled`.
    static func draw(_ mark: StormDrawing.Mark, labelled: Bool = true, in context: CGContext) {
        let colour = UIColor(mark.colour)
        let rect = labelRect(mark)
        switch mark.kind {
        case .low(let storm, _, _):
            dot(radius: storm ? 7 : 5, colour: colour, edge: .white, in: context)
            if labelled {
                plate(
                    mark.text, colour: colour, font: labelFont(12), origin: rect.origin, in: context)
            }
        case .day:
            dot(radius: 4, colour: colour, edge: .white, in: context)
            if labelled {
                plate(
                    mark.text, colour: colour, font: labelFont(11), origin: rect.origin, in: context)
            }
        case .hurricane:
            let side = CGFloat(StormMapRules.symbolSize)
            let ring = CGRect(x: -side / 2, y: -side / 2, width: side, height: side)
            context.setFillColor(UIColor.white.withAlphaComponent(0.95).cgColor)
            context.fillEllipse(in: ring)
            context.setStrokeColor(colour.cgColor)
            context.setLineWidth(2.5)
            context.strokeEllipse(in: ring)
            let configuration = UIImage.SymbolConfiguration(pointSize: side * 0.55, weight: .bold)
            if let symbol = UIImage(systemName: "hurricane", withConfiguration: configuration)?
                .withTintColor(colour, renderingMode: .alwaysOriginal)
            {
                let size = symbol.size
                symbol.draw(
                    in: CGRect(
                        x: -size.width / 2, y: -size.height / 2, width: size.width,
                        height: size.height))
            }
            if labelled {
                plate(
                    mark.text, colour: colour, font: labelFont(13), origin: rect.origin, in: context)
            }
        }
    }
}

extension UIColor {
    /// A colour of the storms drawing.
    nonisolated convenience init(_ c: StormDrawing.Colour) {
        self.init(
            red: CGFloat(c.red) / 255, green: CGFloat(c.green) / 255, blue: CGFloat(c.blue) / 255,
            alpha: 1)
    }
}
