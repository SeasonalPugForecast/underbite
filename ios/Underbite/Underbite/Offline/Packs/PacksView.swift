import SwiftUI
import UnderbiteKit

/// What the packs screen shows beside (or over) the list.
enum PackSelection: Hashable {
    case pack(String)
    case new
}

/// The offline packs: each with its area, what it holds on the phone, when it was refreshed and
/// whether it needs a refresh; a new pack of the map's view; the Wi-Fi choice and how refreshes
/// happen, in the core's words. On the iPad the list and a pack sit side by side; on the iPhone
/// choosing a pack pushes it.
struct PacksView: View {
    let start: PacksStart
    /// The map's view and layers, for a new pack.
    let mapArea: PackBox?
    let mapLayers: [String]

    @Environment(SpotStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    @Environment(\.horizontalSizeClass) private var sizeClass
    @State private var selection: PackSelection?
    @State private var compactColumn: NavigationSplitViewColumn = .sidebar
    @State private var started = false
    @State private var now = Int64(Date().timeIntervalSince1970)

    private var library: PackLibrary { .shared }

    private var words: PackWords? { try? packWords(settingsJson: store.settingsJSON) }

    var body: some View {
        NavigationSplitView(preferredCompactColumn: $compactColumn) {
            list
        } detail: {
            NavigationStack {
                detail
            }
        }
        .navigationSplitViewStyle(.balanced)
        .onAppear {
            guard !started else { return }
            started = true
            if start == .new {
                selection = .new
            } else if sizeClass == .regular, let first = library.records.first {
                selection = .pack(first.id)
            }
        }
        .onChange(of: selection) { _, chosen in
            compactColumn = chosen == nil ? .sidebar : .detail
        }
        .onChange(of: compactColumn) { _, column in
            if column == .sidebar, sizeClass != .regular { selection = nil }
        }
        .task {
            await library.loadSizes()
            await library.resumeInterrupted(settingsJSON: store.settingsJSON)
        }
        .task {
            // The rows' ages follow the clock.
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(60))
                now = Int64(Date().timeIntervalSince1970)
            }
        }
    }

    private var list: some View {
        List(selection: $selection) {
            if let words {
                Section {
                    ForEach(library.records) { record in
                        PackListRow(record: record, words: words, now: now)
                            .tag(PackSelection.pack(record.id))
                    }
                    .onDelete { offsets in
                        let ids = offsets.map { library.records[$0].id }
                        Task { for id in ids { await library.delete(id) } }
                    }
                    Button {
                        selection = .new
                    } label: {
                        Label("New pack", systemImage: "plus")
                    }
                    .accessibilityIdentifier("packs-new")
                }
                Section {
                    Toggle(words.wifiOnlyTitle, isOn: wifiOnly)
                } footer: {
                    Text(words.wifiOnlyDetail).readableText()
                }
                Section {
                    Text(words.backgroundRefreshNote).font(.footnote).readableText()
                    Text(words.baseMapNote).font(.footnote).readableText()
                    DrawerNotice(text: words.notice)
                }
            }
        }
        .listStyle(.insetGrouped)
        .navigationTitle("Offline packs")
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                Button("Done") { dismiss() }
            }
        }
    }

    @ViewBuilder
    private var detail: some View {
        switch selection {
        case .pack(let id):
            if library.record(id) != nil {
                PackDetailView(id: id, now: now)
            } else {
                empty
            }
        case .new:
            NewPackView(prefill: PackPrefill(area: mapArea, layers: mapLayers)) { id in
                selection = .pack(id)
            }
        case nil:
            empty
        }
    }

    private var empty: some View {
        ContentUnavailableView("Offline packs", systemImage: "arrow.down.square")
    }

    private var wifiOnly: Binding<Bool> {
        Binding(get: { library.wifiOnly }, set: { library.wifiOnly = $0 })
    }
}

/// A pack in the list: its area, name, what it holds and where its download stands.
private struct PackListRow: View {
    let record: PackRecord
    let words: PackWords
    let now: Int64
    @Environment(SpotStore.self) private var store

    private var library: PackLibrary { .shared }

    var body: some View {
        HStack(spacing: 12) {
            if let area = record.info?.area {
                PackThumbnail(area: area)
                    .frame(width: 64, height: 64)
                    .clipShape(RoundedRectangle(cornerRadius: 8))
            }
            VStack(alignment: .leading, spacing: 2) {
                Text(record.info?.name ?? "").font(.headline)
                if let row {
                    Text(row.sizeText).font(.subheadline).readableText()
                    Text(row.refreshedText).font(.subheadline).readableText()
                    Text(row.datesText).font(.subheadline).readableText()
                    if let stale = row.staleText {
                        Label(stale, systemImage: "clock.badge.exclamationmark")
                            .font(.subheadline)
                            .foregroundStyle(Palette.warning)
                    }
                }
                PackStatusLine(record: record, words: words)
            }
        }
        .padding(.vertical, 4)
        .accessibilityElement(children: .combine)
    }

    private var row: PackRow? {
        try? packRow(
            packJson: record.json,
            storedBytes: UInt64(max(0, library.storedBytes[record.id] ?? 0)), now: now,
            settingsJson: store.settingsJSON, timeZone: TimeZone.current.identifier)
    }
}

/// Where a pack's download stands, with its progress while one runs or is stopped.
struct PackStatusLine: View {
    let record: PackRecord
    let words: PackWords
    @Environment(SpotStore.self) private var store

    private var library: PackLibrary { .shared }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            // "Not downloaded yet" and the last refresh are the row's own words.
            if record.status != .done, record.status != .notDownloaded {
                Text(record.status.text(words.status))
                    .font(.subheadline.weight(.semibold))
            }
            if let progress = library.progress(record.id) {
                ProgressView(value: progress.fraction)
                Text(progressText(progress)).font(.footnote).readableText()
                    .monospacedDigit()
            }
            if let problem = record.problem, record.status == .failed {
                Text(problem).font(.footnote).foregroundStyle(Palette.warning)
            }
        }
    }

    private func progressText(_ p: PackProgress) -> String {
        (try? packProgressText(
            done: p.done, total: p.total, failed: p.failed, bytes: p.bytes,
            settingsJson: store.settingsJSON)) ?? ""
    }
}
