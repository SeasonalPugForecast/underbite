import SwiftUI
import UnderbiteKit

/// A theme's colours at work, for the Colours page and the editor: a tide card, the go / no-go
/// badges and the river flow bands, painted with `theme` in the current appearance. The
/// words are the roles' own names; nothing here is a reading.
struct ThemePreview: View {
    let theme: ColorTheme

    var body: some View {
        VStack(spacing: 12) {
            TideCardPreview(theme: theme)
            VerdictBadges(theme: theme)
            FlowBands(theme: theme)
        }
        .environment(\.colorTheme, theme)
    }
}

/// The card a preview sits in, as the conditions cards are framed.
private struct PreviewCard<Content: View>: View {
    let title: String
    @ViewBuilder var content: Content
    @Environment(\.sunMode) private var sunMode

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(title)
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(.readableSecondary)
            content
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(14)
        .background(Color(.secondarySystemGroupedBackground), in: .rect(cornerRadius: 16, style: .continuous))
        .overlay {
            if sunMode {
                RoundedRectangle(cornerRadius: 16, style: .continuous)
                    .strokeBorder(Palette.cardStroke(sunMode), lineWidth: 2)
            }
        }
    }
}

/// A day of tide: the night at both ends, a snack-time window, slack water at the turns, the
/// curve with the fill under it, the high and low marks and the now line; below, the card
/// states' marks.
private struct TideCardPreview: View {
    let theme: ColorTheme

    var body: some View {
        PreviewCard(title: ColorRoleGroup.tide.title) {
            TideSketch(theme: theme)
                .frame(height: 96)
                .accessibilityHidden(true)
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 130), spacing: 8)], alignment: .leading, spacing: 6) {
                legend
            }
            .font(.footnote)
        }
        .accessibilityElement(children: .combine)
    }

    @ViewBuilder private var legend: some View {
        Key(role: .tideHigh, symbol: "arrowtriangle.up.fill", theme: theme)
        Key(role: .tideLow, symbol: "arrowtriangle.down.fill", theme: theme)
        Key(role: .cardFresh, symbol: "checkmark.circle.fill", theme: theme)
        Key(role: .cardStale, symbol: "clock.badge.exclamationmark", theme: theme)
    }

    /// A role's symbol and name, both in its colour (the words are what the colour is checked
    /// as).
    private struct Key: View {
        let role: ColorRole
        let symbol: String
        let theme: ColorTheme

        var body: some View {
            HStack(spacing: 6) {
                Image(systemName: symbol).accessibilityHidden(true)
                Text(role.title).lineLimit(1).minimumScaleFactor(0.8)
            }
            .foregroundStyle(theme.color(role))
        }
    }
}

/// The tide drawing: two highs and two lows over a day.
private struct TideSketch: View {
    let theme: ColorTheme
    @Environment(\.sunMode) private var sunMode

    /// The height (0 low … 1 high) at `x` (0…1 of the day).
    nonisolated private static func height(_ x: Double) -> Double {
        0.5 + 0.42 * cos((x - 0.2) * 4 * .pi)
    }

    var body: some View {
        Canvas { context, size in
            let w = size.width, h = size.height
            func point(_ x: Double) -> CGPoint {
                CGPoint(x: x * w, y: h - 6 - Self.height(x) * (h - 12))
            }
            // Night, the window and slack water behind the curve.
            for span in [0.0...0.22, 0.8...1.0] {
                context.fill(
                    Path(CGRect(x: span.lowerBound * w, y: 0, width: (span.upperBound - span.lowerBound) * w, height: h)),
                    with: .color(theme.color(.timelineNight)))
            }
            context.fill(
                Path(CGRect(x: 0.36 * w, y: 0, width: 0.12 * w, height: h)), with: .color(theme.color(.timelineWindow)))
            for turn in [0.2, 0.45, 0.7, 0.95] {
                context.fill(
                    Path(CGRect(x: (turn - 0.025) * w, y: 0, width: 0.05 * w, height: h)),
                    with: .color(theme.color(.tideSlack)))
            }
            var curve = Path()
            let steps = 96
            for i in 0...steps {
                let p = point(Double(i) / Double(steps))
                if i == 0 { curve.move(to: p) } else { curve.addLine(to: p) }
            }
            var fill = curve
            fill.addLine(to: CGPoint(x: w, y: h))
            fill.addLine(to: CGPoint(x: 0, y: h))
            fill.closeSubpath()
            context.fill(fill, with: .color(theme.color(.tideFill)))
            context.stroke(curve, with: .color(theme.color(.tideCurve)), lineWidth: 2 * Palette.strokeScale(sunMode))
            // The now line.
            context.stroke(
                Path { $0.addLines([CGPoint(x: 0.58 * w, y: 0), CGPoint(x: 0.58 * w, y: h)]) },
                with: .color(theme.color(.chartNow)), lineWidth: 1.5 * Palette.strokeScale(sunMode))
            // High and low marks.
            for (x, role) in [(0.2, ColorRole.tideHigh), (0.45, .tideLow), (0.7, .tideHigh), (0.95, .tideLow)] {
                let p = point(x)
                context.fill(
                    Path(ellipseIn: CGRect(x: p.x - 4.5, y: p.y - 4.5, width: 9, height: 9)),
                    with: .color(theme.color(role)))
            }
        }
    }
}

/// The verdicts as the go / no-go card shows them: each its symbol and word in its colour.
private struct VerdictBadges: View {
    let theme: ColorTheme

    private static let verdicts: [(role: ColorRole, verdict: String)] = [
        (.verdictGo, "go"), (.verdictMarginal, "marginal"), (.verdictNoGo, "no_go"), (.verdictUnknown, "unknown"),
    ]

    var body: some View {
        PreviewCard(title: ColorRoleGroup.verdicts.title) {
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 118), spacing: 8)], alignment: .leading, spacing: 8) {
                ForEach(Self.verdicts, id: \.role) { item in
                    Label(item.role.title, systemImage: GoNoGoDetail.symbol(item.verdict))
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(theme.color(item.role))
                        .lineLimit(1)
                        .minimumScaleFactor(0.8)
                        .padding(.horizontal, 10)
                        .padding(.vertical, 6)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(theme.color(item.role).opacity(0.12), in: .capsule)
                        .overlay { Capsule().strokeBorder(theme.color(item.role), lineWidth: 1) }
                }
            }
        }
        .accessibilityElement(children: .combine)
    }
}

/// River flow against normal: the seven classes as a band, record low to record high, and
/// their names; the flood stages beside them as a second band.
private struct FlowBands: View {
    let theme: ColorTheme

    var body: some View {
        PreviewCard(title: ColorRoleGroup.flow.title) {
            band(ColorRole.flowClasses)
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 150), spacing: 8)], alignment: .leading, spacing: 4) {
                ForEach(ColorRole.flowClasses, id: \.self) { role in
                    HStack(spacing: 6) {
                        RoundedRectangle(cornerRadius: 3).fill(theme.color(role)).frame(width: 14, height: 14)
                        Text(role.title).font(.footnote).lineLimit(1).minimumScaleFactor(0.8)
                    }
                }
            }
            Text(ColorRoleGroup.flood.title)
                .font(.footnote.weight(.semibold))
                .foregroundStyle(.readableSecondary)
                .padding(.top, 2)
            band(ColorRole.floodStages)
        }
        .accessibilityElement(children: .combine)
    }

    private func band(_ roles: [ColorRole]) -> some View {
        HStack(spacing: 2) {
            ForEach(roles, id: \.self) { Rectangle().fill(theme.color($0)) }
        }
        .frame(height: 18)
        .clipShape(.rect(cornerRadius: 6))
        .accessibilityHidden(true)
    }
}

/// A theme's small swatch for a list row: the flow band over the verdicts.
struct ThemeSwatch: View {
    let theme: ColorTheme

    var body: some View {
        VStack(spacing: 2) {
            HStack(spacing: 0) {
                ForEach(ColorRole.flowClasses, id: \.self) { Rectangle().fill(theme.color($0)) }
            }
            HStack(spacing: 0) {
                ForEach(ColorRole.verdicts, id: \.self) { Rectangle().fill(theme.color($0)) }
            }
        }
        .frame(width: 44, height: 28)
        .clipShape(.rect(cornerRadius: 6))
        .overlay { RoundedRectangle(cornerRadius: 6).strokeBorder(.separator, lineWidth: 0.5) }
        .accessibilityHidden(true)
    }
}
