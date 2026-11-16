import SwiftUI
import UnderbiteKit

/// The day timeline: one row per lane on one shared time axis, scrolled together. Hours are a
/// fixed width, so a wider screen shows more of them. Tap an hour to read its values; each lane's
/// credits and notices sit underneath, whole.
struct DayTimelineView: View {
    let timeline: ViewModel.Timeline
    /// The ids of the cards the page shows: a lane follows its card.
    let shownCards: Set<String>

    @State private var selected: Int64?
    @State private var position = ScrollPosition()

    private let axisHeight: CGFloat = 20
    private let rowSpacing: CGFloat = 6

    private var lanes: [ViewModel.TimelineLane] {
        TimelineLayout.visibleLanes(timeline, shownCards: shownCards)
    }

    private var stripWidth: CGFloat { CGFloat(TimelineLayout.width(timeline)) }

    private func rowHeight(_ lane: ViewModel.TimelineLane) -> CGFloat {
        lane.chart.series.isEmpty ? 48 : 88
    }

    private var stripHeight: CGFloat {
        axisHeight + lanes.reduce(0) { $0 + rowSpacing + rowHeight($1) }
    }

    var body: some View {
        if !lanes.isEmpty {
            VStack(alignment: .leading, spacing: 10) {
                HStack(alignment: .top, spacing: 8) {
                    labels
                    strip
                }
                if let selected { readings(at: selected) }
                ForEach(lanes, id: \.kind) { lane in
                    LaneFootnote(lane: lane)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(14)
            .background(
                Color(.secondarySystemGroupedBackground),
                in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        }
    }

    // MARK: Row labels

    private var labels: some View {
        VStack(alignment: .leading, spacing: rowSpacing) {
            Color.clear.frame(width: 1, height: axisHeight)
            ForEach(lanes, id: \.kind) { lane in
                Text(lane.label)
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .frame(height: rowHeight(lane), alignment: .center)
            }
        }
    }

    // MARK: The strip

    private var strip: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            ZStack(alignment: .topLeading) {
                VStack(alignment: .leading, spacing: rowSpacing) {
                    TimelineAxis(timeline: timeline).frame(height: axisHeight)
                    ForEach(lanes, id: \.kind) { lane in
                        LaneRow(timeline: timeline, lane: lane)
                            .frame(width: stripWidth, height: rowHeight(lane))
                            .accessibilityElement()
                            .accessibilityLabel(Text(lane.chart.accessibility))
                    }
                }
                if let selected {
                    let x = TimelineLayout.x(selected, start: timeline.startT)
                    Rectangle()
                        .fill(Color.accentColor.opacity(0.14))
                        .frame(width: CGFloat(TimelineLayout.hourWidth), height: stripHeight)
                        .offset(x: CGFloat(x))
                        .allowsHitTesting(false)
                }
                if let now = TimelineLayout.now(timeline) {
                    Rectangle()
                        .fill(Color.red)
                        .frame(width: 1.5, height: stripHeight)
                        .offset(x: CGFloat(TimelineLayout.x(now, start: timeline.startT)))
                        .allowsHitTesting(false)
                        .accessibilityHidden(true)
                }
            }
            .frame(width: stripWidth, height: stripHeight, alignment: .topLeading)
            .contentShape(Rectangle())
            .onTapGesture { location in
                let hour = TimelineLayout.hour(atX: Double(location.x), timeline: timeline)
                selected = hour == selected ? nil : hour
            }
        }
        .scrollPosition($position)
        .task(id: TimelineLayout.now(timeline)) {
            // Open a little before now, so the coming hours show; again when now moves.
            position.scrollTo(x: CGFloat(anchorX))
        }
        .frame(height: stripHeight)
    }

    private var anchorX: Double {
        guard let now = TimelineLayout.now(timeline) else { return 0 }
        return max(0, TimelineLayout.x(now, start: timeline.startT) - 60)
    }

    // MARK: The selected hour

    private func readings(at hour: Int64) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            ForEach(lanes, id: \.kind) { lane in
                let lines = TimelineLayout.readings(
                    of: lane, hour: hour, stepMinutes: timeline.stepMinutes)
                if !lines.isEmpty {
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        Text(lane.label).font(.caption.weight(.semibold))
                        Text(lines.joined(separator: " · ")).font(.caption)
                    }
                    .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
    }
}

/// One lane's sources and notices, under the strip: the timeline stands apart from the cards.
private struct LaneFootnote: View {
    let lane: ViewModel.TimelineLane

    var body: some View {
        if !lane.credits.isEmpty || !lane.notices.isEmpty {
            VStack(alignment: .leading, spacing: 2) {
                Text(lane.label).font(.caption2.weight(.semibold))
                NoticeList(notices: lane.notices)
                CreditsLine(credits: lane.credits)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

// MARK: - Drawing

/// The time ticks along the top, labelled by the view model, and the hour grid.
private struct TimelineAxis: View {
    let timeline: ViewModel.Timeline

    var body: some View {
        Canvas { context, size in
            var lastEnd: CGFloat = -.infinity
            for tick in timeline.xTicks {
                let x = CGFloat(TimelineLayout.x(tick.t, start: timeline.startT))
                var line = Path()
                line.move(to: CGPoint(x: x, y: size.height - 5))
                line.addLine(to: CGPoint(x: x, y: size.height))
                context.stroke(line, with: .color(.secondary), lineWidth: 1)
                let text = context.resolve(
                    Text(tick.label).font(.caption2.weight(tick.major ? .semibold : .regular))
                        .foregroundStyle(tick.major ? Color.primary : Color.secondary))
                let width = text.measure(in: size).width
                guard x + 3 >= lastEnd + 4, x + 3 + width <= size.width + 40 else { continue }
                context.draw(text, at: CGPoint(x: x + 3, y: size.height / 2 - 1), anchor: .leading)
                lastEnd = x + 3 + width
            }
        }
        .accessibilityHidden(true)
    }
}

/// One lane: bands behind, the series, then the markers with their labels.
private struct LaneRow: View {
    let timeline: ViewModel.Timeline
    let lane: ViewModel.TimelineLane

    private let topInset: CGFloat = 16
    private let bottomInset: CGFloat = 15

    var body: some View {
        Canvas { context, size in
            let chart = lane.chart
            let hasCurve = !chart.series.isEmpty
            let plotTop = hasCurve ? topInset : 0
            let plotBottom = size.height - (hasCurve ? bottomInset : 0)

            // Row background and the hour grid.
            context.fill(
                Path(CGRect(origin: .zero, size: size)), with: .color(Color(.systemFill).opacity(0.35)))
            var grid = Path()
            for hour in TimelineLayout.hourStarts(timeline) {
                let x = CGFloat(TimelineLayout.x(hour, start: timeline.startT))
                grid.move(to: CGPoint(x: x, y: 0))
                grid.addLine(to: CGPoint(x: x, y: size.height))
            }
            context.stroke(grid, with: .color(.secondary.opacity(0.18)), lineWidth: 0.5)

            // Bands.
            for band in chart.bands {
                let a = CGFloat(TimelineLayout.x(band.startT, start: timeline.startT))
                let b = CGFloat(TimelineLayout.x(band.endT, start: timeline.startT))
                let rect = CGRect(x: a, y: 0, width: max(b - a, 1), height: size.height)
                context.fill(Path(rect), with: .color(Self.color(band.kind)))
                let text = context.resolve(
                    Text(band.label).font(.caption2).foregroundStyle(Color.secondary))
                if text.measure(in: size).width + 6 <= rect.width {
                    context.draw(
                        text, at: CGPoint(x: rect.minX + 4, y: size.height - 8), anchor: .leading)
                }
            }

            // Series.
            let range = TimelineLayout.range(of: lane)
            func y(_ value: Double) -> CGFloat {
                guard let range, range.upperBound > range.lowerBound else {
                    return (plotTop + plotBottom) / 2
                }
                let f = (value - range.lowerBound) / (range.upperBound - range.lowerBound)
                return plotBottom - CGFloat(f) * (plotBottom - plotTop)
            }
            let gap = Int64(timeline.stepMinutes) * 60 * 3
            for (index, series) in chart.series.enumerated() {
                let color: Color = index == 0 ? .accentColor : .orange
                let dashed = series.role == "forecast" || series.role == "modelled"
                let points = series.points.map {
                    CGPoint(x: CGFloat(TimelineLayout.x($0.t, start: timeline.startT)), y: y($0.value))
                }
                switch series.style {
                case "dots":
                    for point in points {
                        context.fill(
                            Path(ellipseIn: CGRect(x: point.x - 2, y: point.y - 2, width: 4, height: 4)),
                            with: .color(color))
                    }
                case "bars":
                    let w = max(CGFloat(TimelineLayout.hourWidth) * CGFloat(timeline.stepMinutes) / 60 - 1, 1)
                    for point in points {
                        context.fill(
                            Path(CGRect(x: point.x, y: point.y, width: w, height: plotBottom - point.y)),
                            with: .color(color.opacity(0.6)))
                    }
                default:
                    var runs: [[CGPoint]] = []
                    var last: Int64?
                    for (point, source) in zip(points, series.points) {
                        if let last, source.t - last <= gap, !runs.isEmpty {
                            runs[runs.count - 1].append(point)
                        } else {
                            runs.append([point])
                        }
                        last = source.t
                    }
                    for run in runs where run.count > 1 {
                        var line = Path()
                        line.addLines(run)
                        if series.style == "area", let first = run.first, let end = run.last {
                            var area = line
                            area.addLine(to: CGPoint(x: end.x, y: plotBottom))
                            area.addLine(to: CGPoint(x: first.x, y: plotBottom))
                            area.closeSubpath()
                            context.fill(area, with: .color(color.opacity(0.25)))
                        }
                        context.stroke(
                            line, with: .color(color),
                            style: StrokeStyle(
                                lineWidth: 2, lineCap: .round, lineJoin: .round,
                                dash: dashed ? [5, 3] : []))
                    }
                }
            }

            // Markers: a dot on the curve where they have a value, a rule where they do not.
            var lastEnd: CGFloat = -.infinity
            for marker in chart.markers.sorted(by: { $0.t < $1.t }) {
                let x = CGFloat(TimelineLayout.x(marker.t, start: timeline.startT))
                if let value = marker.value {
                    let point = CGPoint(x: x, y: y(value))
                    context.fill(
                        Path(ellipseIn: CGRect(x: point.x - 3.5, y: point.y - 3.5, width: 7, height: 7)),
                        with: .color(.accentColor))
                } else {
                    var rule = Path()
                    rule.move(to: CGPoint(x: x, y: 0))
                    rule.addLine(to: CGPoint(x: x, y: size.height))
                    context.stroke(
                        rule, with: .color(Self.markerColor(marker.kind)), lineWidth: 1.5)
                }
                let text = context.resolve(
                    Text(marker.label).font(.caption2.weight(.medium)).foregroundStyle(Color.primary))
                let width = text.measure(in: size).width
                var left = x + 4
                if left + width > size.width { left = x - 4 - width }
                guard left >= lastEnd + 4, left >= 0 else { continue }
                context.draw(text, at: CGPoint(x: left, y: 8), anchor: .leading)
                lastEnd = left + width
            }
        }
    }

    static func color(_ kind: String) -> Color {
        switch kind {
        case "night": .indigo.opacity(0.22)
        case "slack": .gray.opacity(0.3)
        case "solunar_major": .green.opacity(0.35)
        case "solunar_minor": .green.opacity(0.18)
        default: .yellow.opacity(0.3)
        }
    }

    static func markerColor(_ kind: String) -> Color {
        switch kind {
        case "sunrise", "sunset": .orange
        default: .secondary
        }
    }
}
