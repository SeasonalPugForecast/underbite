import XCTest

@testable import UnderbiteKit

/// The Privacy page's model: it shows the core's words for the stored rounding, and a choice
/// writes valid settings.
final class PrivacyModelTests: XCTestCase {
    func testTheModelShowsTheCoresSectionsAndChoices() throws {
        let base = defaultSettingsJson()
        let model = PrivacyModel(settingsJSON: base)
        let words = privacyWords(settingsJson: base)
        XCTAssertEqual(model.words.sections.count, 3)
        XCTAssertEqual(model.words.sections.map(\.title), words.sections.map(\.title))
        XCTAssertTrue(model.words.sections.allSatisfy { !$0.title.isEmpty && !$0.paragraphs.isEmpty })
        XCTAssertEqual(model.choices.map(\.decimals), [2, 1, 0])
        XCTAssertEqual(model.choices.map(\.label), words.roundingChoices.map(\.label))
        XCTAssertEqual(model.precision, 2)
        XCTAssertTrue(model.offersCurrent)
        XCTAssertEqual(model.words.roundingCurrent, words.roundingCurrent)
    }

    func testAChoiceChangesTheStoredPrecisionAndTheWords() throws {
        let base = defaultSettingsJson()
        for choice in PrivacyModel(settingsJSON: base).choices {
            let edited = try PrivacyModel.choosing(choice.decimals, in: base)
            let model = PrivacyModel(settingsJSON: edited)
            XCTAssertEqual(try Settings(json: edited).sources.coordinatePrecision, choice.decimals)
            XCTAssertEqual(model.precision, choice.decimals)
            XCTAssertTrue(model.offersCurrent)
        }
        let coarse = PrivacyModel(settingsJSON: try PrivacyModel.choosing(0, in: base))
        XCTAssertNotEqual(coarse.words.roundingCurrent, PrivacyModel(settingsJSON: base).words.roundingCurrent)
    }

    func testAPrecisionFinerThanAllowedIsNotAChoice() throws {
        var settings = try Settings(json: defaultSettingsJson())
        settings.sources.coordinatePrecision = 3
        let model = PrivacyModel(settingsJSON: try settings.json())
        XCTAssertEqual(model.precision, 3)
        XCTAssertFalse(model.offersCurrent)
        XCTAssertFalse(model.words.roundingTooFine.isEmpty)
    }
}
