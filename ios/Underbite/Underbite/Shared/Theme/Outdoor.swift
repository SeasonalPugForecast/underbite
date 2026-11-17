import SwiftUI
import UIKit

private nonisolated struct SunModeKey: EnvironmentKey {
    static let defaultValue = false
}

extension EnvironmentValues {
    /// Sun mode: high contrast and heavier strokes for reading in bright light. It changes
    /// colours and weights only, never a value or a word. Read by the text styles too, which
    /// resolve off the main actor.
    nonisolated var sunMode: Bool {
        get { self[SunModeKey.self] }
        set { self[SunModeKey.self] = newValue }
    }
}

extension View {
    /// Sun mode below this point when `on`: text in its bold legibility weight, and the flag
    /// the readable text styles and the drawing code read for their colours, strokes and
    /// outlines.
    func sunMode(_ on: Bool) -> some View {
        modifier(SunModeStyle(on: on))
    }
}

private struct SunModeStyle: ViewModifier {
    let on: Bool

    func body(content: Content) -> some View {
        if on {
            content.environment(\.sunMode, true).environment(\.legibilityWeight, .bold)
        } else {
            // The user's own Bold Text setting stays as it is.
            content.environment(\.sunMode, false)
        }
    }
}

/// Secondary text that keeps 4.5:1 on the app's backgrounds, near the primary colour in sun
/// mode. The app's text uses it where the system's secondary would be fainter than that.
nonisolated struct ReadableSecondary: ShapeStyle {
    func resolve(in environment: EnvironmentValues) -> Color {
        // On a selected row's colour, the system's own light text.
        if environment.backgroundProminence == .increased { return Color.white }
        return environment.sunMode ? Color.primary.opacity(0.88) : Palette.secondaryText
    }
}

/// Tertiary marks and text (a chevron, a hint), still 4.5:1; near primary in sun mode.
nonisolated struct ReadableTertiary: ShapeStyle {
    func resolve(in environment: EnvironmentValues) -> Color {
        if environment.backgroundProminence == .increased { return Color.white.opacity(0.92) }
        return environment.sunMode ? Color.primary.opacity(0.8) : Palette.tertiaryText
    }
}

extension View {
    /// The readable styles as this view's secondary and tertiary levels, so the text a list
    /// styles itself (section headers and footers, a row's value) keeps 4.5:1 too. Not for a
    /// list with a selection, whose selected row draws its own text colours.
    func readableText() -> some View {
        foregroundStyle(Color.primary, ReadableSecondary(), ReadableTertiary())
    }
}

extension ShapeStyle where Self == ReadableSecondary {
    static var readableSecondary: ReadableSecondary { ReadableSecondary() }
}

extension ShapeStyle where Self == ReadableTertiary {
    static var readableTertiary: ReadableTertiary { ReadableTertiary() }
}

/// The colours that carry a state. Each one is told apart by a symbol or a word too, never by
/// colour alone, and the pairs that appear together (observed and predicted, a flood stage and
/// the user's range) are blue against orange or red, which colour-blind eyes separate.
nonisolated enum Palette {
    /// Secondary text: the system's secondary label made a little stronger, so small text
    /// keeps 4.5:1 on the grouped backgrounds in light and dark.
    static let secondaryText = Color(
        UIColor { traits in
            traits.userInterfaceStyle == .dark
                ? UIColor(red: 235 / 255, green: 235 / 255, blue: 245 / 255, alpha: 0.75)
                : UIColor(red: 60 / 255, green: 60 / 255, blue: 67 / 255, alpha: 0.85)
        })

    /// Tertiary text (a chevron, a hint): weaker than secondary, still 4.5:1.
    static let tertiaryText = Color(
        UIColor { traits in
            traits.userInterfaceStyle == .dark
                ? UIColor(red: 235 / 255, green: 235 / 255, blue: 245 / 255, alpha: 0.62)
                : UIColor(red: 60 / 255, green: 60 / 255, blue: 67 / 255, alpha: 0.72)
        })

    /// The app's accent: a blue that keeps 4.5:1 for small text on the light backgrounds, the
    /// system blue on dark ones.
    static let accent = Color(
        UIColor { traits in
            traits.userInterfaceStyle == .dark
                ? UIColor(red: 0.039, green: 0.518, blue: 1.0, alpha: 1)
                : UIColor(red: 0.0, green: 0.40, blue: 0.85, alpha: 1)
        })

    /// Warnings in text (why a value is greyed, a failed fetch): an orange dark enough to keep
    /// 4.5:1 on the light cards and the light page, the system orange on dark ones.
    static let warning = Color(
        UIColor { traits in
            traits.userInterfaceStyle == .dark
                ? UIColor(red: 1.0, green: 0.624, blue: 0.039, alpha: 1)
                : UIColor(red: 0.659, green: 0.302, blue: 0.0, alpha: 1)
        })

    /// A value the user typed that cannot be used: a dark red on light backgrounds (6:1), the
    /// system red on dark ones.
    static let error = Color(
        UIColor { traits in
            traits.userInterfaceStyle == .dark
                ? UIColor(red: 1.0, green: 0.271, blue: 0.227, alpha: 1)
                : UIColor(red: 0.75, green: 0.0, blue: 0.0, alpha: 1)
        })

    /// Something done (a location found), always beside a symbol and words: a dark green on
    /// light backgrounds (6:1), the system green on dark ones.
    static let success = Color(
        UIColor { traits in
            traits.userInterfaceStyle == .dark
                ? UIColor(red: 0.188, green: 0.820, blue: 0.345, alpha: 1)
                : UIColor(red: 0.0, green: 0.45, blue: 0.15, alpha: 1)
        })

    /// The outline of a card, drawn in sun mode so its edge holds in glare.
    static func cardStroke(_ sunMode: Bool) -> Color {
        sunMode ? Color.primary.opacity(0.55) : .clear
    }

    /// How much heavier lines are drawn in sun mode.
    static func strokeScale(_ sunMode: Bool) -> CGFloat { sunMode ? 1.75 : 1 }

    /// How much stronger shaded spans are in sun mode.
    static func fillScale(_ sunMode: Bool) -> Double { sunMode ? 1.8 : 1 }
}
