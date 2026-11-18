import MapKit
import UIKit

/// What a records overlay holds, swapped whole under a lock: MapKit draws on several threads.
nonisolated private final class Held<Value>: @unchecked Sendable {
    private let lock = NSLock()
    private var value: Value

    init(_ value: Value) { self.value = value }

    func get() -> Value { lock.withLock { value } }
    func set(_ new: Value) { lock.withLock { value = new } }
}

/// One of the records module's layers as one overlay over the whole world; its renderer draws
/// what lies in each tile.
nonisolated class RecordsWorldOverlay: NSObject, MKOverlay {
    let coordinate = CLLocationCoordinate2D(latitude: 0, longitude: 0)
    let boundingMapRect = MKMapRect.world
}

// MARK: - Species records

/// The species records: blurred records as circles, the others as marks.
nonisolated final class RecordsOverlay: RecordsWorldOverlay {
    private struct Content {
        var areas: [RecordArea] = []
        var marks: [RecordMark] = []
        var picked: Set<String> = []
    }

    private let content = Held(Content())

    /// Replaces what is drawn; `picked` are the ids of the areas picked out.
    func set(areas: [RecordArea], marks: [RecordMark], picked: Set<String>) {
        content.set(Content(areas: areas, marks: marks, picked: picked))
    }

    /// The areas that reach into `rect`, largest first, with whether each is picked out.
    func areas(meeting rect: MKMapRect) -> [(area: RecordArea, picked: Bool)] {
        let held = content.get()
        return held.areas.filter {
            rect.intersects(
                MKMapRect(
                    x: $0.at.x - $0.radius, y: $0.at.y - $0.radius, width: 2 * $0.radius,
                    height: 2 * $0.radius))
        }
        .map { ($0, held.picked.contains($0.id)) }
    }

    /// The marks whose centre lies in `rect`.
    func marks(in rect: MKMapRect) -> [RecordMark] {
        content.get().marks.filter { rect.contains(MKMapPoint(x: $0.at.x, y: $0.at.y)) }
    }
}

/// Draws a ``RecordsOverlay``: each blurred record as a tinted circle of its stated radius with
/// a dashed edge, then each record as a disc with a fish, and each USGS Nonindigenous Aquatic
/// Species record as an orange diamond with an exclamation mark, a group with its number.
nonisolated final class RecordsRenderer: MKOverlayRenderer {
    private let records: RecordsOverlay

    init(_ overlay: RecordsOverlay) {
        records = overlay
        super.init(overlay: overlay)
    }

    override func draw(_ mapRect: MKMapRect, zoomScale: MKZoomScale, in context: CGContext) {
        let perPoint = 1 / Double(zoomScale)
        for (area, picked) in records.areas(meeting: mapRect) {
            let centre = point(for: MKMapPoint(x: area.at.x, y: area.at.y))
            let r = CGFloat(area.radius)
            let circle = CGRect(x: centre.x - r, y: centre.y - r, width: 2 * r, height: 2 * r)
            let colour = area.invasive ? RecordDrawing.invasive : RecordDrawing.record
            context.setFillColor(colour.withAlphaComponent(picked ? 0.28 : 0.14).cgColor)
            context.fillEllipse(in: circle)
            context.saveGState()
            let width = (picked ? 3.5 : 2) * perPoint
            context.setLineWidth(CGFloat(width))
            context.setLineDash(phase: 0, lengths: [CGFloat(6 * perPoint), CGFloat(4 * perPoint)])
            context.setStrokeColor(colour.cgColor)
            context.strokeEllipse(in: circle)
            context.restoreGState()
        }
        let margin = RecordDrawing.margin * perPoint
        let marks = records.marks(in: mapRect.insetBy(dx: -margin, dy: -margin))
        guard !marks.isEmpty else { return }
        UIGraphicsPushContext(context)
        defer { UIGraphicsPopContext() }
        for mark in marks {
            let at = point(for: MKMapPoint(x: mark.at.x, y: mark.at.y))
            context.saveGState()
            // From here on, one unit is one point on screen, centred on the mark.
            context.translateBy(x: at.x, y: at.y)
            context.scaleBy(x: CGFloat(perPoint), y: CGFloat(perPoint))
            RecordDrawing.mark(mark, in: context)
            context.restoreGState()
        }
    }
}

// MARK: - Forecast beaches

/// The beaches of the Gulf respiratory irritation forecast.
nonisolated final class BeachesOverlay: RecordsWorldOverlay {
    private let marks = Held<[RecordMark]>([])

    func set(_ new: [RecordMark]) { marks.set(new) }

    /// The marks whose centre lies in `rect`.
    func marks(in rect: MKMapRect) -> [RecordMark] {
        marks.get().filter { rect.contains(MKMapPoint(x: $0.at.x, y: $0.at.y)) }
    }
}

/// Draws a ``BeachesOverlay``: each beach as a rounded square in its level's colour, a group in
/// the colour of its highest level, with its number.
nonisolated final class BeachesRenderer: MKOverlayRenderer {
    private let beaches: BeachesOverlay

    init(_ overlay: BeachesOverlay) {
        beaches = overlay
        super.init(overlay: overlay)
    }

    override func draw(_ mapRect: MKMapRect, zoomScale: MKZoomScale, in context: CGContext) {
        let perPoint = 1 / Double(zoomScale)
        let margin = RecordDrawing.margin * perPoint
        let marks = beaches.marks(in: mapRect.insetBy(dx: -margin, dy: -margin))
        guard !marks.isEmpty else { return }
        UIGraphicsPushContext(context)
        defer { UIGraphicsPopContext() }
        for mark in marks {
            let at = point(for: MKMapPoint(x: mark.at.x, y: mark.at.y))
            context.saveGState()
            context.translateBy(x: at.x, y: at.y)
            context.scaleBy(x: CGFloat(perPoint), y: CGFloat(perPoint))
            RecordDrawing.mark(mark, in: context)
            context.restoreGState()
        }
    }
}

// MARK: - Drawing

/// The drawing of the records' and beaches' marks, centred on the origin, in points. Each kind
/// has its own shape as well as its own colour, so they tell apart without colour.
nonisolated enum RecordDrawing {
    /// The marks' size on screen, in points.
    static let size = 24.0
    /// The largest mark's half size with its number, in points.
    static let margin = 32.0

    /// Records: a deep teal, apart from the catches' raspberry and the stations' colours; the
    /// white fish on it has a contrast above 5:1.
    static let record = colour(0x00695C)
    /// Nonindigenous records: a burnt orange, with a dark exclamation mark on it.
    static let invasive = colour(0xE86A10)
    /// The number's capsule.
    static let ink = colour(0x1B1B1F)

    /// An opaque colour of `rgb` (0xRRGGBB).
    static func colour(_ rgb: UInt32) -> UIColor {
        UIColor(
            red: CGFloat((rgb >> 16) & 0xFF) / 255, green: CGFloat((rgb >> 8) & 0xFF) / 255,
            blue: CGFloat(rgb & 0xFF) / 255, alpha: 1)
    }

    /// A forecast level's colour, light to dark as the level rises (`unknown` grey), and
    /// whether its glyph is drawn dark for contrast.
    static func beach(_ level: String?) -> (fill: UIColor, darkGlyph: Bool) {
        switch level {
        case "very_low": (colour(0xFFF2AE), true)
        case "low": (colour(0xFDB863), true)
        case "moderate": (colour(0xD95F0E), false)
        case "high": (colour(0x991F3C), false)
        default: (colour(0x75757A), false)
        }
    }

    static func mark(_ mark: RecordMark, in context: CGContext) {
        let side = size
        let box = CGRect(x: -side / 2, y: -side / 2, width: side, height: side)
        let path: CGPath
        let fill: UIColor
        let glyph: UIImage?
        switch mark.kind {
        case .record:
            path = CGPath(ellipseIn: box, transform: nil)
            fill = record
            glyph = symbol("fish.fill", dark: false)
        case .invasive:
            let half = side / 2 + 2
            let diamond = CGMutablePath()
            diamond.move(to: CGPoint(x: 0, y: -half))
            diamond.addLine(to: CGPoint(x: half, y: 0))
            diamond.addLine(to: CGPoint(x: 0, y: half))
            diamond.addLine(to: CGPoint(x: -half, y: 0))
            diamond.closeSubpath()
            path = diamond
            fill = invasive
            glyph = symbol("exclamationmark", dark: true)
        case .beach:
            path = CGPath(roundedRect: box, cornerWidth: 6, cornerHeight: 6, transform: nil)
            let look = beach(mark.level)
            fill = look.fill
            glyph = symbol("lungs.fill", dark: look.darkGlyph)
        }
        context.addPath(path)
        context.setStrokeColor(UIColor.black.withAlphaComponent(0.55).cgColor)
        context.setLineWidth(4.5)
        context.setLineJoin(.round)
        context.strokePath()
        context.addPath(path)
        context.setFillColor(fill.cgColor)
        context.fillPath()
        context.addPath(path)
        context.setStrokeColor(UIColor.white.cgColor)
        context.setLineWidth(2)
        context.strokePath()
        if let glyph {
            let size = glyph.size
            let frame = CGRect(
                x: -size.width / 2, y: -size.height / 2, width: size.width, height: size.height)
            glyph.draw(in: frame)
        }
        if mark.items.count > 1 { number(mark.items.count, in: context) }
    }

    /// The number of items in a group, on an ink capsule at the mark's top right.
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

    private static let glyphs = Held<[String: UIImage]>([:])

    /// A symbol in white or near-black, made once.
    private static func symbol(_ name: String, dark: Bool) -> UIImage? {
        let key = "\(name)|\(dark)"
        if let held = glyphs.get()[key] { return held }
        let configuration = UIImage.SymbolConfiguration(pointSize: 11, weight: .bold)
        guard
            let image = UIImage(systemName: name, withConfiguration: configuration)?
                .withTintColor(dark ? ink : .white, renderingMode: .alwaysOriginal)
        else { return nil }
        var all = glyphs.get()
        all[key] = image
        glyphs.set(all)
        return image
    }

    /// A small picture of a mark, for the key in the drawer and the popover's rows.
    @MainActor
    static func image(_ kind: RecordItem.Kind, level: String? = nil) -> UIImage {
        let side = margin
        let renderer = UIGraphicsImageRenderer(size: CGSize(width: side, height: side))
        return renderer.image { drawing in
            let context = drawing.cgContext
            context.translateBy(x: side / 2, y: side / 2)
            let item = RecordItem(id: "", kind: kind, latitude: 0, longitude: 0, level: level)
            mark(RecordMark(items: [item]), in: context)
        }
    }
}
