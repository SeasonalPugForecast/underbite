import SwiftUI
@preconcurrency import UnderbiteKit

/// The storms module's part of the layer drawer (the side panel on the iPad, the sheet on the
/// iPhone): whether the outlook is loading or could not be loaded, the storm near the map's
/// centre, the time step whose lows are marked, the legend (each model's band and dash, NHC's
/// tracks and NHC's forecast cone with what it is and how it is made), each active hurricane with
/// NHC's advisory link, the safety text, when the outlook was checked, the credits and "Not for
/// navigation.". Every word is the core's; the safety text is whole and in the plain voice.
struct StormsSection: View {
    let module: StormsModule

    var body: some View {
        Section {
            if let layer = module.layer {
                content(layer)
            } else if let words = module.words {
                if module.failed {
                    Label(words.failed, systemImage: "exclamationmark.triangle")
                        .font(.subheadline)
                        .accessibilityIdentifier("map-storms-status")
                } else {
                    HStack(spacing: 8) {
                        ProgressView().controlSize(.small)
                        Text(words.loading).font(.subheadline)
                    }
                    .accessibilityElement(children: .combine)
                    .accessibilityIdentifier("map-storms-status")
                }
                Text(words.safety)
                    .font(.caption)
                    .fixedSize(horizontal: false, vertical: true)
                Text(words.notice)
                    .font(.caption.weight(.bold))
            }
        } header: {
            Text(module.info?.title ?? "")
        }
    }

    @ViewBuilder
    private func content(_ layer: StormLayer) -> some View {
        if let status = layer.status {
            Label(status, systemImage: "exclamationmark.triangle")
                .font(.subheadline)
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityIdentifier("map-storms-status")
        }
        if let summary = layer.summary {
            Text(summary)
                .font(.subheadline.weight(.semibold))
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityIdentifier("map-storms-summary")
        }
        if layer.steps.count > 1 {
            StormStepControl(steps: layer.steps, step: module.step) { module.setStep($0) }
        }
        ForEach(Array(layer.hurricanes.enumerated()), id: \.element.id) { _, hurricane in
            StormHurricaneRow(hurricane: hurricane, linkTitle: layer.advisoryLinkTitle)
        }
        StormsLegend(layer: layer)
        VStack(alignment: .leading, spacing: 6) {
            Text(layer.safety)
                .font(.caption)
                .fixedSize(horizontal: false, vertical: true)
            if let checked = layer.checked {
                Text(checked)
                    .font(.caption)
                    .foregroundStyle(.readableSecondary)
            }
            ForEach(layer.credits, id: \.self) { credit in
                Text(credit)
                    .font(.caption)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Text(layer.notice)
                .font(.caption.weight(.bold))
        }
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("map-storms-credits")
    }
}

/// The time step whose lows are marked, over the core's steps ("Sun 7 PM · +72 h").
private struct StormStepControl: View {
    let steps: [StormStep]
    let step: Int
    let choose: (Int) -> Void

    var body: some View {
        let index = StormMapRules.startStep(step, count: steps.count)
        VStack(alignment: .leading, spacing: 4) {
            Text(steps[index].label)
                .font(.subheadline.monospacedDigit())
            Slider(
                value: Binding(
                    get: { Double(index) },
                    set: { choose(Int($0.rounded())) }),
                in: 0...Double(steps.count - 1), step: 1
            )
            .accessibilityValue(steps[index].label)
            .accessibilityIdentifier("map-storms-step")
        }
    }
}

/// An active hurricane: its symbol in its colour, its title, strength and advisory, and NHC's
/// advisory link.
private struct StormHurricaneRow: View {
    let hurricane: StormHurricane
    let linkTitle: String

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Label {
                Text(hurricane.title)
                    .font(.subheadline.weight(.semibold))
                    .fixedSize(horizontal: false, vertical: true)
            } icon: {
                Image(systemName: "hurricane")
                    .foregroundStyle(hurricane.colour.color)
            }
            Text(hurricane.strength)
                .font(.caption)
                .fixedSize(horizontal: false, vertical: true)
            Text(hurricane.advisory)
                .font(.caption)
                .foregroundStyle(.readableSecondary)
            if let url = hurricane.advisoryUrl.flatMap(URL.init(string:)) {
                Link(linkTitle, destination: url)
                    .font(.caption.weight(.semibold))
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("map-storms-hurricane-\(hurricane.id)")
    }
}
