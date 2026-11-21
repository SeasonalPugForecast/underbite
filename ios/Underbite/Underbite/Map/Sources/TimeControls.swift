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

    @State private var fold = TimeControlsFold()
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    /// Shots (debug builds): `-mapTimeControls expanded` opens the controls and keeps them open
    /// while the map settles.
    private static var startsExpanded: Bool {
        #if DEBUG
            LaunchArguments.value(after: "-mapTimeControls") == "expanded"
        #else
            false
        #endif
    }

    private func send(_ event: TimeControlsFold.Event) {
        let animation: Animation? = reduceMotion ? nil : .spring(duration: 0.3, bounce: 0.1)
        withAnimation(animation) { fold.apply(event) }
    }

    var body: some View {
        // The layers drawn from the device's store say so above the controls.
        LayerBadges(controller: controller)
        if let module, let top = DateControl.topmost(controller.plan) {
            let title = top.layer.info.title
            Group {
                if fold.expanded {
                    expanded(module: module, top: top, title: title)
                } else {
                    TimeChip(module: module, top: top, title: title, send: send)
                }
            }
            .dynamicTypeSize(...MapChromeType.largest)
            .environment(\.timeControlsEvent, send)
            .onChange(of: controller.region) { old, _ in
                // The first settle is the map finding its place, not the user moving it.
                if old != nil, fold.expanded, !Self.startsExpanded { send(.pan) }
            }
            .task {
                if Self.startsExpanded { send(.expand) }
            }
        }
    }

    private func expanded(
        module: SourcesModule, top: (layer: PlannedLayer, control: DateControl), title: String
    ) -> some View {
        HStack(alignment: .top, spacing: 4) {
            VStack(alignment: .leading, spacing: 4) {
                switch top.control {
                case .satelliteDays:
                    SatelliteDayControl(module: module, layer: top.layer, title: title)
                case .gridDays:
                    GridDayControl(module: module, layer: top.layer, title: title)
                case .gridHours:
                    GridHourControl(module: module, layer: top.layer, title: title)
                case .radarFrames:
                    RadarLoopControl(module: module, title: title)
                }
            }
            Button {
                send(.collapse)
            } label: {
                Image(systemName: "chevron.down")
                    .font(.body.weight(.semibold))
                    .frame(width: 36, height: 36)
            }
            .buttonStyle(.glass)
            .accessibilityLabel(String(localized: "Hide date controls", comment: "Accessibility label of the button that folds the map's date controls into a small chip"))
            .accessibilityIdentifier("map-time-collapse")
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 6)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 12))
        .accessibilityElement(children: .contain)
        .accessibilityAction(named: String(localized: "Hide date controls", comment: "Accessibility action that folds the map's date controls into a small chip")) {
            send(.collapse)
        }
        .accessibilityIdentifier("map-time-controls")
    }
}

extension EnvironmentValues {
    /// What the date controls report to the chip that holds them: a pick or a step.
    @Entry fileprivate var timeControlsEvent: (TimeControlsFold.Event) -> Void = { _ in }
}

/// The compact form: a glass chip, as wide as its content, with a symbol and the date or time
/// shown; the radar loop's chip keeps play and pause beside it.
private struct TimeChip: View {
    let module: SourcesModule
    let top: (layer: PlannedLayer, control: DateControl)
    let title: String
    let send: (TimeControlsFold.Event) -> Void

    /// The core's label of what the layer shows now, when it has one.
    private var shown: String? {
        let layer = top.layer
        switch top.control {
        case .satelliteDays:
            return module.days[layer.id]?.asked.label
        case .gridDays:
            return module.times(of: layer).flatMap { GridTimes.shown(layer.time, in: $0)?.label }
        case .gridHours:
            guard let axis = module.times(of: layer) else { return nil }
            let offered = GridTimes.offered(axis, dates: .hourly, now: module.now)
            let at = layer.time.flatMap { t in offered.firstIndex { $0.time == t } } ?? offered.count - 1
            return offered.indices.contains(at) ? offered[at].label : nil
        case .radarFrames:
            let frames = module.frames
            return frames.isEmpty ? nil : frames[min(module.frameIndex, frames.count - 1)].label
        }
    }

    private var symbol: String {
        switch top.control {
        case .satelliteDays, .gridDays: "calendar"
        case .gridHours, .radarFrames: "clock"
        }
    }

    var body: some View {
        HStack(spacing: 6) {
            if top.control == .radarFrames, module.canPlay {
                Button {
                    module.playing ? module.pause() : module.play()
                } label: {
                    Image(systemName: module.playing ? "pause.fill" : "play.fill")
                        .frame(width: 44, height: 44)
                }
                .buttonStyle(.glass)
                .accessibilityLabel(module.playing ? "Pause" : "Play")
            }
            Button {
                send(.expand)
            } label: {
                if let shown {
                    Label(shown, systemImage: symbol)
                        .font(.subheadline.weight(.medium).monospacedDigit())
                        .foregroundStyle(Color.primary)
                        .lineLimit(2)
                        .frame(minHeight: 28)
                        .padding(.horizontal, 4)
                        .accessibilityIdentifier("map-date-label")
                } else {
                    ProgressView().frame(minHeight: 28)
                }
            }
            .buttonStyle(.glass)
            .accessibilityLabel(shown.map { "\(title), \($0)" } ?? title)
            .accessibilityHint(String(localized: "Shows the date controls", comment: "Accessibility hint of the chip that opens the map's date controls"))
            .accessibilityIdentifier("map-date-picker")
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("map-time-controls")
    }
}

/// A button that steps to the day or time before or after.
private struct StepButton: View {
    let back: Bool
    let label: String
    let action: (() -> Void)?
    @Environment(\.timeControlsEvent) private var event

    var body: some View {
        Button {
            action?()
            event(.step)
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
    /// The layer's title, a caption above the day.
    let title: String
    let shown: String
    let items: [Item]
    let label: (Item) -> String
    let isShown: (Item) -> Bool
    let choose: (Item) -> Void
    @Environment(\.timeControlsEvent) private var event

    var body: some View {
        Menu {
            ForEach(items, id: \.self) { item in
                Button {
                    choose(item)
                    event(.pick)
                } label: {
                    if isShown(item) {
                        Label(label(item), systemImage: "checkmark")
                    } else {
                        Text(label(item))
                    }
                }
            }
        } label: {
            VStack(spacing: 0) {
                Text(title)
                    .font(.caption2)
                    .foregroundStyle(.readableSecondary)
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
                Label(shown, systemImage: "calendar")
                    .lineLimit(1)
                    .minimumScaleFactor(0.7)
                    .font(.subheadline.weight(.medium))
                    .foregroundStyle(Color.primary)
            }
            .padding(.horizontal, 8)
            .frame(maxWidth: .infinity, minHeight: 36)
        }
        .buttonStyle(.glass)
        .accessibilityIdentifier("map-date-picker")
    }
}

/// A satellite layer's day: the day served, the days before and after, and the picker.
private struct SatelliteDayControl: View {
    let module: SourcesModule
    let layer: PlannedLayer
    let title: String

    var body: some View {
        let days = module.satelliteDays()
        if let day = module.days[layer.id] {
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 8) {
                    StepButton(back: true, label: String(localized: "Day before", comment: "Accessibility label of the button that steps the map layer back one day"), action: before(day))
                    DayMenu(
                        title: title, shown: day.asked.label, items: days, label: \.label,
                        isShown: { $0 == day.asked }, choose: { module.pick($0, for: layer.id) })
                    StepButton(back: false, label: String(localized: "Day after", comment: "Accessibility label of the button that steps the map layer forward one day"), action: after(day, in: days))
                }
                if day.exhausted {
                    Note(text: String(localized: "No picture for these days.", comment: "Note under the day control of a satellite layer that has no picture for the days offered"))
                } else if day.steps > 0 {
                    Note(text: String(localized: "The days after it have no picture yet.", comment: "Note under the day control of a satellite layer: later days have no picture yet"))
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
    let title: String
    @Environment(\.timeControlsEvent) private var event

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
                        Note(text: String(localized: "Looking for a day with values in view.", comment: "Note shown while the app looks for the latest day with values in the map view"))
                    }
                case .some(.none):
                    Note(text: String(localized: "No day with values in view among the last days.", comment: "Note shown when no recent day has values in the map view"))
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
            StepButton(back: true, label: String(localized: "Day before", comment: "Accessibility label of the button that steps the map layer back one day"), action: step(-1, axis))
            DayMenu(
                title: title, shown: shown.label, items: Array(axis.reversed()), label: \.label,
                isShown: { $0.time == shown.time },
                choose: { module.pick(GridTimes.checked($0.time, in: axis), for: layer.id) })
            StepButton(back: false, label: String(localized: "Day after", comment: "Accessibility label of the button that steps the map layer forward one day"), action: step(1, axis))
        }
    }

    @ViewBuilder private var clearDayButton: some View {
        if layer.kind == .gridRaster {
            Button {
                module.findClearDay(layer)
                event(.pick)
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
    let title: String
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
                    StepButton(back: true, label: String(localized: "Hour before", comment: "Accessibility label of the button that steps the map layer back one hour"), action: step(-1, offered))
                    TitledValue(title: title, value: label)
                        .frame(maxWidth: .infinity)
                    StepButton(back: false, label: String(localized: "Hour after", comment: "Accessibility label of the button that steps the map layer forward one hour"), action: step(1, offered))
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
                    .controlSize(.small)
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
    let title: String

    var body: some View {
        let frames = module.frames
        if frames.isEmpty {
            ProgressView()
        } else {
            let index = min(module.frameIndex, frames.count - 1)
            VStack(alignment: .leading, spacing: 2) {
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
                    TitledValue(title: title, value: frames[index].label, alignment: .leading)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
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
                    .controlSize(.small)
                    .accessibilityLabel("Radar frame")
                    .accessibilityValue(frames[index].label)
                }
            }
        }
    }
}

/// The layer's title as a caption above the date or time shown.
private struct TitledValue: View {
    let title: String
    let value: String
    var alignment: HorizontalAlignment = .center

    var body: some View {
        VStack(alignment: alignment, spacing: 0) {
            Text(title)
                .font(.caption2)
                .foregroundStyle(.readableSecondary)
                .lineLimit(1)
                .minimumScaleFactor(0.8)
            Text(value)
                .font(.subheadline.weight(.medium).monospacedDigit())
                .foregroundStyle(Color.primary)
                .lineLimit(1)
                .minimumScaleFactor(0.7)
                .accessibilityIdentifier("map-date-label")
        }
        .accessibilityElement(children: .combine)
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
