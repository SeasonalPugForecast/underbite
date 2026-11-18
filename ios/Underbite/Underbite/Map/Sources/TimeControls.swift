import MapKit
import SwiftUI
import UnderbiteKit

/// The date and time controls of the topmost dated layer on the map: a satellite layer's day
/// (the day served, a picker of recent days), a grid layer's day from its time axis (with "last
/// clear day" for a cloudy one), an hourly grid's slider over the last day, or the radar loop's
/// slider and play button. Every date and time shown is the core's label.
struct TimeControls: View {
    let controller: MapController

    init(controller: MapController) {
        self.controller = controller
    }

    private var module: SourcesModule? {
        controller.modules.lazy.compactMap { $0 as? SourcesModule }.first
    }

    var body: some View {
        if let module, let top = DateControl.topmost(controller.plan) {
            VStack(alignment: .leading, spacing: 6) {
                Text(top.layer.info.title)
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(Color.primary)
                switch top.control {
                case .satelliteDays:
                    SatelliteDayControl(module: module, layer: top.layer)
                case .gridDays:
                    GridDayControl(module: module, layer: top.layer)
                case .gridHours:
                    GridHourControl(module: module, layer: top.layer)
                case .radarFrames:
                    RadarLoopControl(module: module)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 12))
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("map-time-controls")
        }
    }
}

/// A button that steps to the day or time before or after.
private struct StepButton: View {
    let back: Bool
    let label: String
    let action: (() -> Void)?

    var body: some View {
        Button {
            action?()
        } label: {
            Image(systemName: back ? "chevron.backward" : "chevron.forward")
                .font(.body.weight(.semibold))
                .frame(width: 36, height: 36)
        }
        .buttonStyle(.glass)
        .disabled(action == nil)
        .accessibilityLabel(label)
    }
}

/// The day shown, as a menu of the days to choose from.
private struct DayMenu<Item: Hashable>: View {
    let shown: String
    let items: [Item]
    let label: (Item) -> String
    let isShown: (Item) -> Bool
    let choose: (Item) -> Void

    var body: some View {
        Menu {
            ForEach(items, id: \.self) { item in
                Button {
                    choose(item)
                } label: {
                    if isShown(item) {
                        Label(label(item), systemImage: "checkmark")
                    } else {
                        Text(label(item))
                    }
                }
            }
        } label: {
            Label(shown, systemImage: "calendar")
                .lineLimit(1)
                .fixedSize()
                .font(.body.weight(.medium))
                .foregroundStyle(Color.primary)
                .padding(.horizontal, 12)
                .frame(minHeight: 36)
        }
        .buttonStyle(.glass)
        .accessibilityIdentifier("map-date-picker")
    }
}

/// A satellite layer's day: the day served, the days before and after, and the picker.
private struct SatelliteDayControl: View {
    let module: SourcesModule
    let layer: PlannedLayer

    var body: some View {
        let days = module.satelliteDays()
        if let day = module.days[layer.id] {
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 8) {
                    StepButton(back: true, label: "Day before", action: before(day))
                    DayMenu(
                        shown: day.asked.label, items: days, label: \.label,
                        isShown: { $0 == day.asked }, choose: { module.pick($0, for: layer.id) })
                    StepButton(back: false, label: "Day after", action: after(day, in: days))
                }
                if day.exhausted {
                    Note(text: "No picture for these days.")
                } else if day.steps > 0 {
                    Note(text: "The days after it have no picture yet.")
                }
            }
        } else {
            ProgressView()
        }
    }

    private func before(_ day: SatelliteDay) -> (() -> Void)? {
        guard let before = try? previousDate(date: day.asked.date) else { return nil }
        return { module.pick(before, for: layer.id) }
    }

    /// The day after the one shown, among the days offered (none after the newest).
    private func after(_ day: SatelliteDay, in days: [MapDate]) -> (() -> Void)? {
        guard let at = days.firstIndex(of: day.asked), at > 0 else { return nil }
        let after = days[at - 1]
        return { module.pick(after, for: layer.id) }
    }
}

/// A grid layer's day from its time axis, and "last clear day".
private struct GridDayControl: View {
    let module: SourcesModule
    let layer: PlannedLayer

    var body: some View {
        if let axis = module.times(of: layer), let shown = GridTimes.shown(layer.time, in: axis) {
            VStack(alignment: .leading, spacing: 4) {
                // One row where it fits; on a narrow screen "last clear day" goes under the days.
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: 8) {
                        days(axis, shown)
                        clearDayButton
                    }
                    VStack(alignment: .leading, spacing: 6) {
                        days(axis, shown)
                        clearDayButton
                    }
                }
                switch module.clearSearches[layer.id] {
                case .looking:
                    HStack(spacing: 6) {
                        ProgressView()
                        Note(text: "Looking for a day with values in view.")
                    }
                case .some(.none):
                    Note(text: "No day with values in view among the last days.")
                default:
                    EmptyView()
                }
            }
        } else {
            ProgressView()
        }
    }

    /// The day before, the day shown with the days of the axis to choose from, the day after.
    private func days(_ axis: [LayerTime], _ shown: LayerTime) -> some View {
        HStack(spacing: 8) {
            StepButton(back: true, label: "Day before", action: step(-1, axis))
            DayMenu(
                shown: shown.label, items: Array(axis.reversed()), label: \.label,
                isShown: { $0.time == shown.time },
                choose: { module.pick(GridTimes.checked($0.time, in: axis), for: layer.id) })
            StepButton(back: false, label: "Day after", action: step(1, axis))
        }
    }

    @ViewBuilder private var clearDayButton: some View {
        if layer.kind == .gridRaster {
            Button {
                module.findClearDay(layer)
            } label: {
                Label("Last clear day", systemImage: "sun.max")
                    .lineLimit(1)
                    .fixedSize()
                    .padding(.horizontal, 4)
                    .frame(minHeight: 36)
            }
            .buttonStyle(.glass)
            .disabled(!module.canFindClearDay(layer) || module.clearSearches[layer.id] == .looking)
            .accessibilityIdentifier("map-clear-day")
        }
    }

    private func step(_ by: Int, _ axis: [LayerTime]) -> (() -> Void)? {
        guard let next = GridTimes.step(from: layer.time, by: by, in: axis) else { return nil }
        return { module.pick(next, for: layer.id) }
    }
}

/// An hourly grid's hour: a slider over the last day's hours, moved to on release.
private struct GridHourControl: View {
    let module: SourcesModule
    let layer: PlannedLayer
    @State private var dragged: Double?

    var body: some View {
        if let axis = module.times(of: layer) {
            let offered = GridTimes.offered(axis, dates: .hourly, now: module.now)
            let at = Double(
                layer.time.flatMap { t in offered.firstIndex { $0.time == t } } ?? offered.count - 1)
            let index = Int((dragged ?? at).rounded())
            let label = offered.indices.contains(index) ? offered[index].label : ""
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 8) {
                    StepButton(back: true, label: "Hour before", action: step(-1, offered))
                    Text(label)
                        .font(.body.weight(.medium).monospacedDigit())
                        .foregroundStyle(Color.primary)
                        .accessibilityIdentifier("map-date-label")
                    Spacer(minLength: 0)
                    StepButton(back: false, label: "Hour after", action: step(1, offered))
                }
                if offered.count > 1 {
                    Slider(
                        value: Binding(get: { dragged ?? at }, set: { dragged = $0 }),
                        in: 0...Double(offered.count - 1), step: 1,
                        onEditingChanged: { editing in
                            guard !editing, let value = dragged else { return }
                            dragged = nil
                            let i = Int(value.rounded())
                            guard offered.indices.contains(i) else { return }
                            module.pick(
                                i == offered.count - 1 ? nil : offered[i].time, for: layer.id)
                        }
                    )
                    .accessibilityLabel("Hour")
                    .accessibilityValue(label)
                }
            }
        } else {
            ProgressView()
        }
    }

    private func step(_ by: Int, _ offered: [LayerTime]) -> (() -> Void)? {
        guard let next = GridTimes.step(from: layer.time, by: by, in: offered) else { return nil }
        return { module.pick(next, for: layer.id) }
    }
}

/// The radar loop: play and pause (not in the battery-friendly mode), the frame's time and a
/// slider over the frames.
private struct RadarLoopControl: View {
    let module: SourcesModule

    var body: some View {
        let frames = module.frames
        if frames.isEmpty {
            ProgressView()
        } else {
            let index = min(module.frameIndex, frames.count - 1)
            HStack(spacing: 8) {
                if module.canPlay {
                    Button {
                        module.playing ? module.pause() : module.play()
                    } label: {
                        Image(systemName: module.playing ? "pause.fill" : "play.fill")
                            .frame(width: 36, height: 36)
                    }
                    .buttonStyle(.glass)
                    .accessibilityLabel(module.playing ? "Pause" : "Play")
                }
                Text(frames[index].label)
                    .font(.body.weight(.medium).monospacedDigit())
                    .foregroundStyle(Color.primary)
                    .accessibilityIdentifier("map-date-label")
                if frames.count > 1 {
                    Slider(
                        value: Binding(
                            get: { Double(index) },
                            set: {
                                module.pause()
                                module.showFrame(Int($0.rounded()))
                            }),
                        in: 0...Double(frames.count - 1), step: 1
                    )
                    .accessibilityLabel("Radar frame")
                    .accessibilityValue(frames[index].label)
                }
            }
        }
    }
}

/// A line under a control: what the layer shows instead, or what "last clear day" found.
private struct Note: View {
    let text: String

    var body: some View {
        Text(text)
            .font(.footnote)
            .foregroundStyle(.readableSecondary)
            .fixedSize(horizontal: false, vertical: true)
    }
}
