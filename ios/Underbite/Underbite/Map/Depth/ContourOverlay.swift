import MapKit
import UIKit

/// The depth contours of the view as one overlay with one renderer, however many lines there are
/// (an overlay per line or per segment is too slow on older phones). The lines are the core's,
/// placed once (``ContourSet``); new lines come as a new overlay.
nonisolated final class ContourOverlay: NSObject, MKOverlay {
    let contours: ContourSet
    let coordinate = CLLocationCoordinate2D(latitude: 0, longitude: 0)
    let boundingMapRect = MKMapRect.world

    init(_ contours: ContourSet) {
        self.contours = contours
    }
}

/// Draws a ``ContourOverlay``: each line a fixed width on screen whatever the zoom, major lines
/// heavier, and each label the core placed, turned along its line, on a light halo so that it
/// reads over the shading.
nonisolated final class ContourRenderer: MKOverlayRenderer {
    /// Line widths in points.
    static let minorWidth = 0.8
    static let majorWidth = 1.8
    /// The labels' text size in points.
    static let labelSize = 11.0
    /// How far from its anchor a label's text may reach, in points.
    static let labelReach = 60.0

    private let contours: ContourSet
    private let line = UIColor(red: 0.08, green: 0.13, blue: 0.22, alpha: 0.7).cgColor
    private let majorLine = UIColor(red: 0.05, green: 0.09, blue: 0.16, alpha: 0.9).cgColor
    private let text: [NSAttributedString.Key: Any]
    private let halo: [NSAttributedString.Key: Any]

    init(_ overlay: ContourOverlay) {
        contours = overlay.contours
        let font = UIFont.systemFont(ofSize: Self.labelSize, weight: .semibold)
        text = [.font: font, .foregroundColor: UIColor(white: 0.08, alpha: 1)]
        halo = [
            .font: font, .foregroundColor: UIColor.white,
            .strokeColor: UIColor(white: 1, alpha: 0.9), .strokeWidth: 6.0,
        ]
        super.init(overlay: overlay)
    }

    override func draw(_ mapRect: MKMapRect, zoomScale: MKZoomScale, in context: CGContext) {
        let perPoint = 1 / Double(zoomScale)
        context.setLineCap(.round)
        context.setLineJoin(.round)
        let lines = contours.lines(in: mapRect, margin: Self.majorWidth * perPoint)
        for major in [false, true] {
            context.setLineWidth(CGFloat((major ? Self.majorWidth : Self.minorWidth) * perPoint))
            context.setStrokeColor(major ? majorLine : line)
            for contour in lines where contour.major == major {
                let points = contour.points
                context.move(to: point(for: points[0]))
                for p in points.dropFirst() {
                    context.addLine(to: point(for: p))
                }
            }
            context.strokePath()
        }
        let labels = contours.labels(near: mapRect, reach: Self.labelReach * perPoint)
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
