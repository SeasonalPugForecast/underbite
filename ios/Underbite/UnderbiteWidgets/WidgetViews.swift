import SwiftUI
import UnderbiteKit
import WidgetKit

/// A widget at one moment: its kind's view for the payload's entry, or its placeholder. Every
/// word and number is the payload's; the views only lay them out.
struct WidgetMomentView: View {
    let kind: WidgetKind
    let moment: WidgetMoment
    let family: WidgetFamily
    let paint: WidgetPaint

    var body: some View {
        switch moment.content {
        case .gap(let gap):
            WidgetGapView(kind: kind, name: moment.name, gap: gap, family: family)
        case .entry(let entry, let places):
            entryView(entry, places: places)
        }
    }

    @ViewBuilder private func entryView(
        _ entry: ViewModel.WidgetEntry, places: [ViewModel.WidgetPlace]
    ) -> some View {
        let place = entry.places.first
        let info = places.first
        let name = info?.name ?? moment.name ?? ""
        switch kind {
        case .tide:
            if let tide = place?.tide {
                TideWidgetView(
                    name: name, tide: tide, chart: info?.tideChart, now: entry.t, family: family,
                    paint: paint)
            }
        case .flow:
            if let flow = place?.flow {
                FlowWidgetView(
                    name: name, flow: flow, spark: info?.flowSpark ?? [], family: family, paint: paint)
            }
        case .snack:
            if let snack = place?.snack {
                SnackWidgetView(
                    name: name, snack: snack, place: place, info: info, now: entry.t, family: family,
                    paint: paint)
            }
        case .nextTrip:
            if let trip = entry.trip {
                TripWidgetView(trip: trip, entry: entry, places: places, family: family, paint: paint)
            }
        }
    }
}

extension WidgetFamily {
    /// A Lock Screen family.
    var isAccessory: Bool {
        [.accessoryRectangular, .accessoryCircular, .accessoryInline].contains(self)
    }
}

// MARK: - Parts

/// The place's name with the widget's symbol.
struct WidgetName: View {
    let name: String
    let symbol: String
    var font: Font = .caption.weight(.semibold)

    var body: some View {
        Label {
            Text(name).lineLimit(1).minimumScaleFactor(0.75)
        } icon: {
            Image(systemName: symbol).widgetAccentedRenderingMode(.accented)
        }
        .labelStyle(.titleAndIcon)
        .font(font)
        .imageScale(.small)
    }
}

/// A displayed value, its unit smaller beside it, and an arrow for where it is going.
struct WidgetHeadline: View {
    let quantity: ViewModel.Quantity
    var arrow: String?
    var size: CGFloat = 30
    var dimmed = false

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 4) {
            if !quantity.text.isEmpty {
                Text(quantity.text)
                    .font(.system(size: size, weight: .semibold, design: .rounded))
                    .widgetAccentable()
            }
            if !quantity.unit.isEmpty {
                Text(quantity.unit)
                    .font(.system(size: max(size * 0.42, 11), weight: .medium))
                    .foregroundStyle(.secondary)
            }
            if let arrow {
                Image(systemName: arrow)
                    .font(.system(size: max(size * 0.5, 11), weight: .bold))
                    .widgetAccentable()
            }
        }
        .lineLimit(1)
        .minimumScaleFactor(0.6)
        .opacity(dimmed ? 0.55 : 1)
    }
}

/// Why a value is greyed (too old, too far, unavailable), in the card state's colour.
struct WidgetStateNote: View {
    let state: ViewModel.CardState
    let paint: WidgetPaint
    var font: Font = .caption2
    @Environment(\.colorTheme) private var theme

    var body: some View {
        if let reason = state.reason, !reason.isEmpty {
            Text(reason)
                .font(font)
                .foregroundStyle(WidgetInk(theme: theme, paint: paint)(role, mono: 0.8))
                .lineLimit(2)
                .minimumScaleFactor(0.8)
        }
    }

    private var role: ColorRole {
        switch state {
        case .stale: .cardStale
        case .far: .cardFar
        default: .cardUnavailable
        }
    }
}

extension ViewModel.CardState {
    var isFresh: Bool { if case .fresh = self { true } else { false } }
}

/// A card's short lines, in order.
struct WidgetLines: View {
    let lines: [String]
    var font: Font = .caption
    var limit = 2

    var body: some View {
        ForEach(Array(lines.enumerated()), id: \.offset) { _, line in
            Text(line)
                .font(font)
                .foregroundStyle(.secondary)
                .lineLimit(limit)
                .minimumScaleFactor(0.8)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

/// The sources the widget's values come from and their notices, at its foot: always there,
/// and each notice whole.
struct WidgetFooter: View {
    let credits: WidgetCredits
    var compact = false

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if !credits.credits.isEmpty {
                Text(credits.line)
                    .lineLimit(compact ? 1 : 2)
                    .minimumScaleFactor(0.7)
            }
            ForEach(credits.notices, id: \.self) { notice in
                Text(notice)
                    .lineLimit(2)
                    .minimumScaleFactor(0.75)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .font(.system(size: compact ? 9 : 10))
        .foregroundStyle(.secondary)
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
    }
}

enum WidgetSymbols {
    static func kind(_ kind: WidgetKind) -> String {
        switch kind {
        case .tide: "water.waves"
        case .flow: "drop.fill"
        case .snack: "fish.fill"
        case .nextTrip: "calendar"
        }
    }

    /// The tide's stage: rising, falling, slack.
    static func stage(_ stage: String) -> String? {
        switch stage {
        case "rising": "arrow.up"
        case "falling": "arrow.down"
        case "high_slack", "low_slack": "equal"
        default: nil
        }
    }

    /// A trend's direction.
    static func trend(_ direction: String?) -> String? {
        switch direction {
        case "rising": "arrow.up.right"
        case "falling": "arrow.down.right"
        case "steady": "arrow.right"
        default: nil
        }
    }

    /// A go / no-go verdict.
    static func verdict(_ verdict: String?) -> String {
        switch verdict {
        case "go": "checkmark.circle.fill"
        case "marginal": "exclamationmark.circle.fill"
        case "no_go": "xmark.octagon.fill"
        default: "questionmark.circle"
        }
    }
}

// MARK: - Placeholder

/// A widget without a payload's values: what it is, the spot when it is known, and what to do.
/// Plain words of the widget's own, never a value.
struct WidgetGapView: View {
    let kind: WidgetKind
    let name: String?
    let gap: WidgetGap
    let family: WidgetFamily

    var body: some View {
        if gap == .loading {
            content.redacted(reason: .placeholder)
        } else {
            content
        }
    }

    private var content: some View {
        VStack(alignment: .leading, spacing: family.isAccessory ? 1 : 4) {
            Label(Self.title(kind), systemImage: WidgetSymbols.kind(kind))
                .font(.caption.weight(.semibold))
                .lineLimit(1)
            if let name, !name.isEmpty {
                Text(name)
                    .font(family.isAccessory ? .caption : .headline)
                    .lineLimit(family == .systemSmall ? 2 : 1)
                    .minimumScaleFactor(0.8)
            }
            if !family.isAccessory { Spacer(minLength: 0) }
            Text(message)
                .font(family.isAccessory ? .caption2 : .caption)
                .foregroundStyle(.secondary)
                .lineLimit(3)
                .minimumScaleFactor(0.8)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    static func title(_ kind: WidgetKind) -> String {
        switch kind {
        case .tide: "Tide"
        case .flow: "River flow"
        case .snack: "Snack times"
        case .nextTrip: "Next trip"
        }
    }

    private var message: String {
        switch gap {
        case .loading: "Underbite"
        case .noSpot: "Save a spot in Underbite to see it here."
        case .noPayload: "Open Underbite to load this spot."
        case .noTrip: "No trip ahead. Plan one in Underbite."
        case .outdated: "Open Underbite to update."
        case .noData:
            switch kind {
            case .tide: "No tide at this spot."
            case .flow: "No river gauge at this spot."
            case .snack: "No snack times at this spot yet."
            case .nextTrip: "No go/no-go for this trip yet."
            }
        }
    }
}

// MARK: - Tide

struct TideWidgetView: View {
    let name: String
    let tide: ViewModel.WidgetTide
    let chart: ViewModel.Chart?
    let now: Int64
    let family: WidgetFamily
    let paint: WidgetPaint

    private var card: ViewModel.WidgetCard { tide.card }
    private var window: TideWindow? {
        chart.map { TideWindow(chart: $0, start: tide.startT, end: tide.endT, now: now) }
    }
    private var credits: WidgetCredits {
        var credits = WidgetCredits()
        credits.add(card)
        return credits
    }
    private var headline: WidgetHeadline {
        WidgetHeadline(
            quantity: card.headline, arrow: WidgetSymbols.stage(tide.stage), size: headlineSize,
            dimmed: !card.state.isFresh)
    }
    private var headlineSize: CGFloat {
        switch family {
        case .accessoryRectangular: 17
        case .systemSmall: 28
        case .systemMedium: 30
        default: 36
        }
    }

    var body: some View {
        Group {
            switch family {
            case .accessoryRectangular:
                VStack(alignment: .leading, spacing: 0) {
                    headline
                    WidgetLines(lines: Array(card.lines.prefix(1)), font: .caption2, limit: 1)
                    WidgetFooter(credits: credits, compact: true)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            case .systemSmall:
                VStack(alignment: .leading, spacing: 2) {
                    WidgetName(name: name, symbol: WidgetSymbols.kind(.tide))
                    headline
                    WidgetLines(lines: Array(card.lines.prefix(1)), font: .caption2, limit: 1)
                    WidgetStateNote(state: card.state, paint: paint)
                    curve(labels: false).frame(minHeight: 18)
                    WidgetFooter(credits: credits, compact: true)
                }
            case .systemMedium:
                VStack(alignment: .leading, spacing: 4) {
                    HStack(alignment: .top, spacing: 10) {
                        VStack(alignment: .leading, spacing: 2) {
                            WidgetName(name: name, symbol: WidgetSymbols.kind(.tide))
                            headline
                            WidgetLines(lines: card.lines, font: .caption2, limit: 1)
                            WidgetStateNote(state: card.state, paint: paint)
                        }
                        .frame(width: 128, alignment: .leading)
                        curve(labels: false)
                    }
                    WidgetFooter(credits: credits, compact: true)
                }
            case .systemExtraLarge:
                VStack(alignment: .leading, spacing: 8) {
                    HStack(alignment: .top, spacing: 16) {
                        header.frame(width: 200, alignment: .leading)
                        curve(labels: true)
                    }
                    WidgetFooter(credits: credits)
                }
            default:
                VStack(alignment: .leading, spacing: 6) {
                    header
                    curve(labels: true)
                    WidgetFooter(credits: credits)
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(Text(card.accessibility))
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack {
                WidgetName(name: name, symbol: WidgetSymbols.kind(.tide), font: .subheadline.weight(.semibold))
                Spacer(minLength: 4)
                Text(card.title).font(.caption).foregroundStyle(.secondary)
            }
            headline
            WidgetLines(lines: card.lines, font: .caption)
            WidgetStateNote(state: card.state, paint: paint)
        }
    }

    @ViewBuilder private func curve(labels: Bool) -> some View {
        if let window, window.curve.count >= 2 {
            TideCurve(window: window, paint: paint, labels: labels)
        } else {
            Spacer(minLength: 0)
        }
    }
}

// MARK: - River flow

struct FlowWidgetView: View {
    let name: String
    let flow: ViewModel.WidgetFlow
    let spark: [Double]
    let family: WidgetFamily
    let paint: WidgetPaint

    private var card: ViewModel.WidgetCard { flow.card }
    private var credits: WidgetCredits {
        var credits = WidgetCredits()
        credits.add(card)
        return credits
    }
    private var headline: WidgetHeadline {
        WidgetHeadline(
            quantity: card.headline, arrow: WidgetSymbols.trend(flow.trend?.direction), size: headlineSize,
            dimmed: !card.state.isFresh)
    }
    private var headlineSize: CGFloat {
        switch family {
        case .accessoryRectangular: 17
        case .systemSmall: 28
        case .systemMedium: 30
        default: 36
        }
    }
    /// The trend in words, then against normal.
    private var status: [String] {
        [flow.trend?.text, flow.normal].compactMap { $0 }.filter { !$0.isEmpty }
    }

    var body: some View {
        Group {
            switch family {
            case .accessoryRectangular:
                VStack(alignment: .leading, spacing: 0) {
                    headline
                    WidgetLines(lines: Array(status.prefix(1)), font: .caption2, limit: 1)
                    WidgetFooter(credits: credits, compact: true)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            case .systemSmall:
                VStack(alignment: .leading, spacing: 2) {
                    WidgetName(name: name, symbol: WidgetSymbols.kind(.flow))
                    headline
                    WidgetLines(lines: Array(status.suffix(1)), font: .caption2, limit: 1)
                    WidgetStateNote(state: card.state, paint: paint)
                    line.frame(minHeight: 16)
                    WidgetFooter(credits: credits, compact: true)
                }
            case .systemMedium:
                VStack(alignment: .leading, spacing: 4) {
                    HStack(alignment: .top, spacing: 10) {
                        VStack(alignment: .leading, spacing: 2) {
                            WidgetName(name: name, symbol: WidgetSymbols.kind(.flow))
                            headline
                            WidgetLines(lines: status, font: .caption2, limit: 1)
                            WidgetStateNote(state: card.state, paint: paint)
                        }
                        .frame(width: 150, alignment: .leading)
                        line
                    }
                    WidgetFooter(credits: credits, compact: true)
                }
            case .systemExtraLarge:
                VStack(alignment: .leading, spacing: 8) {
                    HStack(alignment: .top, spacing: 16) {
                        header.frame(width: 240, alignment: .leading)
                        line
                    }
                    WidgetFooter(credits: credits)
                }
            default:
                VStack(alignment: .leading, spacing: 6) {
                    header
                    line
                    WidgetFooter(credits: credits)
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(Text(card.accessibility))
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack {
                WidgetName(name: name, symbol: WidgetSymbols.kind(.flow), font: .subheadline.weight(.semibold))
                Spacer(minLength: 4)
                Text(card.title).font(.caption).foregroundStyle(.secondary)
            }
            headline
            WidgetLines(lines: status, font: .caption, limit: 1)
            WidgetLines(lines: card.lines, font: .caption)
            WidgetStateNote(state: card.state, paint: paint)
        }
    }

    @ViewBuilder private var line: some View {
        if spark.count >= 2 {
            FlowSpark(values: spark, paint: paint)
        } else {
            Spacer(minLength: 0)
        }
    }
}

// MARK: - Snack times

struct SnackWidgetView: View {
    let name: String
    let snack: ViewModel.WidgetCard
    /// The place at this entry and what its entries share, for the tide or the flow beside the
    /// largest sizes.
    let place: ViewModel.WidgetPlaceEntry?
    let info: ViewModel.WidgetPlace?
    let now: Int64
    let family: WidgetFamily
    let paint: WidgetPaint

    /// The tide's window to draw beside the snack time on the largest sizes.
    private var tideWindow: TideWindow? {
        guard family == .systemLarge || family == .systemExtraLarge, let tide = place?.tide,
            let chart = info?.tideChart
        else { return nil }
        let window = TideWindow(chart: chart, start: tide.startT, end: tide.endT, now: now)
        return window.curve.count >= 2 ? window : nil
    }
    /// The week of flow to draw instead, where there is no tide.
    private var flowSpark: [Double]? {
        guard family == .systemLarge || family == .systemExtraLarge, tideWindow == nil,
            place?.flow != nil, let spark = info?.flowSpark, spark.count >= 2
        else { return nil }
        return spark
    }
    private var credits: WidgetCredits {
        var credits = WidgetCredits()
        credits.add(snack)
        if tideWindow != nil, let tide = place?.tide { credits.add(tide.card) }
        if flowSpark != nil, let flow = place?.flow { credits.add(flow.card) }
        return credits
    }
    private var headlineSize: CGFloat {
        switch family {
        case .systemSmall: 28
        case .systemMedium: 30
        default: 36
        }
    }

    var body: some View {
        Group {
            switch family {
            case .systemSmall:
                VStack(alignment: .leading, spacing: 2) {
                    WidgetName(name: name, symbol: WidgetSymbols.kind(.snack))
                    headline
                    WidgetLines(lines: Array(snack.lines.prefix(1)), font: .caption, limit: 3)
                    WidgetStateNote(state: snack.state, paint: paint)
                    Spacer(minLength: 0)
                    WidgetFooter(credits: credits, compact: true)
                }
            case .systemMedium:
                VStack(alignment: .leading, spacing: 3) {
                    HStack {
                        WidgetName(name: name, symbol: WidgetSymbols.kind(.snack))
                        Spacer(minLength: 4)
                        Text(snack.title).font(.caption).foregroundStyle(.secondary)
                    }
                    headline
                    WidgetLines(lines: snack.lines, font: .caption)
                    WidgetStateNote(state: snack.state, paint: paint)
                    Spacer(minLength: 0)
                    WidgetFooter(credits: credits, compact: true)
                }
            case .systemExtraLarge:
                VStack(alignment: .leading, spacing: 8) {
                    HStack(alignment: .top, spacing: 16) {
                        header.frame(width: 240, alignment: .leading)
                        context
                    }
                    WidgetFooter(credits: credits)
                }
            default:
                VStack(alignment: .leading, spacing: 6) {
                    header
                    context
                    WidgetFooter(credits: credits)
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(Text(snack.accessibility))
    }

    /// The time, or (no snack time ahead) nothing: the first line says so.
    @ViewBuilder private var headline: some View {
        if !snack.headline.text.isEmpty {
            WidgetHeadline(quantity: snack.headline, size: headlineSize, dimmed: !snack.state.isFresh)
        }
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack {
                WidgetName(name: name, symbol: WidgetSymbols.kind(.snack), font: .subheadline.weight(.semibold))
                Spacer(minLength: 4)
                Text(snack.title).font(.caption).foregroundStyle(.secondary)
            }
            headline
            WidgetLines(lines: snack.lines, font: .subheadline, limit: 3)
            WidgetStateNote(state: snack.state, paint: paint)
        }
    }

    @ViewBuilder private var context: some View {
        if let tideWindow {
            TideCurve(window: tideWindow, paint: paint, labels: true)
        } else if let flowSpark {
            FlowSpark(values: flowSpark, paint: paint)
        } else {
            Spacer(minLength: 0)
        }
    }
}

// MARK: - Next trip

struct TripWidgetView: View {
    let trip: ViewModel.WidgetTrip
    let entry: ViewModel.WidgetEntry
    let places: [ViewModel.WidgetPlace]
    let family: WidgetFamily
    let paint: WidgetPaint
    @Environment(\.colorTheme) private var theme

    private var compact: ViewModel.TripCompact { trip.compact }
    private var showsPlaces: Bool { family == .systemLarge || family == .systemExtraLarge }
    private var credits: WidgetCredits {
        WidgetCredits.trip(trip, places: entry.places, parts: showsPlaces)
    }
    private var verdictColor: Color {
        WidgetInk(theme: theme, paint: paint)(ColorRole.verdict(compact.verdict ?? "unknown"))
    }

    var body: some View {
        Group {
            switch family {
            case .systemSmall:
                VStack(alignment: .leading, spacing: 3) {
                    title(lines: 2)
                    verdict(font: .subheadline.weight(.semibold), lines: 3)
                    Spacer(minLength: 0)
                    WidgetFooter(credits: credits, compact: true)
                }
            case .systemMedium:
                VStack(alignment: .leading, spacing: 4) {
                    title(lines: 1)
                    verdict(font: .headline, lines: 2)
                    Text(compact.line).font(.caption).foregroundStyle(.secondary).lineLimit(2)
                    Spacer(minLength: 0)
                    WidgetFooter(credits: credits, compact: true)
                }
            default:
                VStack(alignment: .leading, spacing: 8) {
                    title(lines: 1)
                    verdict(font: .title3.weight(.semibold), lines: 2)
                    Text(compact.line).font(.subheadline).foregroundStyle(.secondary).lineLimit(2)
                    Divider()
                    placeList
                    Spacer(minLength: 0)
                    WidgetFooter(credits: credits)
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(Text(compact.accessibility))
    }

    private func title(lines: Int) -> some View {
        VStack(alignment: .leading, spacing: 1) {
            Label {
                Text(compact.title).lineLimit(lines).minimumScaleFactor(0.8)
            } icon: {
                Image(systemName: WidgetSymbols.kind(.nextTrip))
            }
            .font(.caption.weight(.semibold))
            Text(compact.when).font(.caption2).foregroundStyle(.secondary).lineLimit(1)
        }
    }

    private func verdict(font: Font, lines: Int) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 5) {
            Image(systemName: WidgetSymbols.verdict(compact.verdict))
                .foregroundStyle(verdictColor)
                .widgetAccentable()
            if compact.storm {
                Image(systemName: "hurricane")
                    .foregroundStyle(WidgetInk(theme: theme, paint: paint)(.alertSevere))
            }
            Text(compact.headline)
                .lineLimit(lines)
                .minimumScaleFactor(0.8)
                .fixedSize(horizontal: false, vertical: true)
        }
        .font(font)
    }

    /// Each place of the trip with its tide, flow and snack time at this entry: side by side
    /// on the extra large size.
    private var placeList: some View {
        let layout =
            family == .systemExtraLarge
            ? AnyLayout(HStackLayout(alignment: .top, spacing: 16))
            : AnyLayout(VStackLayout(alignment: .leading, spacing: 8))
        return layout {
            ForEach(Array(zip(entry.places.indices, entry.places)), id: \.0) { index, part in
                placeView(name: places.indices.contains(index) ? places[index].name : "", part: part)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
    }

    private func placeView(name: String, part: ViewModel.WidgetPlaceEntry) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(name).font(.subheadline.weight(.semibold)).lineLimit(1)
            if let tide = part.tide {
                partRow(symbol: WidgetSymbols.kind(.tide), quantity: tide.card.headline, line: tide.card.lines.first)
            }
            if let flow = part.flow {
                partRow(symbol: WidgetSymbols.kind(.flow), quantity: flow.card.headline, line: flow.trend?.text)
            }
            if let snack = part.snack, let line = snack.lines.first {
                Label(line, systemImage: WidgetSymbols.kind(.snack))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
            }
        }
    }

    private func partRow(symbol: String, quantity: ViewModel.Quantity, line: String?) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 5) {
            Image(systemName: symbol).font(.caption).foregroundStyle(.secondary)
            Text(quantity.text).font(.subheadline.weight(.semibold))
            Text(quantity.unit).font(.caption).foregroundStyle(.secondary)
            if let line {
                Text(line).font(.caption).foregroundStyle(.secondary).lineLimit(1).minimumScaleFactor(0.8)
            }
        }
    }
}
