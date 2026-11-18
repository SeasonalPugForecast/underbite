import SwiftUI
import UnderbiteKit

/// A custom theme's every role, in the editor's groups, each with a colour picker (two where
/// light and dark appearance differ) and a reset button once it differs from the base preset.
/// Above them: what the theme falls short of (`ThemeChecks`), as warnings, never a block; the
/// previews (a tide card, the go / no-go badges, the flow bands) in the theme's colours; and
/// "Preview as" a reader with another colour vision. Every change is saved at once.
struct ThemeEditor: View {
    let model: AppTheme
    /// The custom theme's id.
    let id: String
    @Environment(\.dismiss) private var dismiss
    @State private var preview = PreviewVision.typical
    @State private var confirmRestore = false
    @State private var showsEveryWarning = false

    /// How many warnings show before "Show every warning".
    private static let firstWarnings = 4

    /// "Preview as": the previews and the swatches through a simulation.
    enum PreviewVision: String, CaseIterable, Identifiable {
        case typical, protan, deutan, tritan, noColor
        var id: Self { self }

        var deficiency: Deficiency? {
            switch self {
            case .typical: nil
            case .protan: .protan
            case .deutan: .deutan
            case .tritan: .tritan
            case .noColor: .achromat
            }
        }

        var title: String {
            switch self {
            case .typical: "Typical"
            case .protan: "Protan"
            case .deutan: "Deutan"
            case .tritan: "Tritan"
            case .noColor: "No colour"
            }
        }
    }

    var body: some View {
        let saved = model.custom(id)
        NavigationStack {
            Group {
                if let saved {
                    editor(saved, base: AppTheme.base(of: saved))
                } else {
                    ContentUnavailableView("These colours were deleted", systemImage: "paintpalette")
                }
            }
            .navigationTitle(saved?.name ?? "Colours")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } }
            }
        }
        .task {
            #if DEBUG
                // Screenshots: `-previewAs deutan` (or protan, tritan, noColor).
                if let asked = LaunchArguments.value(after: "-previewAs").flatMap(PreviewVision.init(rawValue:)) {
                    preview = asked
                }
            #endif
        }
    }

    private func editor(_ saved: ColorTheme, base: ColorTheme) -> some View {
        let theme = saved.completed(from: base)
        let shown = preview.deficiency.map { theme.simulated(as: $0) } ?? theme
        let warnings = ThemeCheckLines.lines(theme)
        let failing = ThemeCheckLines.roles(warnings)
        return List {
            // What an edit falls short of comes first, where it is seen; a pass after the previews.
            if !warnings.isEmpty { checks(warnings, base: base) }
            Section {
                ThemePreview(theme: shown)
                    .listRowInsets(EdgeInsets(top: 6, leading: 0, bottom: 6, trailing: 0))
                    .listRowBackground(Color.clear)
                VStack(alignment: .leading, spacing: 6) {
                    Text("Preview as").font(.subheadline).foregroundStyle(.readableSecondary)
                    Picker("Preview as", selection: $preview) {
                        ForEach(PreviewVision.allCases) { Text($0.title).tag($0) }
                    }
                    .pickerStyle(.segmented)
                    .labelsHidden()
                }
            }
            if warnings.isEmpty { checks(warnings, base: base) }
            ForEach(ColorRoleGroup.allCases, id: \.self) { group in
                Section(group.title) {
                    ForEach(ColorRole.roles(in: group), id: \.self) { role in
                        RoleRow(
                            role: role, color: theme[role], differs: saved.differs(role, from: base),
                            warns: failing.contains(role), preview: preview.deficiency,
                            set: { model.set(role, to: $0, in: id) },
                            reset: { model.reset(role, in: id) })
                    }
                }
            }
            Section {
                Button("Restore the defaults", role: .destructive) { confirmRestore = true }
                    .disabled(saved.colors.isEmpty)
            } footer: {
                Text("Every colour goes back to \(base.name)'s.")
            }
        }
        .confirmationDialog(
            "Restore \(base.name)'s colours?", isPresented: $confirmRestore, titleVisibility: .visible
        ) {
            Button("Restore the defaults", role: .destructive) { model.restoreDefaults(id) }
        } message: {
            Text("Every colour you changed in \(saved.name) goes back to \(base.name)'s.")
        }
    }

    /// What the theme falls short of, or that it passes.
    private func checks(_ warnings: [ThemeCheckLine], base: ColorTheme) -> some View {
        Section {
            if warnings.isEmpty {
                CheckLine(
                    passes: true,
                    text: "Passes every check: words, marks and pins stand out on their backgrounds, and the colours drawn together stay apart, for colour-blind eyes too.")
            } else {
                let shown = showsEveryWarning ? warnings : Array(warnings.prefix(Self.firstWarnings))
                ForEach(shown) { CheckLine(passes: false, text: $0.text) }
                if shown.count < warnings.count {
                    Button("Show every warning") { withAnimation { showsEveryWarning = true } }
                }
            }
        } header: {
            Text("Checks")
        } footer: {
            Text(
                base.id == ColorPresets.sunID
                    ? "Colours based on Sun are held to the higher contrast for bright light: words at 7:1, marks at 4.5:1. A warning never stops a change; the roles it is about are marked below."
                    : "Words are held to 4.5:1 and marks to 3:1 against the light and dark backgrounds. A warning never stops a change; the roles it is about are marked below."
            )
        }
    }
}

/// One check: a checkmark when the theme passes, a warning for what it falls short of.
private struct CheckLine: View {
    let passes: Bool
    let text: String

    var body: some View {
        Label {
            Text(text).font(.footnote)
        } icon: {
            Image(systemName: passes ? "checkmark.circle.fill" : "exclamationmark.triangle.fill")
                .foregroundStyle(passes ? Palette.success : Palette.warning)
                .accessibilityLabel(passes ? "Passes" : "Warning")
        }
    }
}

/// One role: its name (marked when a check warns about it), a reset button once it differs
/// from the base, the simulated swatches while previewing, and its colour picker (light, and
/// dark where it has one).
private struct RoleRow: View {
    let role: ColorRole
    /// Its colour in the completed theme.
    let color: ThemeColor
    let differs: Bool
    let warns: Bool
    let preview: Deficiency?
    let set: (ThemeColor) -> Void
    let reset: () -> Void

    var body: some View {
        let hasDark = color.dark.map { $0 != color.light } ?? false
        HStack(spacing: 10) {
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(role.title)
                    if warns {
                        Image(systemName: "exclamationmark.triangle.fill")
                            .font(.footnote)
                            .foregroundStyle(Palette.warning)
                            .accessibilityLabel("Warning")
                    }
                }
                if hasDark {
                    Text("Light and dark appearance").font(.caption).foregroundStyle(.readableSecondary)
                }
            }
            .accessibilityElement(children: .combine)
            Spacer(minLength: 4)
            if differs {
                Button("Reset \(role.title)", systemImage: "arrow.uturn.backward.circle", action: reset)
                    .labelStyle(.iconOnly)
                    .buttonStyle(.borderless)
            }
            if let preview {
                // How the preview's readers see this colour.
                HStack(spacing: 4) {
                    swatch(ColorMath.simulate(color.light, preview))
                    if hasDark, let dark = color.dark { swatch(ColorMath.simulate(dark, preview)) }
                }
                .accessibilityHidden(true)
            }
            ColorPicker(role.title, selection: binding(dark: false), supportsOpacity: color.light.alpha < 1)
                .labelsHidden()
            if hasDark {
                ColorPicker(
                    "\(role.title), dark appearance", selection: binding(dark: true),
                    supportsOpacity: (color.dark?.alpha ?? 1) < 1
                )
                .labelsHidden()
            }
        }
    }

    private func swatch(_ c: RGBA) -> some View {
        RoundedRectangle(cornerRadius: 5)
            .fill(Color(c))
            .overlay { RoundedRectangle(cornerRadius: 5).strokeBorder(.separator, lineWidth: 0.5) }
            .frame(width: 22, height: 22)
    }

    /// The picker's colour: the light or the dark one, in sRGB; a pick is matched into sRGB
    /// and rounded to the steps a theme saves (`PickedColor`).
    private func binding(dark: Bool) -> Binding<CGColor> {
        Binding(
            get: { PickedColor.cgColor(color.resolved(dark: dark)) },
            set: { picked in
                if let changed = PickedColor.picking(picked, dark: dark, in: color) { set(changed) }
            })
    }
}
