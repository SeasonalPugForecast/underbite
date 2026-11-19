import Foundation
import UnderbiteKit

/// What the Privacy page shows for one set of settings: the core's words and the rounding
/// choices, and the edit that picks one. Foundation only, so the package tests compile it too.
/// Swift words nothing here; every text comes from `privacyWords`.
nonisolated struct PrivacyModel {
    let words: PrivacyWords
    /// The stored rounding, in decimals; the finest allowed when the settings cannot be read.
    let precision: Int

    init(settingsJSON: String) {
        words = privacyWords(settingsJson: settingsJSON)
        precision = (try? Settings(json: settingsJSON).sources.coordinatePrecision) ?? 2
    }

    /// The choices, finest first, as the core words them.
    var choices: [(decimals: Int, label: String)] {
        words.roundingChoices.map { (Int($0.decimals), $0.label) }
    }

    /// Whether the stored rounding is one of the choices; if not, the page shows the core's
    /// "too fine" label for it so the picker still has a selection.
    var offersCurrent: Bool { choices.contains { $0.decimals == precision } }

    /// The edit that sets the rounding to `decimals`: what the page's picker applies through
    /// the store, and what `choosing` applies to a document.
    static func setting(_ decimals: Int) -> (inout Settings) -> Void {
        { $0.sources.coordinatePrecision = decimals }
    }

    /// `json` with the rounding set to `decimals`, or the reason it is not valid.
    static func choosing(_ decimals: Int, in json: String) throws -> String {
        try Preferences.applying(json, setting(decimals))
    }
}
