import CoreGraphics
import Foundation
import UnderbiteKit

/// The colour picker's colours as a theme keeps them. The picker answers in whatever space
/// the reader picked in (Display P3, extended sRGB, grey); a theme stores sRGB in 256 steps
/// (`#RRGGBB`, `#RRGGBBAA`). So a pick is matched into sRGB by ColorSync (a P3 colour outside
/// sRGB lands on its edge), clamped, and rounded to the steps the theme saves, so what is
/// painted is what is saved and read back, and picking the base's colour again matches it.
/// The package tests compile this file too.
nonisolated enum PickedColor {
    /// sRGB, the one space a theme's colours are in.
    private static var sRGB: CGColorSpace? { CGColorSpace(name: CGColorSpace.sRGB) }

    /// A picked colour in sRGB, rounded to a theme's steps; `nil` for one ColorSync cannot
    /// match (a pattern).
    static func rgba(_ picked: CGColor) -> RGBA? {
        guard let sRGB, let matched = picked.converted(to: sRGB, intent: .relativeColorimetric, options: nil),
            let c = matched.components, c.count == 4
        else { return nil }
        let raw = RGBA(red: Double(c[0]), green: Double(c[1]), blue: Double(c[2]), alpha: Double(c[3]))
        return RGBA(hex: raw.clamped.hex)
    }

    /// A theme's colour for the picker, in sRGB.
    static func cgColor(_ c: RGBA) -> CGColor {
        CGColor(srgbRed: c.red, green: c.green, blue: c.blue, alpha: c.alpha)
    }

    /// `color` with its light (`dark == false`) or dark colour set to the pick; `nil` when the
    /// pick is the colour it already has, or cannot be matched. Setting the dark colour of a
    /// colour without one gives it one; the light colour of one whose dark colour is the same
    /// sets both.
    static func picking(_ picked: CGColor, dark: Bool, in color: ThemeColor) -> ThemeColor? {
        guard let rgba = rgba(picked), rgba != color.resolved(dark: dark) else { return nil }
        var out = color
        if dark {
            out.dark = rgba
        } else {
            if out.dark == out.light { out.dark = nil }
            out.light = rgba
        }
        return out
    }
}
