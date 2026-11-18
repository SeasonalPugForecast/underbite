#if DEBUG
    import Foundation
    @preconcurrency import UnderbiteKit

    /// The sample rule of screenshots (`-seedAlerts`): the Croton River under 800 cfs and under
    /// 66 °F, with quiet hours at night. `-alertRule <id>` opens a rule's editor.
    nonisolated enum AlertSeeds {
        /// The sample rule's id: seeding again replaces it.
        static let id = "5EED0000-0000-4000-8000-0000000000A1"

        static var wanted: Bool { ProcessInfo.processInfo.arguments.contains("-seedAlerts") }

        /// The rule `-alertRule` asks to open, the first time it is asked for: coming back to
        /// the list does not open it again.
        @MainActor
        static func requestedRule() -> String? {
            guard !opened else { return nil }
            let all = ProcessInfo.processInfo.arguments
            guard let index = all.firstIndex(of: "-alertRule"), index + 1 < all.count else {
                return nil
            }
            opened = true
            return all[index + 1]
        }

        @MainActor private static var opened = false

        /// 800 cfs and 66 °F in the core's canonical units (m³/s, °C).
        static let flow = 800 * 0.028316846592
        static let temperature = (66.0 - 32) * 5 / 9

        static var rule: AlertRuleDocument {
            AlertRuleDocument(
                id: id, name: "Croton low and cool", target: .spot(SeedSpots.croton.id),
                allOf: [
                    AlertRuleDocument.Condition(kind: "measurement", metric: "discharge", below: flow),
                    AlertRuleDocument.Condition(
                        kind: "measurement", metric: "water_temperature", below: temperature),
                ],
                quietHours: AlertRuleDocument.QuietHours(startMinute: 22 * 60, endMinute: 6 * 60))
        }

        /// Adds the Croton River when it is missing, and the sample rule on it.
        @MainActor
        static func seed(into store: SpotStore) {
            if store.spot(SeedSpots.croton.id) == nil { store.add(SeedSpots.croton) }
            guard let rule = try? rule.json(),
                let json = try? putAlertRule(settingsJson: store.settingsJSON, ruleJson: rule)
            else { return }
            try? store.updateSettings(json)
        }
    }
#endif
