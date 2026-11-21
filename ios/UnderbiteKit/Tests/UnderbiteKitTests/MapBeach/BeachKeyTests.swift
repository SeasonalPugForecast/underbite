import XCTest

@testable import UnderbiteKit

/// The map's compact key of the beach forecast colours: when it shows and which entries.
final class BeachKeyTests: XCTestCase {
    private let legend: [(role: String, label: String)] = [
        (role: "low", label: "Low"), (role: "moderate", label: "Moderate"),
        (role: "high", label: "High"), (role: "none", label: "No forecast"),
    ]

    func testTheKeyShowsTheCoresWordsInOrderWhileTheLayerIsOn() {
        let entries = BeachKeyModel.entries(layerOn: true, legend: legend)
        XCTAssertEqual(entries.map(\.label), ["Low", "Moderate", "High", "No forecast"])
        XCTAssertEqual(entries.map(\.role), [.low, .moderate, .high, .none])
    }

    func testTheKeyIsEmptyWhileTheLayerIsOff() {
        XCTAssertTrue(BeachKeyModel.entries(layerOn: false, legend: legend).isEmpty)
    }

    func testTheKeyIsEmptyUntilTheCoreHasGivenItsWords() {
        XCTAssertTrue(BeachKeyModel.entries(layerOn: true, legend: []).isEmpty)
    }

    func testARepeatedColourAndAnEmptyWordAreLeftOut() {
        let entries = BeachKeyModel.entries(
            layerOn: true,
            legend: [
                (role: "low", label: "Low"), (role: "low", label: "Again"),
                (role: "high", label: ""),
            ])
        XCTAssertEqual(entries.map(\.label), ["Low"])
    }
}
