import SwiftUI
import UnderbiteKit
import WidgetKit

/// How a widget is painted: in the theme's colours, or (a tinted or clear Home Screen, the Lock
/// Screen) in the one colour the system gives it, where the theme's colours would vanish.
enum WidgetPaint: Sendable {
    case color, mono

    init(_ mode: WidgetRenderingMode) {
        self = mode == .fullColor ? .color : .mono
    }
}

/// The theme's colour for a role, or the system's single colour (at `strength`) when the
/// widget is tinted.
struct WidgetInk {
    let theme: ColorTheme
    let paint: WidgetPaint

    func callAsFunction(_ role: ColorRole, mono strength: Double = 1) -> Color {
        paint == .color ? theme.color(role) : Color.primary.opacity(strength)
    }
}

/// A line through points in unit space (`y` up), and the area under it for `filled`.
nonisolated struct PlotLine: Shape {
    let points: [PlotPoint]
    var filled = false

    func path(in rect: CGRect) -> Path {
        var path = Path()
        guard let first = points.first else { return path }
        func at(_ p: PlotPoint) -> CGPoint {
            CGPoint(x: rect.minX + p.x * rect.width, y: rect.maxY - p.y * rect.height)
        }
        if filled {
            path.move(to: CGPoint(x: at(first).x, y: rect.maxY))
            path.addLine(to: at(first))
        } else {
            path.move(to: at(first))
        }
        for point in points.dropFirst() { path.addLine(to: at(point)) }
        if filled, let last = points.last {
            path.addLine(to: CGPoint(x: at(last).x, y: rect.maxY))
            path.closeSubpath()
        }
        return path
    }
}

/// The tide over the entry's window: night shaded, the curve and the water under it, now as a
/// line, the highs and lows as dots and (`labels`) the core's words for them and the major hours.
struct TideCurve: View {
    let window: TideWindow
    let paint: WidgetPaint
    var labels = false
    @Environment(\.colorTheme) private var theme

    private var ink: WidgetInk { WidgetInk(theme: theme, paint: paint) }
    private static let tickHeight: CGFloat = 13

    var body: some View {
        GeometryReader { geometry in
            let size = geometry.size
            let plot = CGSize(
                width: size.width, height: max(size.height - (labels ? Self.tickHeight : 0), 1))
            ZStack(alignment: .topLeading) {
                ForEach(Array(window.nights.enumerated()), id: \.offset) { _, night in
                    Rectangle()
                        .fill(ink(.timelineNight, mono: 0.12))
                        .frame(width: max((night.end - night.start) * plot.width, 0), height: plot.height)
                        .offset(x: night.start * plot.width)
                }
                PlotLine(points: window.curve, filled: true)
                    .fill(ink(.tideFill, mono: 0.22))
                    .frame(width: plot.width, height: plot.height)
                PlotLine(points: window.curve)
                    .stroke(ink(.tideCurve), style: StrokeStyle(lineWidth: 2, lineCap: .round, lineJoin: .round))
                    .frame(width: plot.width, height: plot.height)
                    .widgetAccentable()
                if let now = window.now {
                    Rectangle()
                        .fill(ink(.chartNow))
                        .frame(width: 1.5, height: plot.height)
                        .offset(x: now * plot.width - 0.75)
                        .widgetAccentable()
                }
                ForEach(Array(window.marks.enumerated()), id: \.offset) { _, mark in
                    markView(mark, in: plot)
                }
                if labels {
                    ForEach(Array(window.ticks.enumerated()), id: \.offset) { _, tick in
                        Text(tick.label)
                            .font(.system(size: 9))
                            .foregroundStyle(.secondary)
                            .fixedSize()
                            .position(
                                x: clamp(tick.x * plot.width, 14, plot.width - 14),
                                y: plot.height + Self.tickHeight / 2 + 1)
                    }
                }
            }
            .frame(width: size.width, height: size.height, alignment: .topLeading)
        }
        .clipped()
        .accessibilityHidden(true)
    }

    @ViewBuilder private func markView(_ mark: TideMark, in plot: CGSize) -> some View {
        let high = mark.kind == "high"
        let x = mark.at.x * plot.width
        let y = (1 - mark.at.y) * plot.height
        Circle()
            .fill(ink(high ? .tideHigh : .tideLow))
            .frame(width: 5, height: 5)
            .position(x: x, y: y)
            .widgetAccentable()
        if labels {
            Text(mark.label)
                .font(.system(size: 9, weight: .semibold))
                .fixedSize()
                .position(
                    x: clamp(x, 28, plot.width - 28),
                    y: clamp(high ? y - 9 : y + 9, 6, plot.height - 6))
        }
    }

    private func clamp(_ value: CGFloat, _ low: CGFloat, _ high: CGFloat) -> CGFloat {
        guard high > low else { return value }
        return min(max(value, low), high)
    }
}

/// A week of river flow as a line over the water under it.
struct FlowSpark: View {
    let values: [Double]
    let paint: WidgetPaint
    @Environment(\.colorTheme) private var theme

    private var ink: WidgetInk { WidgetInk(theme: theme, paint: paint) }

    var body: some View {
        let points = FlowLine.points(values)
        ZStack {
            PlotLine(points: points, filled: true)
                .fill(ink(.chartObserved, mono: 0.2).opacity(paint == .color ? 0.18 : 1))
            PlotLine(points: points)
                .stroke(ink(.chartObserved), style: StrokeStyle(lineWidth: 2, lineCap: .round, lineJoin: .round))
                .widgetAccentable()
        }
        .padding(.vertical, 2)
        .accessibilityHidden(true)
    }
}
