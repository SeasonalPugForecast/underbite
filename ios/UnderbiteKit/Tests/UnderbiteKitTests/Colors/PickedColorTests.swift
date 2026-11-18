import CoreGraphics
import Foundation
import XCTest

@testable import UnderbiteKit

/// Picked colours land in sRGB in a theme's 256 steps, whatever space they were picked in, so
/// a theme saves what it paints and reads back the same.
final class PickedColorTests: XCTestCase {
    private func color(_ space: CFString, _ components: [CGFloat]) throws -> CGColor {
        let space = try XCTUnwrap(CGColorSpace(name: space))
        return try XCTUnwrap(CGColor(colorSpace: space, components: components))
    }

    func testAnSRGBPickIsRoundedToTheSavedSteps() throws {
        let picked = try XCTUnwrap(PickedColor.rgba(CGColor(srgbRed: 0.2, green: 0.4, blue: 0.6, alpha: 1)))
        XCTAssertEqual(picked.hex, "#336699")
        XCTAssertEqual(picked, RGBA(rgb: 0x336699))
        let near = try XCTUnwrap(PickedColor.rgba(CGColor(srgbRed: 0.2004, green: 0.3998, blue: 0.6001, alpha: 1)))
        XCTAssertEqual(near, RGBA(rgb: 0x336699))
    }

    func testEveryPresetColourRoundTripsThroughThePicker() throws {
        for preset in ColorPresets.all {
            for (role, color) in preset.colors {
                for c in [color.light] + (color.dark.map { [$0] } ?? []) {
                    let back = try XCTUnwrap(PickedColor.rgba(PickedColor.cgColor(c)), "\(role)")
                    XCTAssertEqual(back, c, "\(preset.id) \(role)")
                }
            }
        }
    }

    func testAWideGamutPickIsMatchedIntoSRGB() throws {
        let p3Red = try XCTUnwrap(PickedColor.rgba(color(CGColorSpace.displayP3, [1, 0, 0, 1])))
        for c in [p3Red.red, p3Red.green, p3Red.blue] { XCTAssertTrue((0...1).contains(c)) }
        XCTAssertEqual(p3Red.red, 1)
        XCTAssertEqual(p3Red, RGBA(hex: p3Red.hex))

        let extended = try XCTUnwrap(PickedColor.rgba(color(CGColorSpace.extendedSRGB, [1.2, -0.1, 0.6, 1])))
        XCTAssertEqual(extended.red, 1)
        XCTAssertEqual(extended.green, 0)
        XCTAssertEqual(extended.hex, "#FF0099")
    }

    func testOpacityAndGreysAreKept() throws {
        let translucent = try XCTUnwrap(
            PickedColor.rgba(CGColor(srgbRed: 0, green: 122.0 / 255, blue: 1, alpha: 77.0 / 255)))
        XCTAssertEqual(translucent.hex, "#007AFF4D")
        let grey = try XCTUnwrap(PickedColor.rgba(color(CGColorSpace.genericGrayGamma2_2, [0.5, 1])))
        XCTAssertEqual(grey.red, grey.green, accuracy: 1.0 / 255)
        XCTAssertEqual(grey.green, grey.blue, accuracy: 1.0 / 255)
        XCTAssertEqual(grey.alpha, 1)
    }

    func testPickingChangesOnlyTheSideAskedFor() throws {
        let color = ThemeColor(light: RGBA(rgb: 0x0066D9), dark: RGBA(rgb: 0x0A84FF))
        let red = CGColor(srgbRed: 1, green: 0, blue: 0, alpha: 1)
        XCTAssertEqual(
            PickedColor.picking(red, dark: true, in: color),
            ThemeColor(light: RGBA(rgb: 0x0066D9), dark: RGBA(rgb: 0xFF0000)))
        XCTAssertEqual(
            PickedColor.picking(red, dark: false, in: color),
            ThemeColor(light: RGBA(rgb: 0xFF0000), dark: RGBA(rgb: 0x0A84FF)))
        XCTAssertNil(PickedColor.picking(PickedColor.cgColor(RGBA(rgb: 0x0A84FF)), dark: true, in: color))
        let single = ThemeColor(light: RGBA(rgb: 0x34435E))
        XCTAssertEqual(
            PickedColor.picking(red, dark: true, in: single),
            ThemeColor(light: RGBA(rgb: 0x34435E), dark: RGBA(rgb: 0xFF0000)))
        let same = ThemeColor(light: RGBA(rgb: 0x34435E), dark: RGBA(rgb: 0x34435E))
        XCTAssertEqual(PickedColor.picking(red, dark: false, in: same), ThemeColor(light: RGBA(rgb: 0xFF0000)))
    }
}
