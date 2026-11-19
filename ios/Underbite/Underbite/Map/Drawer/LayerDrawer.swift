import SwiftUI
import UnderbiteKit

/// The layer drawer: the core's catalogue by group, each overlay with its switch, opacity,
/// legend, credit and availability, the base maps to choose from, the modules' own sections and
/// the battery-friendly mode. Every layer's words are the catalogue's; switches, opacity and
/// order are written to the settings' `[map]` section. It is data-driven: a layer the catalogue
/// gains, or a module's section, shows here without changes.
struct LayerDrawer: View {
    let controller: MapController
    @Environment(SpotStore.self) private var store
    @State private var expanded: Set<String> = []
    @State private var started = false
    @State private var editMode: EditMode = .inactive
    /// The page shown instead of the layers (the presets). The drawer swaps its pages itself: a
    /// navigation stack inside the sheet the inspector becomes on the iPhone shows nothing.
    @State private var page: DrawerPage?

    init(controller: MapController) {
        self.controller = controller
        _page = State(initialValue: DrawerLaunch.showsPresets ? .presets : nil)
    }

    /// The map settings the map draws with (the launch's overrides included).
    private var drawnSettings: Settings.MapSettings? {
        try? Settings(json: controller.settingsJSON).map
    }

    private var groups: [DrawerGroup] {
        guard let map = drawnSettings else { return [] }
        return DrawerSections.groups(catalogue: controller.catalogue, settings: map)
    }

    private var editing: Bool { editMode.isEditing }

    var body: some View {
        Group {
            switch page {
            case .presets:
                PresetList(controller: controller) { withAnimation { page = nil } }
            case nil:
                layers
            }
        }
        .onAppear {
            guard !started else { return }
            started = true
            // The groups with a layer on open and the base maps; a launch opens the one it names.
            if let group = DrawerLaunch.group {
                expanded = [group]
            } else {
                expanded = Set(
                    groups.filter { $0.isBase || $0.layers.contains(where: \.shown) }.map(\.id))
            }
        }
    }

    private var layers: some View {
        List {
            let groups = groups
            if !editing {
                Section {
                    PresetLink { withAnimation { page = .presets } }
                }
                PacksDrawerSection(controller: controller)
            }
            ForEach(groups) { group in
                Section(isExpanded: expansion(group.id)) {
                    if group.isBase {
                        ForEach(group.layers, id: \.id) { layer in
                            BaseRow(layer: layer) { controller.setBase(layer.id) }
                        }
                    } else {
                        ForEach(group.layers, id: \.id) { layer in
                            if editing {
                                Text(layer.title)
                            } else {
                                LayerRow(layer: layer, controller: controller)
                            }
                        }
                        .onMove { source, destination in
                            let ids = group.layers.map(\.id)
                            edit {
                                DrawerSections.move(
                                    ids: ids, fromOffsets: source, toOffset: destination,
                                    in: &$0)
                            }
                        }
                    }
                } header: {
                    Text(group.title)
                }
                ForEach(modules(after: group, in: groups)) { $0.view }
            }
            ForEach(modules(after: nil, in: groups)) { $0.view }
            if !editing {
                BatterySection(
                    mode: drawnSettings?.batteryMode ?? "auto", on: controller.battery
                ) { mode in
                    edit { $0.batteryMode = mode }
                }
            }
        }
        .listStyle(.sidebar)
        .environment(\.editMode, $editMode)
        .safeAreaInset(edge: .top, spacing: 0) {
            DrawerHeader(title: "Layers") {
                Button(editing ? "Done" : "Reorder") {
                    withAnimation { editMode = editing ? .inactive : .active }
                }
                .accessibilityIdentifier("map-drawer-reorder")
            }
        }
    }

    /// Whether a group is open; every group is while the order is being edited.
    private func expansion(_ id: String) -> Binding<Bool> {
        Binding(
            get: { editing || expanded.contains(id) },
            set: { open in
                if open { expanded.insert(id) } else { expanded.remove(id) }
            })
    }

    /// The modules' own sections that go after `group` (after the last group holding one of
    /// their layers); with `group` nil, those of modules whose layers no group holds.
    private func modules(after group: DrawerGroup?, in groups: [DrawerGroup]) -> [ModuleSection] {
        guard !editing else { return [] }
        var out: [ModuleSection] = []
        for (offset, module) in controller.modules.enumerated() {
            let last = groups.last { g in g.layers.contains { module.layerIDs.contains($0.id) } }
            guard last?.id == group?.id, let view = module.drawerSection() else { continue }
            out.append(ModuleSection(id: offset, view: view))
        }
        return out
    }

    private func edit(_ change: (inout Settings.MapSettings) -> Void) {
        store.editMap(change)
    }
}

extension SpotStore {
    /// Applies `change` to the map section of the app's settings (not a launch's overrides) and
    /// saves them; nothing is written when nothing changed.
    func editMap(_ change: (inout Settings.MapSettings) -> Void) {
        guard var settings = try? Settings(json: settingsJSON) else { return }
        let before = settings.map
        change(&settings.map)
        guard settings.map != before, let json = try? settings.json() else { return }
        try? updateSettings(json)
    }
}

/// The title of a drawer page, with a way back to the layers and a control on the right.
struct DrawerHeader<Trailing: View>: View {
    let title: String
    var back: (() -> Void)?
    @ViewBuilder var trailing: () -> Trailing

    var body: some View {
        ZStack {
            Text(title)
                .font(.headline)
                .accessibilityAddTraits(.isHeader)
            HStack {
                if let back {
                    Button(action: back) {
                        Image(systemName: "chevron.backward")
                            .font(.body.weight(.semibold))
                            .frame(minWidth: 44, minHeight: 44)
                    }
                    .accessibilityLabel("Layers")
                    .accessibilityIdentifier("map-drawer-back")
                }
                Spacer()
                trailing()
            }
        }
        .padding(.horizontal, 16)
        .padding(.top, 12)
        .padding(.bottom, 4)
    }
}

extension DrawerHeader where Trailing == EmptyView {
    init(title: String, back: (() -> Void)? = nil) {
        self.init(title: title, back: back, trailing: { EmptyView() })
    }
}

/// A module's own section of the drawer.
private struct ModuleSection: Identifiable {
    /// The module's place in drawing order.
    let id: Int
    let view: AnyView
}

/// A base map: chosen by a tap, with its credit and, for the chart, "Not for navigation.".
private struct BaseRow: View {
    let layer: LayerInfo
    let choose: () -> Void

    var body: some View {
        Button(action: choose) {
            VStack(alignment: .leading, spacing: 4) {
                HStack {
                    Text(layer.title)
                        .foregroundStyle(Color.primary)
                    Spacer()
                    if layer.shown {
                        Image(systemName: "checkmark")
                            .foregroundStyle(Palette.accent)
                            .accessibilityHidden(true)
                    }
                }
                if let reason = layer.unavailableText {
                    DrawerFootnote(text: reason)
                }
                if let notice = layer.notice {
                    DrawerNotice(text: notice)
                }
                if !layer.credit.isEmpty {
                    DrawerCredit(text: layer.credit)
                }
            }
        }
        .disabled(!layer.available)
        .accessibilityAddTraits(layer.shown ? .isSelected : [])
        .accessibilityIdentifier("map-base-\(layer.id)")
    }
}

/// An overlay: its switch, or why it cannot be switched on; then, while it is on, its opacity
/// and legend; its notice and credit always.
private struct LayerRow: View {
    let layer: LayerInfo
    let controller: MapController

    private var blocked: Bool {
        DrawerSections.batteryBlocks(
            layer, catalogue: controller.catalogue, battery: controller.battery)
    }

    private var held: Bool { controller.plan.held.contains(layer.id) }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Toggle(
                layer.title,
                isOn: Binding(
                    get: { layer.shown }, set: { controller.setShown(layer.id, $0) })
            )
            .disabled(!layer.available || blocked)
            .accessibilityIdentifier("map-layer-\(layer.id)")
            if let reason = layer.unavailableText {
                DrawerFootnote(text: reason)
            } else if blocked {
                DrawerFootnote(
                    text:
                        "The battery-friendly map draws two image layers at most. Switch one off to show this one."
                )
            } else if held {
                DrawerFootnote(
                    text:
                        "Not drawn now: the battery-friendly map draws only the top two image layers."
                )
            }
            if let notice = layer.notice {
                DrawerNotice(text: notice)
            }
            if layer.shown && DrawerSections.hasOpacity(layer) {
                OpacitySlider(layer: layer, controller: controller)
            }
            if layer.shown {
                let stops = DrawerSections.legend(layer, drawn: controller.legends[layer.id])
                if !stops.isEmpty {
                    DrawerLegend(
                        stops: stops, fitted: controller.legends[layer.id]?.fittedCaption,
                        caption: layer.legendCaption)
                }
                if LayerRange.measurement(for: layer.id) != nil {
                    LayerRangeRow(layer: layer)
                }
            }
            if !layer.credit.isEmpty {
                DrawerCredit(text: layer.credit)
            }
        }
        .padding(.vertical, 2)
    }
}

/// A layer's opacity: the map follows the slider as it moves, and the settings are written once
/// it is let go.
private struct OpacitySlider: View {
    let layer: LayerInfo
    let controller: MapController
    @State private var value: Double = 1
    @State private var dragging = false

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "circle.lefthalf.filled")
                .foregroundStyle(.readableSecondary)
                .accessibilityHidden(true)
            Slider(
                value: $value, in: 0...1,
                onEditingChanged: { editing in
                    dragging = editing
                    if !editing { controller.setOpacity(layer.id, value) }
                })
            .accessibilityLabel("\(layer.title) opacity")
            .accessibilityIdentifier("map-opacity-\(layer.id)")
        }
        .onAppear { value = Double(layer.opacity) }
        .onChange(of: value) { _, new in
            if dragging { controller.previewOpacity(layer.id, new) }
        }
        .onChange(of: layer.opacity) { _, saved in
            if !dragging { value = Double(saved) }
        }
    }
}

/// A grid layer's legend bar, the values at its ends and middle in the user's units, and the
/// core's captions.
struct DrawerLegend: View {
    let stops: [LegendStop]
    var fitted: String?
    var caption: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            LinearGradient(
                stops: stops.map {
                    Gradient.Stop(color: $0.colour.color, location: CGFloat($0.fraction))
                },
                startPoint: .leading, endPoint: .trailing
            )
            .frame(height: 10)
            .clipShape(Capsule())
            .overlay(Capsule().strokeBorder(Color(.separator)))
            .accessibilityHidden(true)
            HStack {
                ForEach(Array(DrawerSections.labelled(stops).enumerated()), id: \.offset) {
                    index, stop in
                    if index > 0 { Spacer(minLength: 4) }
                    Text(stop.label)
                }
            }
            .font(.caption2.monospacedDigit())
            if let fitted {
                DrawerFootnote(text: fitted)
            }
            if let caption {
                DrawerFootnote(text: caption)
            }
        }
        .accessibilityElement(children: .combine)
    }
}

/// "Not for navigation." on a chart, current or depth entry: whole, never in Luna's voice.
struct DrawerNotice: View {
    let text: String

    var body: some View {
        Label {
            Text(text)
                .font(.footnote.weight(.semibold))
                .foregroundStyle(Color.primary)
                .fixedSize(horizontal: false, vertical: true)
        } icon: {
            Image(systemName: "exclamationmark.triangle.fill")
                .foregroundStyle(Palette.warning)
        }
        .accessibilityElement(children: .combine)
    }
}

/// A layer's credit line, whole.
struct DrawerCredit: View {
    let text: String

    var body: some View {
        Text(text)
            .font(.caption2)
            .foregroundStyle(.readableSecondary)
            .fixedSize(horizontal: false, vertical: true)
    }
}

/// A short line under a layer: why it is off, or what its legend means.
struct DrawerFootnote: View {
    let text: String

    var body: some View {
        Text(text)
            .font(.footnote)
            .foregroundStyle(.readableSecondary)
            .fixedSize(horizontal: false, vertical: true)
    }
}

/// The battery-friendly mode: on with Low Power Mode, always or never.
private struct BatterySection: View {
    let mode: String
    let on: Bool
    let set: (String) -> Void

    var body: some View {
        Section {
            Picker(
                "Battery-friendly map",
                selection: Binding(get: { mode }, set: { set($0) })
            ) {
                Text("With Low Power Mode").tag("auto")
                Text("Always").tag("on")
                Text("Never").tag("off")
            }
            .accessibilityIdentifier("map-battery")
        } footer: {
            Text(
                on
                    ? "On now: no animation, and two image layers at most."
                    : "When on: no animation, and two image layers at most."
            )
        }
    }
}
