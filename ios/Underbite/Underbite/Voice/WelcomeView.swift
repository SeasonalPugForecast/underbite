import SwiftUI

/// The first launch's welcome: Luna swimming toward a dock. It asks for nothing: location
/// permission comes with the action that needs it (adding a spot from where you stand).
/// `onDone` closes it.
struct WelcomeView: View {
    let onDone: () -> Void
    @Environment(\.voiceStyle) private var voice

    var body: some View {
        #if DEBUG
            // Screenshot runs: `-about` shows the About screen in place of the welcome.
            if ProcessInfo.processInfo.arguments.contains("-about") {
                NavigationStack {
                    AboutView()
                        .toolbar {
                            ToolbarItem(placement: .confirmationAction) {
                                Button("Done", action: onDone)
                            }
                        }
                }
            } else {
                welcome
            }
        #else
            welcome
        #endif
    }

    private var welcome: some View {
        ScrollView {
            VStack(spacing: 20) {
                LunaArt(picture: .dock, maxWidth: 420)
                    .padding(.top, 24)
                Text("Underbite")
                    .font(.largeTitle.bold())
                Text(
                    voice.pick(
                        luna: "Luna is swimming for the dock, and she is bringing the water report.",
                        plain: "Tides, currents, river flow, water temperature and weather for the places you fish.")
                )
                .font(.title3)
                .multilineTextAlignment(.center)
                .foregroundStyle(.secondary)

                VStack(alignment: .leading, spacing: 14) {
                    point("mappin.and.ellipse", "Add the places you fish, or search for a place, drop a pin or type coordinates.")
                    point("slider.horizontal.3", "Choose your sources, units, cards and limits in Settings.")
                    point("lock.shield", "No account and no ads. Your spots stay on your device.")
                }
                .padding(.top, 8)

                Button(action: onDone) {
                    Text("Get started")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
                .padding(.top, 12)
            }
            .padding(.horizontal, 32)
            .padding(.bottom, 32)
            .frame(maxWidth: 560)
            .frame(maxWidth: .infinity)
        }
    }

    private func point(_ symbol: String, _ text: LocalizedStringKey) -> some View {
        HStack(alignment: .top, spacing: 14) {
            Image(systemName: symbol)
                .font(.title3)
                .foregroundStyle(.tint)
                .frame(width: 32)
                .accessibilityHidden(true)
            Text(text)
        }
    }
}
