import SwiftUI

/// The voice setting: Luna's playful words or plain ones. Only the app's own empty and loading
/// copy follows it; values, condition phrases and snack times come from the core's view model,
/// and safety text is plain in both.
enum VoiceStyle: String, Sendable {
    case luna, plain

    /// From the settings' `presentation.voice` (`luna` or `plain`); anything else is Luna, the default.
    init(setting: String) {
        self = VoiceStyle(rawValue: setting) ?? .luna
    }

    /// `luna` in Luna's voice, `plain` otherwise.
    func pick(luna: String, plain: String) -> String {
        self == .luna ? luna : plain
    }
}

extension EnvironmentValues {
    /// The voice the screens below this point speak in.
    @Entry var voiceStyle: VoiceStyle = .luna
}

/// The placeholder pictures of Luna. Decorative: the words beside them carry the meaning.
enum LunaPicture: String {
    case paddling = "LunaPaddling"
    case dock = "LunaDock"
}

/// A picture of Luna sized to the readable width, not to the screen.
struct LunaArt: View {
    let picture: LunaPicture
    var maxWidth: CGFloat = 360

    var body: some View {
        Image(picture.rawValue)
            .resizable()
            .scaledToFit()
            .frame(maxWidth: maxWidth)
            .accessibilityHidden(true)
    }
}
