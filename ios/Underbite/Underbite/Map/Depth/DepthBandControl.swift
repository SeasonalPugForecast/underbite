import SwiftUI
import UnderbiteKit

/// The depth band as a range slider under its title and the band in the user's unit
/// ("Highlight", "15–25 ft"): two ends that snap to the core's round stops (shallow on the
/// left), the depths between them highlighted on the map. The labels are the stops' own and
/// every word is the core's (`depth_words`). Without a band both ends rest at the track's ends
/// and the track is plain; moving either end sets one, and the clear button takes it away.
struct DepthBandControl: View {
    let stops: [DisplayStop]
    /// The band as positions in `stops`; none for no band.
    let band: ClosedRange<Int>?
    let words: DepthWords
    let change: (ClosedRange<Int>?) -> Void

    /// The band while an end is being dragged.
    @State private var dragging: ClosedRange<Int>?

    private static let thumb: CGFloat = 28
    private static let track: CGFloat = 6
    private static let target: CGFloat = 44
    private static let space = "depth-band"

    private var shown: ClosedRange<Int>? { dragging ?? band }
    private var resting: ClosedRange<Int> { 0...max(stops.count - 1, 0) }

    var body: some View {
        if stops.count > 1 {
            VStack(alignment: .leading, spacing: 6) {
                HStack(alignment: .firstTextBaseline) {
                    Text(words.bandTitle)
                    Spacer(minLength: 8)
                    Text(words.bandText)
                        .monospacedDigit()
                        .foregroundStyle(
                            band == nil ? AnyShapeStyle(.readableSecondary) : AnyShapeStyle(.primary))
                }
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("map-depth-band-title")
                HStack(spacing: 8) {
                    slider
                    if band != nil {
                        Button {
                            change(nil)
                        } label: {
                            Image(systemName: "xmark.circle.fill")
                                .font(.title3)
                                .foregroundStyle(.secondary)
                                .frame(minWidth: 44, minHeight: 44)
                        }
                        .buttonStyle(.borderless)
                        .accessibilityLabel(words.bandClearLabel)
                        .accessibilityIdentifier("map-depth-band-clear")
                    }
                }
                labels
            }
            .accessibilityIdentifier("map-depth-band")
        }
    }

    private var slider: some View {
        GeometryReader { geometry in
            let width = geometry.size.width - Self.thumb
            let x = { (i: Int) -> CGFloat in
                Self.thumb / 2 + width * CGFloat(i) / CGFloat(stops.count - 1)
            }
            let range = shown ?? resting
            ZStack(alignment: .leading) {
                Capsule()
                    .fill(Color(.systemFill))
                    .frame(height: Self.track)
                    .padding(.horizontal, Self.thumb / 2)
                if shown != nil {
                    Capsule()
                        .fill(Palette.accent)
                        .frame(width: x(range.upperBound) - x(range.lowerBound), height: Self.track)
                        .offset(x: x(range.lowerBound))
                }
                end(shallow: true, at: range.lowerBound, x: x, width: width)
                end(shallow: false, at: range.upperBound, x: x, width: width)
            }
            .frame(height: geometry.size.height)
            .coordinateSpace(.named(Self.space))
        }
        .frame(height: 44)
    }

    /// One end of the band: dragged along the track, or moved a stop at a time by VoiceOver.
    private func end(
        shallow: Bool, at position: Int, x: @escaping (Int) -> CGFloat, width: CGFloat
    ) -> some View {
        Circle()
            .fill(shown == nil ? Color(.systemBackground) : Palette.accent)
            .overlay(Circle().strokeBorder(Palette.accent, lineWidth: 2))
            .shadow(color: .black.opacity(0.2), radius: 2, y: 1)
            .frame(width: Self.thumb, height: Self.thumb)
            // A touch target of 44 points round the thumb.
            .frame(width: Self.target, height: Self.target)
            .contentShape(Rectangle())
            .offset(x: x(position) - Self.target / 2)
            .gesture(
                DragGesture(minimumDistance: 2, coordinateSpace: .named(Self.space))
                    .onChanged { drag in
                        let fraction = Double((drag.location.x - Self.thumb / 2) / max(width, 1))
                        let to = DepthBand.position(at: fraction, count: stops.count)
                        dragging = DepthBand.moved(
                            dragging ?? band ?? resting, shallow: shallow, to: to,
                            count: stops.count)
                    }
                    .onEnded { _ in
                        if let dragging { change(dragging) }
                        dragging = nil
                    }
            )
            .accessibilityElement()
            .accessibilityLabel(shallow ? words.bandFromLabel : words.bandToLabel)
            .accessibilityValue(band == nil ? words.bandOff : stops[position].label)
            .accessibilityAdjustableAction { direction in
                let step = direction == .increment ? 1 : -1
                change(
                    DepthBand.moved(
                        band ?? resting, shallow: shallow, to: position + step,
                        count: stops.count))
            }
    }

    /// The stops' labels under the track: every one while they fit, else the ends of the band
    /// ("from 15 ft", "to 25 ft"), or of the track without one.
    private var labels: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 0) {
                ForEach(Array(stops.enumerated()), id: \.offset) { index, stop in
                    if index > 0 { Spacer(minLength: 6) }
                    Text(stop.label)
                        .fontWeight(shown?.contains(index) == true ? .semibold : .regular)
                        .fixedSize()
                }
            }
            HStack(alignment: .firstTextBaseline) {
                if let range = shown {
                    endLabel(words.bandFrom, stops[range.lowerBound].label)
                    Spacer(minLength: 6)
                    endLabel(words.bandTo, stops[range.upperBound].label)
                } else {
                    Text(stops[resting.lowerBound].label)
                    Spacer(minLength: 6)
                    Text(stops[resting.upperBound].label)
                }
            }
        }
        .font(.caption.monospacedDigit())
        .accessibilityHidden(true)
    }

    /// One end's word and depth, side by side.
    private func endLabel(_ word: String, _ label: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 4) {
            Text(word)
                .foregroundStyle(.readableSecondary)
            Text(label)
                .fontWeight(.semibold)
        }
    }
}
