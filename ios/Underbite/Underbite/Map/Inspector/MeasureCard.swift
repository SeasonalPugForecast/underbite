import SwiftUI
import UnderbiteKit

/// The measured line: where it starts and ends (a point on the map, a saved spot or the phone's
/// location), the core's distance and true bearing, and the core's "not for navigation" note,
/// whole, on every measurement. Taps on the map move the end.
struct MeasureCard: View {
    let module: InspectorModule
    let settingsJSON: String
    @Environment(SpotStore.self) private var store
    @State private var problem: String?
    @State private var locating = false

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Label("Measure", systemImage: "ruler")
                    .font(.headline)
                Spacer()
                Button("Done") { module.stopMeasuring() }
                    .accessibilityIdentifier("measure-done")
            }
            if let line = module.line {
                end("From", line.from, start: true)
                if let to = line.to {
                    end("To", to, start: false)
                } else {
                    endChoices("To", start: false) {
                        Text("Tap the map")
                            .foregroundStyle(.readableSecondary)
                    }
                }
                result(line)
            }
            if let problem {
                Text(problem)
                    .font(.footnote)
                    .foregroundStyle(.readableSecondary)
            }
        }
        .padding(16)
        .readableText()
    }

    @ViewBuilder
    private func result(_ line: MeasureLine) -> some View {
        if let text = line.text(settingsJSON: settingsJSON) {
            VStack(alignment: .leading, spacing: 2) {
                Text(text.distance)
                    .font(.title.weight(.semibold).monospacedDigit())
                    .accessibilityIdentifier("measure-distance")
                Text(text.bearing)
                    .font(.title3.monospacedDigit())
                    .accessibilityIdentifier("measure-bearing")
            }
            .accessibilityElement(children: .combine)
            .accessibilityLabel(text.line)
            Notice(text: text.notice)
        } else if let notice = (try? appWords(settingsJson: settingsJSON))?.mapNotice {
            Notice(text: notice)
        }
    }

    /// One end of the line, with the choice of a spot or the phone's location instead.
    private func end(_ title: String, _ end: MeasureLine.End, start: Bool) -> some View {
        endChoices(title, start: start) {
            Text(label(end))
                .font(.body.monospacedDigit())
                .lineLimit(2)
        }
    }

    private func endChoices<Current: View>(
        _ title: String, start: Bool, @ViewBuilder current: () -> Current
    ) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text(title)
                .font(.subheadline.weight(.semibold))
                .frame(minWidth: 44, alignment: .leading)
            current()
                .frame(maxWidth: .infinity, alignment: .leading)
            Menu {
                Button("My location", systemImage: "location") { locate(start: start) }
                    .disabled(locating)
                if !store.spots.isEmpty {
                    Section("Spots") {
                        ForEach(store.spots) { spot in
                            Button(spot.name) {
                                let chosen = MeasureLine.End(
                                    kind: .spot(spot.id), latitude: spot.latitude,
                                    longitude: spot.longitude)
                                if start { module.setStart(chosen) } else { module.setEnd(chosen) }
                            }
                        }
                    }
                }
            } label: {
                Image(systemName: "ellipsis.circle")
                    .imageScale(.large)
                    .frame(minWidth: 44, minHeight: 44)
            }
            .accessibilityLabel(start ? "Start from" : "End at")
        }
    }

    /// How an end is named: the core's coordinates for a point, the spot's name, or the phone's
    /// location.
    private func label(_ end: MeasureLine.End) -> String {
        switch end.kind {
        case .point:
            (try? pointWords(point: end.mapPoint, settingsJson: settingsJSON))?.point ?? ""
        case .spot(let id):
            store.spot(id)?.name ?? ""
        case .here:
            "My location"
        }
    }

    /// Reads the phone's location for one end of the line.
    private func locate(start: Bool) {
        locating = true
        problem = nil
        Task {
            defer { locating = false }
            do {
                let fix = try await CurrentLocation.fix(reuseRecent: true)
                let here = MeasureLine.End(
                    kind: .here, latitude: fix.latitude, longitude: fix.longitude)
                if start { module.setStart(here) } else { module.setEnd(here) }
            } catch {
                problem = error.localizedDescription
            }
        }
    }
}

/// The core's "Not for navigation." under a measurement: whole, bold, on the warning colour's
/// edge, never in Luna's voice.
private struct Notice: View {
    let text: String

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Image(systemName: "exclamationmark.triangle.fill")
                .foregroundStyle(Palette.warning)
                .accessibilityHidden(true)
            Text(text)
                .font(.subheadline.weight(.bold))
                .foregroundStyle(Color.primary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Color(.systemBackground), in: RoundedRectangle(cornerRadius: 10))
        .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(Palette.warning, lineWidth: 2))
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("measure-notice")
    }
}
