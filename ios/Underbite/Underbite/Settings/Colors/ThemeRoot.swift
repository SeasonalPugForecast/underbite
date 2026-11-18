import SwiftUI
import UnderbiteKit

extension View {
    /// The colour theme below this point (`EnvironmentValues.colorTheme`): the chosen one, or
    /// the sun preset in sun mode (`AppTheme.resolved`), redrawn when the choice or a custom
    /// theme changes. In debug builds it also starts the colour-vision simulation a launch asks
    /// for (`-simulate deutan`).
    func paintsWithTheme(sunMode: Bool) -> some View {
        modifier(ThemeRoot(sunMode: sunMode))
    }
}

private struct ThemeRoot: ViewModifier {
    let sunMode: Bool

    func body(content: Content) -> some View {
        content
            .environment(\.colorTheme, AppTheme.shared.resolved(sunMode: sunMode))
            #if DEBUG
                .task { ColorVisionSimulation.startIfAsked() }
            #endif
    }
}
