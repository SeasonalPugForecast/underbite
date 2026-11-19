import SwiftUI
import UnderbiteKit

/// Settings, Storage: what the offline store holds against the limit, each pack's share and the
/// map cache's, the limit (`map.offline_max_mb`) and clearing the map cache. Every size and word
/// is the core's. Clearing asks first and drops only the map cache: packs and the spots' kept
/// data stay.
struct StoragePage: View {
    @Environment(SpotStore.self) private var store
    @State private var model = StorageModel()
    @State private var library = PackLibrary.shared
    @State private var confirmsClear = false
    @State private var problem: String?

    var body: some View {
        List {
            if let figures = model.figures {
                totals(figures)
                packs(figures)
                limit(figures)
                cache(figures)
            }
        }
        .listStyle(.insetGrouped)
        .navigationTitle("Storage")
        .overlay {
            if model.figures == nil { ProgressView() }
        }
        .task(id: store.settingsJSON) {
            #if DEBUG
                PacksLaunch.seedIfAsked(settingsJSON: store.settingsJSON, library: library)
            #endif
            await reload()
        }
        // A download running while the page shows changes its pack's share.
        .onChange(of: library.live.count) { Task { await reload() } }
    }

    private var packList: [(id: String, json: String)] {
        library.records.map { ($0.id, $0.json) }
    }

    private func reload() async {
        await model.load(settingsJSON: store.settingsJSON, packs: packList)
    }

    private func totals(_ figures: StorageFigures) -> some View {
        Section {
            VStack(alignment: .leading, spacing: 8) {
                Text(figures.words.totalText)
                    .font(.title3.bold())
                if let fill = figures.fill {
                    ProgressView(value: fill)
                        .accessibilityHidden(true)
                }
                Text(figures.words.packsText)
                Text(figures.words.cacheText)
            }
            .padding(.vertical, 4)
            .accessibilityElement(children: .combine)
        }
    }

    @ViewBuilder
    private func packs(_ figures: StorageFigures) -> some View {
        if !figures.packs.isEmpty {
            Section {
                ForEach(figures.packs) { pack in
                    LabeledContent {
                        Text(pack.sizeText)
                    } label: {
                        Text(pack.name)
                        Text(pack.refreshedText)
                    }
                }
            } header: {
                Text("Offline packs").foregroundStyle(.readableSecondary)
            }
        }
    }

    private func limit(_ figures: StorageFigures) -> some View {
        Section {
            Picker(
                "Limit",
                selection: Binding(
                    get: { figures.limitMegabytes },
                    set: { setLimit($0) })
            ) {
                ForEach(figures.limits) { choice in
                    Text(choice.label).tag(choice.megabytes)
                }
            }
            if let problem {
                Text(problem)
                    .font(.footnote)
                    .foregroundStyle(Palette.warning)
            }
        } footer: {
            Text(figures.words.limitText)
        }
    }

    private func cache(_ figures: StorageFigures) -> some View {
        Section {
            Button("Clear map cache", role: .destructive) {
                confirmsClear = true
            }
            .disabled(model.clearing)
            .confirmationDialog(
                figures.words.cacheText, isPresented: $confirmsClear, titleVisibility: .visible
            ) {
                Button("Clear map cache", role: .destructive) {
                    Task {
                        await model.clearCache(settingsJSON: store.settingsJSON, packs: packList)
                    }
                }
            }
        }
    }

    private func setLimit(_ megabytes: Int) {
        problem = store.edit { $0.map.offlineMaxMb = megabytes }
        guard problem == nil else { return }
        Task {
            await model.bound(to: megabytes, settingsJSON: store.settingsJSON, packs: packList)
        }
    }
}
