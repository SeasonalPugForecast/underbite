import PhotosUI
import SwiftUI
import UnderbiteKit

/// About: the name's story, a dedication with an optional photo, the version, and the
/// not-for-navigation statement. Pushed from Settings.
struct AboutView: View {
    @State private var photo: UIImage? = AboutPhotoStore.load()
    @State private var pickerItem: PhotosPickerItem?
    @State private var photoFailed = false

    var body: some View {
        List {
            Section {
                VStack(spacing: 8) {
                    LunaArt(picture: .paddling, maxWidth: 260)
                    Text("Underbite")
                        .font(.title.bold())
                    Text("Version \(appVersion), core \(coreVersion())")
                        .font(.footnote)
                        .foregroundStyle(.readableSecondary)
                }
                .frame(maxWidth: .infinity)
                .accessibilityElement(children: .combine)
            }

            Section("In memory") {
                // Placeholder dedication: replace before release. The brackets
                // keep it visibly unfinished until then.
                Text(verbatim: "[Dedication placeholder: a few words about Luna, a bulldog who loved swimming and watching people, and snacks, most of all sashimi.]")
                photoRow
            }

            Section("Not for navigation") {
                Text("Underbite is not for navigation. Tides, currents, depths, charts and every other value it shows are for planning a day on the water only. They can be wrong, late or missing. Never rely on them to steer a vessel or to judge a hazard, and always follow official charts, notices and local regulations.")
            }

            Section("Regulations") {
                Text("Underbite does not tell you what is legal. Seasons, sizes, limits, closures and licences are links to the agencies' own pages, never data held here. Check them before you fish.")
            }

            Section("Privacy") {
                Text("There is no account, no server of ours and no analytics. Spot coordinates leave your device only, rounded, to the data providers you choose, or in what you share yourself. The photo above stays in the app on this device.")
                NavigationLink("Privacy settings") { PrivacyPage() }
            }

            Section("Sources") {
                NavigationLink("Attribution") { AttributionView() }
                NavigationLink("Acknowledgements") { AcknowledgementsView() }
            }
        }
        .readableText()
        .navigationTitle("About")
        .navigationBarTitleDisplayMode(.inline)
        .onChange(of: pickerItem) { _, item in
            guard let item else { return }
            Task { await choose(item) }
        }
        .alert("The photo could not be saved", isPresented: $photoFailed) {
            Button("OK", role: .cancel) {}
        }
    }

    @ViewBuilder
    private var photoRow: some View {
        VStack(spacing: 12) {
            if let photo {
                Image(uiImage: photo)
                    .resizable()
                    .scaledToFit()
                    .frame(maxWidth: 320)
                    .clipShape(RoundedRectangle(cornerRadius: 12))
                    .accessibilityLabel("Your photo")
            } else {
                RoundedRectangle(cornerRadius: 12)
                    .fill(.quaternary)
                    .frame(maxWidth: 320)
                    .aspectRatio(4 / 3, contentMode: .fit)
                    .overlay {
                        Image(systemName: "pawprint")
                            .font(.largeTitle)
                            .foregroundStyle(.readableSecondary)
                    }
                    .accessibilityHidden(true)
            }
            PhotosPicker(
                photo == nil ? "Choose a photo" : "Change the photo",
                selection: $pickerItem, matching: .images)
            if photo != nil {
                Button("Remove the photo", role: .destructive) {
                    AboutPhotoStore.remove()
                    photo = nil
                    pickerItem = nil
                }
            }
            Text("Optional. The photo stays on this device and is never shared or uploaded.")
                .font(.footnote)
                .foregroundStyle(.readableSecondary)
                .multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity)
    }

    private func choose(_ item: PhotosPickerItem) async {
        do {
            guard let data = try await item.loadTransferable(type: Data.self) else {
                photoFailed = true
                return
            }
            try AboutPhotoStore.save(data)
            photo = AboutPhotoStore.load()
        } catch {
            photoFailed = true
        }
    }

    private var appVersion: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0"
    }
}
