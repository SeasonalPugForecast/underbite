import XCTest

@testable import UnderbiteKit

final class GoNoGoReasonsTests: XCTestCase {
    private func reason(_ text: String) -> ViewModel.GoNoGoReasonVm {
        ViewModel.GoNoGoReasonVm(check: "waves", kind: "exceeded", text: text, link: nil)
    }

    func testTheReasonTheVerdictSaysIsNotListedAgain() {
        let listed = GoNoGoReasons.listed(
            [reason("waves to 2.0 ft, your limit is 2.0 ft"), reason("gusts to 20 kn")],
            verdictLine: "waves to 2.0 ft, your limit is 2.0 ft ")
        XCTAssertEqual(listed.map(\.text), ["gusts to 20 kn"])
    }

    func testAReasonThatDiffersFromTheVerdictLineStays() {
        let listed = GoNoGoReasons.listed(
            [reason("waves to 2.0 ft")], verdictLine: "Not great: waves to 2.0 ft")
        XCTAssertEqual(listed.count, 1)
    }
}
