import SwiftUI
import UnderbiteKit

/// The day timeline: one row per lane on one shared time axis, scrolled together. Hours are a
/// fixed width, so a wider screen shows more of them. Tap an hour to read its values; each lane's
/// credits and notices sit underneath, whole. The words drawn in the strip stop growing at the
/// largest standard text size (a lane's values are read aloud and are on the card pages); the
/// lane names, the readings and the credits follow every size.
struct DayTimelineView: View {
    let timeline: ViewModel.Timeline
    /// The ids of the cards the page shows: a lane follows its card.
    let shownCards: Set<String>

    @State private var selected: Int64?
    @State private var position = ScrollPosition()
    /// The part of the strip on screen, so labels are placed where they can be read whole.
    @State private var visible: ClosedRange<CGFloat> = 0...0
    @Environment(\.sunMode) private var sunMode
    @Environment(\.dynamicTypeSize) private var typeSize

    /// At accessibility sizes a lane's name sits above the lane, so the plot has the card's width.
    private var namesAbove: Bool { typeSize.isAccessibilitySize }

    private let axisHeight: CGFloat = 20
    private let rowSpacing: CGFloat = 6

    private var lanes: [ViewModel.TimelineLane] {
        TimelineLayout.visibleLanes(timeline, shownCards: shownCards)
    }

    private var stripWidth: CGFloat { CGFloat(TimelineLayout.width(timeline)) }

    /// The height of a lane's name, which follows every text size.
    @ScaledMetric(relativeTo: .caption) private var labelHeight: CGFloat = 16

    /// The room for a name above its lane (none beside it).
    private var nameGap: CGFloat { namesAbove ? labelHeight + 4 : 0 }

    private func rowHeight(_ lane: ViewModel.TimelineLane) -> CGFloat {
        max(lane.chart.series.isEmpty ? 48 : 88, labelHeight + 12)
    }

    private var stripHeight: CGFloat {
        axisHeight + lanes.reduce(0) { $0 + rowSpacing + nameGap + rowHeight($1) }
    }

    var body: some View {
        if !lanes.isEmpty {
            VStack(alignment: .leading, spacing: 10) {
                if namesAbove {
                    ZStack(alignment: .topLeading) {
                        strip.dynamicTypeSize(...DynamicTypeSize.xxxLarge)
                        labelsAbove.allowsHitTesting(false)
                    }
                } else {
                    HStack(alignment: .top, spacing: 8) {
                        labels
                        strip.dynamicTypeSize(...DynamicTypeSize.xxxLarge)
                    }
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
            .overlay {
                if sunMode {
                    RoundedRectangle(cornerRadius: 16, style: .continuous)
                        .strokeBorder(Palette.cardStroke(sunMode), lineWidth: 2)
                }
            }
        }
    }

    // MARK: Row labels

    private var labels: some View {
        VStack(alignment: .leading, spacing: rowSpacing) {
            Color.clear.frame(width: 1, height: axisHeight)
            ForEach(lanes, id: \.kind) { lane in
                Text(lane.label)
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.readableSecondary)
                    .fixedSize()
                    .frame(height: rowHeight(lane), alignment: .center)
            }
        }
    }

    /// The names over the strip, each in the gap kept above its lane, full width.
    private var labelsAbove: some View {
        VStack(alignment: .leading, spacing: rowSpacing) {
            Color.clear.frame(width: 1, height: axisHeight)
            ForEach(lanes, id: \.kind) { lane in
                VStack(alignment: .leading, spacing: 0) {
                    Text(lane.label)
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.readableSecondary)
                        .lineLimit(1)
                        .minimumScaleFactor(0.5)
                        // On a backing, so the line for now does not strike through the name.
                        .padding(.trailing, 4)
                        .background(Color(.secondarySystemGroupedBackground))
                        .frame(height: nameGap, alignment: .leading)
                    Color.clear.frame(width: 1, height: rowHeight(lane))
                }
            }
        }
        .accessibilityHidden(true)
    }

    // MARK: The strip

    private var strip: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            ZStack(alignment: .topLeading) {
                if let now = TimelineLayout.now(timeline) {
                    // Below the hour labels, so it never covers one, and under the lanes,
                    // whose shading lets it through and whose event labels sit on a backing.
                    Rectangle()
                        .fill(Color.red)
                        .frame(
                            width: 1.5 * Palette.strokeScale(sunMode),
                            height: stripHeight - axisHeight
                        )
                        .offset(
                            x: CGFloat(TimelineLayout.x(now, start: timeline.startT)),
                            y: axisHeight
                        )
                        .allowsHitTesting(false)
                        .accessibilityHidden(true)
                }
                VStack(alignment: .leading, spacing: rowSpacing) {
                    TimelineAxis(timeline: timeline, visible: visible).frame(height: axisHeight)
                    ForEach(lanes, id: \.kind) { lane in
                        VStack(alignment: .leading, spacing: 0) {
                            Color.clear.frame(width: 1, height: nameGap)
                            LaneRow(timeline: timeline, lane: lane, sunMode: sunMode)
                            .overlay {
                                LaneLabels(timeline: timeline, lane: lane, visible: visible)
                            }
                            .frame(width: stripWidth, height: rowHeight(lane))
                            .accessibilityElement()
                            .accessibilityLabel(Text(lane.chart.accessibility))
                            .accessibilityChartDescriptor(
                                ChartAudioGraph(
                                    chart: lane.chart,
                                    yDomain: TimelineLayout.range(of: lane) ?? 0...1))
                        }
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
            }
            .frame(width: stripWidth, height: stripHeight, alignment: .topLeading)
            .contentShape(Rectangle())
            .onTapGesture { location in
                let hour = TimelineLayout.hour(atX: Double(location.x), timeline: timeline)
                selected = hour == selected ? nil : hour
            }
        }
        .scrollPosition($position)
        .onScrollGeometryChange(for: ClosedRange<CGFloat>.self) { geometry in
            let rect = geometry.visibleRect
            return rect.minX.rounded(.down)...max(rect.minX, rect.maxX).rounded(.up)
        } action: { _, now in
            visible = now
        }
        .task(id: TimelineLayout.now(timeline)) {
            // Open a little before now, so the coming hours show; again when now moves.
            position.scrollTo(x: CGFloat(anchorX))
        }
        .frame(height: stripHeight)
    }

    /// The hour line a little before now, so the first hour label shows whole.
    private var anchorX: Double {
        guard let now = TimelineLayout.now(timeline) else { return 0 }
        let x = TimelineLayout.x(now, start: timeline.startT) - 60
        return max(0, (x / TimelineLayout.hourWidth).rounded(.down) * TimelineLayout.hourWidth)
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
                Text(lane.label).font(.caption.weight(.semibold))
                NoticeList(notices: lane.notices)
                CreditsLine(credits: lane.credits)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

// MARK: - Drawing

/// The time ticks along the top, labelled by the view model, and the hour grid.
/// A label that either edge of the part on screen would cut is left out until it is whole.
private struct TimelineAxis: View {
    let timeline: ViewModel.Timeline
    let visible: ClosedRange<CGFloat>

    var body: some View {
        Canvas { context, size in
            var lastEnd: CGFloat = -.infinity
            let shown = visible.upperBound > visible.lowerBound ? visible : 0...size.width
            for tick in timeline.xTicks {
                let x = CGFloat(TimelineLayout.x(tick.t, start: timeline.startT))
                var line = Path()
                line.move(to: CGPoint(x: x, y: size.height - 5))
                line.addLine(to: CGPoint(x: x, y: size.height))
                context.stroke(line, with: .color(.secondary), lineWidth: 1)
                let text = context.resolve(
                    Text(tick.label).font(.caption2.weight(tick.major ? .semibold : .regular))
                        .foregroundStyle(Color.primary))
                let width = text.measure(in: size).width
                let left = x + 3
                guard left >= lastEnd + 4, left >= shown.lowerBound,
                    left + width <= shown.upperBound
                else { continue }
                context.draw(text, at: CGPoint(x: left, y: size.height / 2 - 1), anchor: .leading)
                lastEnd = left + width
            }
        }
        .accessibilityElement()
        .accessibilityLabel(Text(Self.spoken(timeline)))
    }

    /// The axis read aloud: its major ticks, in the view's words.
    static func spoken(_ timeline: ViewModel.Timeline) -> String {
        timeline.xTicks.filter(\.major).map(\.label).joined(separator: ", ")
    }
}

/// One lane: bands behind, the series, then the markers. Nothing here depends on where the strip
/// is scrolled, so scrolling does not draw it again; the words are `LaneLabels`'.
private struct LaneRow: View {
    let timeline: ViewModel.Timeline
    let lane: ViewModel.TimelineLane
    let sunMode: Bool

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
                context.fill(Path(rect), with: .color(Self.color(band.kind, sunMode: sunMode)))
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
            let stroke = Palette.strokeScale(sunMode)
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
                            Path(
                                ellipseIn: CGRect(
                                    x: point.x - 2 * stroke, y: point.y - 2 * stroke,
                                    width: 4 * stroke, height: 4 * stroke)),
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
                                lineWidth: 2 * stroke, lineCap: .round, lineJoin: .round,
                                dash: dashed ? [5, 3] : []))
                    }
                }
            }

            // Markers: a dot on the curve where they have a value, a rule where they do not;
            // their words are drawn over this by `LaneLabels`.
            for marker in chart.markers {
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
                        rule, with: .color(Self.markerColor(marker.kind)),
                        lineWidth: 1.5 * stroke)
                }
            }
        }
    }

    static func color(_ kind: String, sunMode: Bool) -> Color {
        let strength = Palette.fillScale(sunMode)
        return switch kind {
        case "night": .indigo.opacity(0.22 * strength)
        case "slack": .gray.opacity(0.3 * strength)
        case "solunar_major": .green.opacity(0.35 * strength)
        case "solunar_minor": .green.opacity(0.18 * strength)
        default: .yellow.opacity(0.3 * strength)
        }
    }

    static func markerColor(_ kind: String) -> Color {
        switch kind {
        case "sunrise", "sunset": .orange
        default: .secondary
        }
    }
}

/// A lane's words, which follow the part of the strip on screen: its bands' ("Night", "Major")
/// start at the band's visible start and are left out while they do not fit whole inside it, and
/// its markers' ("High 4.3 ft", "Sunset 4:37 PM") go right of their marker, or left of it where
/// the right would run past the part on screen, and are left out while neither fits whole.
private struct LaneLabels: View {
    let timeline: ViewModel.Timeline
    let lane: ViewModel.TimelineLane
    let visible: ClosedRange<CGFloat>

    var body: some View {
        Canvas { context, size in
            let shown = visible.upperBound > visible.lowerBound ? visible : 0...size.width
            let low = max(0, shown.lowerBound)
            let high = min(size.width, shown.upperBound)
            for band in lane.chart.bands {
                let a = CGFloat(TimelineLayout.x(band.startT, start: timeline.startT))
                let b = CGFloat(TimelineLayout.x(band.endT, start: timeline.startT))
                let rect = CGRect(x: a, y: 0, width: max(b - a, 1), height: size.height)
                let text = context.resolve(
                    Text(band.label).font(.caption2).foregroundStyle(Color.primary))
                // Start at the bar's visible start, never left of the part on screen.
                let left = max(rect.minX, low) + 4
                let right = min(rect.maxX, high)
                if left + text.measure(in: size).width + 2 <= right {
                    context.draw(text, at: CGPoint(x: left, y: size.height - 8), anchor: .leading)
                }
            }
            var lastEnd: CGFloat = -.infinity
            for marker in lane.chart.markers.sorted(by: { $0.t < $1.t }) {
                let x = CGFloat(TimelineLayout.x(marker.t, start: timeline.startT))
                let text = context.resolve(
                    Text(marker.label).font(.caption2.weight(.medium))
                        .foregroundStyle(Color.primary))
                let measured = text.measure(in: size)
                let width = measured.width
                var left = x + 4
                if left + width > high { left = x - 4 - width }
                guard left >= lastEnd + 4, left >= low, left + width <= high else { continue }
                // The backing is as tall as the words at the text size in use.
                let box = CGRect(x: left - 2, y: 1, width: width + 4, height: measured.height + 2)
                context.fill(
                    Path(roundedRect: box, cornerRadius: 3),
                    with: .color(Color(.secondarySystemGroupedBackground).opacity(0.9)))
                context.draw(text, at: CGPoint(x: left, y: box.midY), anchor: .leading)
                lastEnd = left + width
            }
        }
        .allowsHitTesting(false)
    }
}
