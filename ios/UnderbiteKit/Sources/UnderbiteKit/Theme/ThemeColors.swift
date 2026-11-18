import Foundation
import SwiftUI

#if canImport(UIKit)
    import UIKit
#endif

extension Color {
    /// A theme's colour: the one place a SwiftUI colour is made from components.
    public init(_ c: RGBA) {
        self.init(.sRGB, red: c.red, green: c.green, blue: c.blue, opacity: c.alpha)
    }
}

#if canImport(UIKit)
    extension UIColor {
        /// A theme's colour, in sRGB.
        public convenience init(_ c: RGBA) {
            self.init(cgColor: CGColor(srgbRed: c.red, green: c.green, blue: c.blue, alpha: c.alpha))
        }
    }
#endif

extension ThemeColor {
    /// The colour to paint with: the light one, and the dark one in dark appearance.
    public var color: Color {
        guard let dark, dark != light else { return Color(light) }
        #if canImport(UIKit)
            return DynamicColors.kept(self) {
                let l = UIColor(light), d = UIColor(dark)
                return Color(uiColor: UIColor { $0.userInterfaceStyle == .dark ? d : l })
            }
        #else
            return Color(light)
        #endif
    }
}

extension ColorTheme {
    /// The colour of `role`.
    public func color(_ role: ColorRole) -> Color { self[role].color }
}

/// The colours that follow light and dark appearance, made once each: a new `UIColor` provider
/// every time would be a new colour to SwiftUI on every redraw, and the views painted with it
/// would redraw too. Bounded, as a theme being edited makes a new pair at every step.
private enum DynamicColors {
    private static let lock = NSLock()
    nonisolated(unsafe) private static var colors: [ThemeColor: Color] = [:]

    static func kept(_ key: ThemeColor, make: () -> Color) -> Color {
        lock.withLock {
            if let color = colors[key] { return color }
            let color = make()
            if colors.count >= 512 { colors.removeAll() }
            colors[key] = color
            return color
        }
    }
}

private struct ColorThemeKey: EnvironmentKey {
    static let defaultValue = ColorPresets.underbite
}

extension EnvironmentValues {
    /// The colour theme to paint with, resolved once at the root from `ThemeStore` (the app's
    /// window, a widget's entry).
    public var colorTheme: ColorTheme {
        get { self[ColorThemeKey.self] }
        set { self[ColorThemeKey.self] = newValue }
    }
}
