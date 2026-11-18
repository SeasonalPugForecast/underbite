import SwiftUI
import UnderbiteKit

/// Settings › Colours: the presets, each a small sample in its own colours (a tap uses it),
/// the chosen theme's previews, and the reader's own colours: "Customize" makes a custom theme
/// from the chosen preset and opens its editor; custom themes are listed below to use, edit,
/// rename, duplicate or delete. The app paints with the choice at once and the widgets from
/// their next update.
struct ColorsView: View {
    private let model = AppTheme.shared
    @Environment(\.sunMode) private var sunMode
    /// The custom theme open in the editor.
    @State private var editing: EditedTheme?
    /// The custom theme being renamed, and the name typed.
    @State private var renaming: ColorTheme?
    @State private var newName = ""
    /// The screenshot arguments ran (debug builds): once, not on every return to the page.
    @State private var launched = false

    /// A custom theme's id, for the editor sheet.
    struct EditedTheme: Identifiable {
        let id: String
    }

    var body: some View {
        let chosen = model.resolved(sunMode: false)
        Form {
            if sunMode && chosen.base != ColorPresets.sunID {
                Section {
                    Label("Sun mode is on", systemImage: "sun.max")
                } footer: {
                    Text(
                        "Sun mode paints with Sun, for bright light, until it is turned off. Colours customized from Sun are kept in sun mode too."
                    )
                }
            }
            Section {
                LazyVGrid(columns: columns, spacing: 12) {
                    ForEach(ColorPresets.all) { preset in
                        PresetCard(
                            preset: preset, selected: preset.id == model.themeID,
                            customize: { customize(preset.id) }
                        ) {
                            withAnimation(.easeInOut(duration: 0.3)) { model.choose(preset.id) }
                        }
                    }
                }
                .listRowInsets(EdgeInsets(top: 6, leading: 4, bottom: 6, trailing: 4))
                .listRowBackground(Color.clear)
                customizeButton(chosen)
            } header: {
                Text("Presets").foregroundStyle(.readableSecondary)
            } footer: {
                Text(
                    "Each preset paints the colours that carry meaning: go / no-go, river flow and flood stages, station pins, the tide, the timeline and the charts. Customize makes your own colours from a preset (or touch and hold one)."
                )
            }
            if !model.customThemes.isEmpty {
                Section {
                    ForEach(model.customThemes) { theme in
                        customRow(theme)
                    }
                } header: {
                    Text("Your colours").foregroundStyle(.readableSecondary)
                } footer: {
                    Text(
                        "Tap to use. Swipe to rename or delete; touch and hold to edit, rename, duplicate or delete. Colours you didn't change follow the preset they came from."
                    )
                }
            }
            Section {
                ThemePreview(theme: chosen)
                    .listRowInsets(EdgeInsets(top: 6, leading: 0, bottom: 6, trailing: 0))
                    .listRowBackground(Color.clear)
            } header: {
                Text("\(chosen.name) at work").foregroundStyle(.readableSecondary)
            } footer: {
                Text(widgetsNote)
            }
        }
        .navigationTitle("Colours")
        .navigationBarTitleDisplayMode(.inline)
        .task { await screenshotArguments() }
        .sheet(item: $editing) { ThemeEditor(model: model, id: $0.id) }
        .alert(
            "Rename colours",
            isPresented: Binding(get: { renaming != nil }, set: { if !$0 { renaming = nil } })
        ) {
            TextField("Name", text: $newName)
            Button("Rename") {
                if let renaming { model.rename(renaming.id, to: newName) }
            }
            .disabled(CustomThemes.cleanName(newName) == nil)
            Button("Cancel", role: .cancel) {}
        }
    }

    private let columns = [GridItem(.flexible(), spacing: 12), GridItem(.flexible(), spacing: 12)]

    private var widgetsNote: String {
        switch model.backing {
        case .appGroup: "The widgets paint with these colours from their next update."
        case .standard: "This build has no shared space for the widgets, so they keep Underbite's colours."
        }
    }

    /// Below the presets: customize the chosen preset, or edit the chosen custom theme.
    @ViewBuilder private func customizeButton(_ chosen: ColorTheme) -> some View {
        if let custom = model.custom(model.themeID) {
            Button("Edit \(custom.name)…", systemImage: "paintbrush") { editing = EditedTheme(id: custom.id) }
        } else {
            Button("Customize \(chosen.name)…", systemImage: "paintbrush") { customize(chosen.id) }
        }
    }

    /// A custom theme's row: tap to use it; the brush opens its editor.
    private func customRow(_ theme: ColorTheme) -> some View {
        let selected = model.themeID == theme.id
        return HStack(spacing: 12) {
            Button {
                withAnimation(.easeInOut(duration: 0.3)) { model.choose(theme.id) }
            } label: {
                HStack(spacing: 12) {
                    ThemeSwatch(theme: ColorPresets.completed(theme))
                    VStack(alignment: .leading, spacing: 2) {
                        Text(theme.name).foregroundStyle(.primary)
                        Text("From \(AppTheme.base(of: theme).name)")
                            .font(.caption)
                            .foregroundStyle(.readableSecondary)
                    }
                    Spacer(minLength: 4)
                    if selected {
                        Image(systemName: "checkmark").foregroundStyle(.tint).accessibilityHidden(true)
                    }
                }
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .accessibilityAddTraits(selected ? .isSelected : [])
            Button("Edit", systemImage: "paintbrush") { editing = EditedTheme(id: theme.id) }
                .labelStyle(.iconOnly)
                .buttonStyle(.borderless)
        }
        .swipeActions(edge: .trailing) {
            Button("Delete", role: .destructive) { delete(theme) }
            Button("Rename") { startRenaming(theme) }
        }
        .contextMenu {
            Button("Edit…", systemImage: "paintbrush") { editing = EditedTheme(id: theme.id) }
            Button("Rename", systemImage: "pencil") { startRenaming(theme) }
            Button("Duplicate", systemImage: "plus.square.on.square") {
                withAnimation { _ = model.duplicate(theme.id) }
            }
            Button("Delete", systemImage: "trash", role: .destructive) { delete(theme) }
        }
    }

    /// Makes "My <Preset>" from the preset `id`, uses it and opens its editor.
    private func customize(_ id: String) {
        let made = withAnimation(.easeInOut(duration: 0.3)) { model.customize(preset: id) }
        if let made { editing = EditedTheme(id: made.id) }
    }

    private func delete(_ theme: ColorTheme) {
        withAnimation { model.delete(theme.id) }
    }

    private func startRenaming(_ theme: ColorTheme) {
        newName = theme.name
        renaming = theme
    }

    /// Screenshots (debug builds, with a launch-only store, `AppTheme.launchStore`):
    /// `-customTheme <preset>` makes "My <Preset>"; `-editTheme <preset>` also opens its editor
    /// with go painted a pale green and above normal flow painted as normal, so the previews
    /// change and two warnings show.
    private func screenshotArguments() async {
        #if DEBUG
            guard !launched else { return }
            launched = true
            for flag in ["-customTheme", "-editTheme"] {
                guard let preset = LaunchArguments.value(after: flag),
                    let theme = model.customize(preset: preset)
                else { continue }
                if flag == "-editTheme" {
                    let base = AppTheme.base(of: theme)
                    model.set(.verdictGo, to: ThemeColor(light: RGBA(rgb: 0x7FD8B0), dark: RGBA(rgb: 0x7FD8B0)), in: theme.id)
                    model.set(.flowAboveNormal, to: base[.flowNormal], in: theme.id)
                    // Once the page has settled in its column.
                    try? await Task.sleep(for: .milliseconds(600))
                    editing = EditedTheme(id: theme.id)
                }
            }
        #endif
    }
}

/// One preset: its sample as a button that uses it, ringed when chosen; touch and hold to use
/// it or customize it.
private struct PresetCard: View {
    let preset: ColorTheme
    let selected: Bool
    let customize: () -> Void
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            VStack(alignment: .leading, spacing: 8) {
                HStack {
                    Text(preset.name).font(.subheadline.weight(.semibold))
                    Spacer(minLength: 4)
                    if selected {
                        Image(systemName: "checkmark.circle.fill").foregroundStyle(.tint).accessibilityHidden(true)
                    }
                }
                Text(Self.summary(preset.id))
                    .font(.caption)
                    .foregroundStyle(.readableSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 0)
                // The flow classes, then the verdicts, as small bands.
                band(ColorRole.flowClasses)
                band(ColorRole.verdicts)
            }
            .padding(12)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .background(Color(.secondarySystemGroupedBackground), in: .rect(cornerRadius: 14, style: .continuous))
            .overlay {
                RoundedRectangle(cornerRadius: 14, style: .continuous)
                    .strokeBorder(selected ? Color.accentColor : Color.clear, lineWidth: 3)
            }
            .contentShape(.rect(cornerRadius: 14))
        }
        .buttonStyle(.plain)
        .contextMenu {
            Button("Use", systemImage: "checkmark.circle", action: action)
            Button("Customize…", systemImage: "paintbrush", action: customize)
        }
        .accessibilityLabel(Text(preset.name))
        .accessibilityHint(Text(Self.summary(preset.id)))
        .accessibilityAddTraits(selected ? .isSelected : [])
        .accessibilityAction(named: "Customize", customize)
    }

    private func band(_ roles: [ColorRole]) -> some View {
        HStack(spacing: 0) {
            ForEach(roles, id: \.self) { Rectangle().fill(preset.color($0)) }
        }
        .frame(height: 10)
        .clipShape(.capsule)
        .accessibilityHidden(true)
    }

    /// What the preset is for.
    static func summary(_ id: String) -> String {
        switch id {
        case ColorPresets.defaultID:
            "Safe for colour-blind eyes: every scale steps in lightness as well as hue."
        case ColorPresets.sunID: "High contrast for bright light, as sun mode paints."
        default: ""
        }
    }
}
