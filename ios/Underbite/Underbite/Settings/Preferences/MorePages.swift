import SwiftUI
import UnderbiteKit

/// Trout stress threshold and the solunar switch.
struct FishingPage: View {
    var body: some View {
        PreferencePage(title: "Trout and fishing windows") { access in
            if let settings = access.settings {
                Section {
                    QuantityRow(
                        title: "Trout stress above", quantity: Preferences.troutStress,
                        settingsJSON: access.json, value: settings.thresholds.troutStressC,
                        optional: false
                    ) { value in
                        if let value { access.edit { $0.thresholds.troutStressC = value } }
                    }
                    Button("Use the usual threshold") {
                        access.edit { $0.thresholds.troutStressC = Preferences.defaultTroutStressC }
                    }
                    .disabled(settings.thresholds.troutStressC == Preferences.defaultTroutStressC)
                } footer: {
                    Text("Above this water temperature the water temperature card warns that trout are stressed. The usual threshold is \(Preferences.Entry(Preferences.troutStress, settingsJSON: access.json).label(of: Preferences.defaultTroutStressC)).")
                }
                Section {
                    Toggle(
                        "Show solunar times",
                        isOn: access.binding({ $0.windows.showSolunar }, fallback: true) {
                            $0.windows.showSolunar = $1
                        })
                } footer: {
                    Text("Solunar major and minor periods, from the sun and moon. Left out of the fishing windows when off.")
                }
            }
        }
    }
}

/// How old a reading may be and how far away a station may be, per kind of data.
struct FreshnessPage: View {
    var body: some View {
        PreferencePage(title: "Freshness and distance") { access in
            if let settings = access.settings {
                ForEach(Preferences.SourceKind.allCases) { kind in
                    let group = kind.group(in: settings.sources)
                    Section(kind.title(settingsJSON: access.json)) {
                        Picker(
                            "Oldest reading",
                            selection: Binding<Int>(
                                get: { group.maxAgeMinutes ?? 0 },
                                set: { minutes in
                                    access.edit {
                                        var changed = kind.group(in: $0.sources)
                                        changed.maxAgeMinutes = minutes == 0 ? nil : minutes
                                        kind.set(changed, in: &$0.sources)
                                    }
                                })
                        ) {
                            Text("Any age").tag(0)
                            ForEach(choices(including: group.maxAgeMinutes), id: \.self) { minutes in
                                Text(Preferences.duration(minutes: minutes, settingsJSON: access.json))
                                    .tag(minutes)
                            }
                        }
                        QuantityRow(
                            title: "Farthest station", quantity: Preferences.distance(of: kind),
                            settingsJSON: access.json, value: group.maxDistanceKm,
                            offLabel: "Any distance"
                        ) { value in
                            access.edit {
                                var changed = kind.group(in: $0.sources)
                                changed.maxDistanceKm = value
                                kind.set(changed, in: &$0.sources)
                            }
                        }
                    }
                }
            }
        }
    }

    /// The usual choices, with a stored age that is not one of them.
    private func choices(including stored: Int?) -> [Int] {
        var all = Preferences.freshnessChoices
        if let stored, stored > 0, !all.contains(stored) { all = (all + [stored]).sorted() }
        return all
    }
}

/// How often the data refreshes: an app preference, not part of the settings document.
struct RefreshPage: View {
    @Environment(SpotStore.self) private var store

    var body: some View {
        Form {
            Section {
                Picker(
                    "Refresh every",
                    selection: Binding(
                        get: { store.refreshMinutes }, set: { store.setRefreshMinutes($0) })
                ) {
                    ForEach(RefreshPolicy.choices, id: \.self) { minutes in
                        Text(Preferences.duration(minutes: minutes, settingsJSON: store.settingsJSON))
                            .tag(minutes)
                    }
                }
                .pickerStyle(.inline)
            } footer: {
                Text("Data is fetched when the app opens and older than about two thirds of this, and again while it is open. Low Power Mode waits longer.")
            }
        }
        .readableText()
        .navigationTitle("Refresh")
        .navigationBarTitleDisplayMode(.inline)
    }
}

/// Luna's voice or plain words. Values, alerts and safety text are plain either way.
struct VoicePage: View {
    var body: some View {
        PreferencePage(title: "Voice") { access in
            Section {
                Picker(
                    "Voice",
                    selection: access.binding({ $0.presentation.voice }, fallback: "luna") {
                        $0.presentation.voice = $1
                    }
                ) {
                    Text("Luna").tag("luna")
                    Text("Plain").tag("plain")
                }
                .pickerStyle(.inline)
                .labelsHidden()
            } footer: {
                Text("Luna adds a little personality to captions and empty screens. Safety text, alerts and the \"not for navigation\" notes are always plain.")
            }
        }
    }
}

/// Sun mode: a display preference of this device, kept with the refresh interval.
struct SunModePage: View {
    @Environment(SpotStore.self) private var store

    var body: some View {
        Form {
            Section {
                Toggle(
                    "Sun mode",
                    isOn: Binding(get: { store.sunMode }, set: { store.setSunMode($0) }))
            } footer: {
                Text("Higher contrast, bolder text and heavier lines and outlines, for reading the screen in bright sun. Values and words stay the same.")
            }
        }
        .readableText()
        .navigationTitle("Sun mode")
        .navigationBarTitleDisplayMode(.inline)
    }
}

/// What stays on the phone, what leaves it, and how exactly a spot's place is sent to the
/// data providers. Every text is the core's.
struct PrivacyPage: View {
    var body: some View {
        ScrollViewReader { proxy in
            page
                #if DEBUG
                .task {
                    // Screenshots only: open scrolled to the rounding choice.
                    if ProcessInfo.processInfo.arguments.contains("-scrollToRounding") {
                        try? await Task.sleep(for: .seconds(2))
                        proxy.scrollTo("rounding", anchor: .top)
                    }
                }
                #endif
        }
    }

    private var page: some View {
        PreferencePage(title: "Privacy") { access in
            let model = PrivacyModel(settingsJSON: access.json)
            ForEach(Array(model.words.sections.enumerated()), id: \.offset) { _, section in
                Section {
                    ForEach(Array(section.paragraphs.enumerated()), id: \.offset) { _, paragraph in
                        Text(paragraph)
                    }
                } header: {
                    Text(section.title)
                        .font(.headline)
                        .textCase(nil)
                        .accessibilityAddTraits(.isHeader)
                }
            }
            Section {
                LabeledContent(model.words.roundingTitle, value: model.words.roundingCurrent)
                Picker(
                    model.words.roundingTitle,
                    selection: access.binding({ $0.sources.coordinatePrecision }, fallback: 2) {
                        $0.sources.coordinatePrecision = $1
                    }
                ) {
                    ForEach(model.choices, id: \.decimals) { choice in
                        Text(choice.label).tag(choice.decimals)
                    }
                    if !model.offersCurrent {
                        Text(model.words.roundingTooFine).tag(model.precision)
                    }
                }
                .pickerStyle(.inline)
                .labelsHidden()
            } header: {
                Text(model.words.roundingTitle).textCase(nil)
            } footer: {
                Text(model.words.roundingFooter)
            }
            .id("rounding")
        }
    }
}
