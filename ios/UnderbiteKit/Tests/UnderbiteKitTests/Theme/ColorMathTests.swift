import Foundation
import XCTest

@testable import UnderbiteKit

/// Colour parsing and the maths the theme checks rest on.
final class ColorMathTests: XCTestCase {
    func testHexParsesAndWritesBack() throws {
        let c = try XCTUnwrap(RGBA(hex: "#0072B2"))
        XCTAssertEqual(c.hex, "#0072B2")
        XCTAssertEqual(c.alpha, 1)
        let t = try XCTUnwrap(RGBA(hex: "5856d638"))
        XCTAssertEqual(t.hex, "#5856D638")
        XCTAssertEqual(t.alpha, 0x38 / 255, accuracy: 1e-9)
        XCTAssertEqual(RGBA(rgb: 0x0072B2), c)
        XCTAssertEqual(RGBA(rgba: 0x5856D638), t)
    }

    func testHexRejectsAnythingElse() {
        for text in ["", "#", "#12345", "#1234567", "#GGGGGG", "red", "#123456789"] {
            XCTAssertNil(RGBA(hex: text), text)
        }
    }

    func testHexClampsOutOfRangeComponents() {
        XCTAssertEqual(RGBA(red: 1.4, green: -0.2, blue: .nan).hex, "#FF0000")
        XCTAssertEqual(RGBA(red: 2, green: 0.5, blue: -1, alpha: 3).clamped, RGBA(red: 1, green: 0.5, blue: 0))
    }

    func testContrastIsWCAGs() {
        XCTAssertEqual(ColorMath.contrast(.white, .black), 21, accuracy: 1e-9)
        XCTAssertEqual(ColorMath.contrast(.black, .white), 21, accuracy: 1e-9)
        XCTAssertEqual(ColorMath.contrast(RGBA(rgb: 0x777777), .white), 4.48, accuracy: 0.01)
    }

    func testOverBlendsByAlpha() {
        let half = RGBA(red: 0, green: 0, blue: 0, alpha: 0.5)
        XCTAssertEqual(ColorMath.over(half, .white), RGBA(red: 0.5, green: 0.5, blue: 0.5))
        XCTAssertEqual(ColorMath.over(RGBA(rgb: 0x123456), .white), RGBA(rgb: 0x123456))
    }

    func testOKLabRoundTripsAndMeasuresDistance() {
        let c = RGBA(rgb: 0xD55E00)
        let back = ColorMath.rgb(ColorMath.oklab(c))
        XCTAssertEqual(back.hex, c.hex)
        XCTAssertEqual(ColorMath.oklab(.white).l, 1, accuracy: 1e-4)
        XCTAssertEqual(ColorMath.oklab(.black).l, 0, accuracy: 1e-9)
        XCTAssertEqual(ColorMath.deltaE(c, c), 0, accuracy: 1e-12)
        XCTAssertGreaterThan(ColorMath.deltaE(.white, .black), 0.99)
    }

    func testSimulationMergesRedAndGreenForDeuteranopes() {
        let red = RGBA(rgb: 0xC80000), green = RGBA(rgb: 0x5A7A00)
        XCTAssertGreaterThan(ColorMath.deltaE(red, green), 0.15)
        XCTAssertLessThan(ColorMath.deltaE(red, green, as: .deutan), ColorMath.deltaE(red, green) / 2)
        // An achromat sees grey.
        XCTAssertEqual(ColorMath.oklab(ColorMath.simulate(red, .achromat)).chroma, 0, accuracy: 1e-6)
    }
}
