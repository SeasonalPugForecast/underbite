import SwiftUI
import UnderbiteKit

/// One picker per measurement, with the choices and the unit in use from the core. Groups that
/// derive from another (tides, waves, wind...) can follow it or have a unit of their own.
/// Changing a unit changes labels only: stored ranges and limits stay as they are.
struct UnitsPage: View {
    var body: some View {
        PreferencePage(title: "Units") { access in
            let groups = (try? unitOptions(settingsJson: access.json)) ?? []
            Section {
                ForEach(groups.filter { $0.follows == nil }, id: \.id) { group in
                    picker(group, groups: groups, access: access)
                }
            } footer: {
                Text("Values you have saved, such as limits and river ranges, keep their size: only how they are written changes.")
            }
            Section {
                ForEach(groups.filter { $0.follows != nil }, id: \.id) { group in
                    picker(group, groups: groups, access: access)
                }
            } header: {
                Text("Different from the group they belong to")
            }
        }
    }

    private func picker(_ group: UnitGroup, groups: [UnitGroup], access: SettingsAccess) -> some View {
        return Picker(
            group.label,
            selection: Binding<String>(
                get: { group.follows == nil || group.overridden ? group.selected : "" },
                set: { choice in
                    access.edit {
                        Preferences.setUnit(choice.isEmpty ? nil : choice, for: group.id, in: &$0.units)
                    }
                })
        ) {
            if let sameAs = group.sameAsLabel {
                Text(sameAs).tag("")
            }
            ForEach(group.options, id: \.id) { option in
                Text(option.label).tag(option.id)
            }
        }
    }
}
