import MapKit
import UIKit

/// The breaks of one layer in view as one overlay with one renderer, however many lines there
/// are. The lines are the core's, placed once (``BreakLines``); new lines come as a new overlay.
nonisolated final class BreakOverlay: NSObject, MKOverlay {
    /// The breaks layer's id (`sst-breaks`).
    let layer: String
    let lines: BreakLines
    let coordinate = CLLocationCoordinate2D(latitude: 0, longitude: 0)
    let boundingMapRect = MKMapRect.world

    init(layer: String, lines: BreakLines) {
        self.layer = layer
        self.lines = lines
    }
}

/// How a breaks layer's lines look: a dark line on a light halo, so that it reads over every
/// colour of the layer under it. Temperature breaks are near black, chlorophyll breaks violet, so
/// that both can be told apart over one map.
nonisolated struct BreakStyle: Equatable, Sendable {
    let red: Double
    let green: Double
    let blue: Double

    static let temperature = BreakStyle(red: 0.05, green: 0.05, blue: 0.08)
    static let chlorophyll = BreakStyle(red: 0.36, green: 0.08, blue: 0.52)

    /// The style of a breaks layer by id.
    static func of(_ layer: String) -> BreakStyle {
        layer.hasPrefix("chlorophyll") ? .chlorophyll : .temperature
    }

    /// The halo's width beyond each side of the line, in points.
    static let halo = 1.5

    var line: UIColor { UIColor(red: red, green: green, blue: blue, alpha: 0.95) }
    var haloColour: UIColor { UIColor(white: 1, alpha: 0.85) }
}

/// Draws a ``BreakOverlay``: each line a fixed width on screen whatever the zoom, heavier the
/// stronger it is (``BreakLines/width(emphasis:)``), on a light halo.
nonisolated final class BreakRenderer: MKOverlayRenderer {
    private let lines: BreakLines
    private let style: BreakStyle

    init(_ overlay: BreakOverlay) {
        lines = overlay.lines
        style = BreakStyle.of(overlay.layer)
        super.init(overlay: overlay)
    }

    override func draw(_ mapRect: MKMapRect, zoomScale: MKZoomScale, in context: CGContext) {
        let perPoint = 1 / Double(zoomScale)
        let margin = (BreakLines.heaviest + 2 * BreakStyle.halo) * perPoint
        let visible = lines.lines(in: mapRect, margin: margin)
        guard !visible.isEmpty else { return }
        context.setLineCap(.round)
        context.setLineJoin(.round)
        // The halos first, all of them, so that a crossing line's halo never cuts another line.
        context.setStrokeColor(style.haloColour.cgColor)
        for line in visible {
            let width = BreakLines.width(emphasis: line.emphasis) + 2 * BreakStyle.halo
            stroke(line, width: width * perPoint, in: context)
        }
        context.setStrokeColor(style.line.cgColor)
        for line in visible {
            stroke(line, width: BreakLines.width(emphasis: line.emphasis) * perPoint, in: context)
        }
    }

    private func stroke(_ line: PlacedBreak, width: Double, in context: CGContext) {
        context.setLineWidth(CGFloat(width))
        context.move(to: point(for: line.points[0]))
        for p in line.points.dropFirst() {
            context.addLine(to: point(for: p))
        }
        context.strokePath()
    }
}
