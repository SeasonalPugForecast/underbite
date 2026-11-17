import SwiftUI

/// The first launch's welcome: Luna swimming toward a dock. There is no default spot: the first
/// one is added from where you stand ("Use my location", asked for only when tapped, while the
/// app is in use) or found by search, in the same sheet as everywhere else. `onDone` closes it.
struct WelcomeView: View {
    let onDone: () -> Void
    @Environment(\.voiceStyle) private var voice
    @State private var addsSpot = false

    var body: some View {
        welcome
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
                .foregroundStyle(.readableSecondary)

                VStack(alignment: .leading, spacing: 14) {
                    point("mappin.and.ellipse", "Add the places you fish, or search for a place, drop a pin or type coordinates.")
                    point("slider.horizontal.3", "Choose your sources, units, cards and limits in Settings.")
                    point("lock.shield", "No account and no ads. Your spots stay on your device.")
                }
                .padding(.top, 8)

                Button {
                    addsSpot = true
                } label: {
                    Label("Add your first spot", systemImage: "location")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
                .padding(.top, 12)
                Text("Use my location, or search for a place, drop a pin or type coordinates.")
                    .font(.footnote)
                    .foregroundStyle(.readableSecondary)
                    .multilineTextAlignment(.center)

                Button(action: onDone) {
                    Text("Not now")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.bordered)
                .controlSize(.large)
            }
            .padding(.horizontal, 32)
            .padding(.bottom, 32)
            .frame(maxWidth: 560)
            .frame(maxWidth: .infinity)
        }
        // The whole welcome in the iPad's sheet, not a form sheet that cuts the buttons off.
        .presentationSizing(.page)
        .sheet(isPresented: $addsSpot) {
            AddSpotView { _ in onDone() }
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
