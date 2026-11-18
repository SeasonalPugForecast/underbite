import SwiftUI
@preconcurrency import UnderbiteKit

/// What the Log's detail column shows.
enum LogSelection: Hashable {
    case insights
    case entry(String)
}

/// The catch editor the Log shows: a new catch, or one to edit.
enum CatchEditorSheet: Identifiable, Hashable {
    case new
    case edit(String)

    var id: Self { self }
}

/// The catch log: the catches newest first with their filters, and "What worked". A split view:
/// the list beside the chosen catch on the iPad, a stack on the iPhone and in a narrow iPad
/// window. The selection is the catch's id, so it survives the collapse and back. The editor is
/// a sheet (a form sheet on the iPad).
struct LogTab: View {
    @Environment(SpotStore.self) private var spots
    @Environment(\.horizontalSizeClass) private var sizeClass
    var catches: CatchStore = .shared
    @State private var selection: LogSelection?
    @State private var columns: NavigationSplitViewVisibility = .all
    @State private var compactColumn: NavigationSplitViewColumn = .sidebar
    @State private var filter = CatchListFilter()
    @State private var editor: CatchEditorSheet?

    var body: some View {
        NavigationSplitView(columnVisibility: $columns, preferredCompactColumn: $compactColumn) {
            CatchListColumn(
                catches: catches, selection: $selection, filter: $filter, editor: $editor)
        } detail: {
            NavigationStack {
                switch selection {
                case .insights:
                    InsightsView()
                case .entry(let id):
                    CatchDetailScreen(catches: catches, id: id) { editor = .edit(id) }.id(id)
                case nil where catches.list?.rows.isEmpty ?? true:
                    // The list column says what to do with an empty log.
                    Color.clear
                case nil:
                    ContentUnavailableView(
                        "Choose a catch", systemImage: "fish",
                        description: Text("Its size, its place and the conditions at the time show here."))
                }
            }
        }
        .navigationSplitViewStyle(.balanced)
        // The list follows the log, the settings (units, voice, clock) and the spots' names.
        .task(id: "\(catches.revision) \(spots.settingsJSON.hashValue) \(places.hashValue)") {
            await catches.rebuild(settingsJSON: spots.settingsJSON, places: places)
        }
        .onAppear {
            if sizeClass == .regular, selection == nil { selection = firstShown }
            #if DEBUG
                LogLaunch.apply(open: { selection = .entry($0) }) {
                    editor = $0
                }
            #endif
        }
        .onChange(of: sizeClass) { _, now in
            if now == .regular, selection == nil { selection = firstShown }
        }
        .onChange(of: catches.list) { _, list in
            let rows = list?.rows ?? []
            filter = filter.pruned(for: rows)
            // A catch deleted while it is open leaves the detail column with nothing.
            if case .entry(let id) = selection, !rows.contains(where: { $0.id == id }) {
                selection = nil
            }
            if sizeClass == .regular, selection == nil { selection = firstShown }
        }
        .onChange(of: selection) { _, chosen in
            compactColumn = chosen == nil ? .sidebar : .detail
        }
        .sheet(item: $editor) { sheet in
            NavigationStack {
                switch sheet {
                case .new:
                    CatchEditorView(catches: catches, catchID: nil) { selection = .entry($0) }
                case .edit(let id):
                    CatchEditorView(catches: catches, catchID: id) { selection = .entry($0) }
                }
            }
            .presentationSizing(.form)
        }
    }

    /// The saved spots' names, for the catches filed under them.
    private var places: [PlaceName] {
        spots.spots.map { PlaceName(spotId: $0.id, name: $0.name) }
    }

    /// The catch the iPad shows before one is chosen: the newest that passes the filters.
    private var firstShown: LogSelection? {
        (catches.list?.rows).flatMap { filter.apply($0).first }.map { .entry($0.id) }
    }
}

/// The list column: "What worked", then the catches that pass the filters; swipe or touch and
/// hold to delete. Its toolbar logs a new catch, filters and exports the log.
private struct CatchListColumn: View {
    @Environment(SpotStore.self) private var spots
    let catches: CatchStore
    @Binding var selection: LogSelection?
    @Binding var filter: CatchListFilter
    @Binding var editor: CatchEditorSheet?
    @State private var deleting: CatchRow?
    /// The exported file the share sheet is showing.
    @State private var sharing: URL?

    var body: some View {
        let rows = catches.list?.rows ?? []
        let shown = filter.apply(rows)
        List(selection: $selection) {
            if catches.unreadable {
                Section { unreadableNotice }
            }
            if let problem = catches.problem {
                Section {
                    Label(problem, systemImage: "exclamationmark.triangle.fill")
                        .font(.subheadline)
                }
            }
            if !rows.isEmpty {
                Section {
                    Label("What worked", systemImage: "chart.bar.xaxis")
                        .tag(LogSelection.insights)
                }
                Section {
                    ForEach(shown) { row in
                        CatchRowView(row: row, selected: selection == .entry(row.id))
                            .tag(LogSelection.entry(row.id))
                            .contextMenu {
                                Button("Delete", systemImage: "trash", role: .destructive) {
                                    deleting = row
                                }
                            }
                            .swipeActions(edge: .trailing) {
                                Button(role: .destructive) {
                                    deleting = row
                                } label: {
                                    Label("Delete", systemImage: "trash")
                                }
                            }
                    }
                }
            }
        }
        .overlay {
            if let list = catches.list {
                if let empty = list.empty {
                    CatchEmptyState(empty: empty, spoken: list.spokenEmpty ?? empty)
                } else if shown.isEmpty {
                    ContentUnavailableView {
                        Label("No catches match", systemImage: "line.3.horizontal.decrease.circle")
                    } description: {
                        Text("No catch in the log passes these filters.")
                    } actions: {
                        Button("Show all catches") { filter = CatchListFilter() }
                    }
                }
            } else if catches.count > 0 {
                ProgressView()
            }
        }
        .navigationTitle("Log")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button("Log a catch", systemImage: "plus") { editor = .new }
            }
            if !rows.isEmpty {
                ToolbarItem(placement: .primaryAction) {
                    CatchFilterMenu(rows: rows, filter: $filter)
                }
                if let words = try? logExportWords(settingsJson: spots.settingsJSON) {
                    ToolbarItem(placement: .primaryAction) {
                        CatchExportMenu(catches: catches, words: words, sharing: $sharing)
                    }
                }
            }
        }
        .background(ExportSharePresenter(file: $sharing))
        .confirmationDialog(
            "Delete this catch?",
            isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }),
            titleVisibility: .visible, presenting: deleting
        ) { row in
            Button("Delete catch", role: .destructive) { catches.delete(row.id) }
        } message: { _ in
            Text("Its photos, notes and conditions are deleted from this device.")
        }
    }

    private var unreadableNotice: some View {
        Label {
            Text(
                catches.keptAs.map {
                    "The saved log could not be read. It was kept as \($0) and a new log started."
                } ?? "The saved log could not be read. It is left as it was and nothing is saved over it."
            )
            .fixedSize(horizontal: false, vertical: true)
        } icon: {
            Image(systemName: "exclamationmark.triangle.fill")
                .foregroundStyle(Palette.warning)
                .accessibilityHidden(true)
        }
        .font(.subheadline)
    }
}

#if DEBUG
    /// Launch arguments of the screenshot runs: `-editCatch <id>` opens that catch in the
    /// editor, `-newCatch` opens the editor for a new catch. The editor comes up once the
    /// catch's page has settled: a sheet asked for while the iPhone's stack is moving is lost.
    enum LogLaunch {
        private static var applied = false

        static func apply(
            open: (String) -> Void, edit: @escaping (CatchEditorSheet) -> Void
        ) {
            guard !applied else { return }
            applied = true
            let sheet: CatchEditorSheet
            if let id = LaunchArguments.value(after: "-editCatch") {
                open(id)
                sheet = .edit(id)
            } else if ProcessInfo.processInfo.arguments.contains("-newCatch") {
                sheet = .new
            } else {
                return
            }
            Task {
                try? await Task.sleep(for: .seconds(1))
                edit(sheet)
            }
        }
    }
#endif

/// The filter menu: species, place and fate, the choices the rows hold.
private struct CatchFilterMenu: View {
    let rows: [CatchRow]
    @Binding var filter: CatchListFilter

    var body: some View {
        Menu {
            choice("Species", all: "All species", CatchListFilter.species(in: rows), $filter.species)
            choice("Place", all: "All places", CatchListFilter.places(in: rows), $filter.place)
            choice("Kept or released", all: "All catches", CatchListFilter.fates(in: rows), $filter.fate)
            if filter.isActive {
                Divider()
                Button("Show all catches", systemImage: "xmark.circle") { filter = CatchListFilter() }
            }
        } label: {
            Label(
                "Filter",
                systemImage: filter.isActive
                    ? "line.3.horizontal.decrease.circle.fill" : "line.3.horizontal.decrease.circle")
        }
        .accessibilityValue(filter.isActive ? Text("Filtered") : Text("All catches"))
    }

    @ViewBuilder
    private func choice(
        _ title: String, all: String, _ values: [String], _ selection: Binding<String?>
    ) -> some View {
        if values.count > 1 || selection.wrappedValue != nil {
            Picker(title, selection: selection) {
                Text(all).tag(String?.none)
                ForEach(values, id: \.self) { Text($0).tag(String?.some($0)) }
            }
            .pickerStyle(.menu)
        }
    }
}

/// One catch of the list, as the core words it: species, size, time, place and fate.
private struct CatchRowView: View {
    let row: CatchRow
    /// Whether the row is the chosen one, drawn on the selection's colour.
    var selected = false

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(alignment: .firstTextBaseline) {
                Text(row.species).font(.headline)
                Spacer(minLength: 8)
                CatchFateTag(row: row, onSelection: selected)
            }
            if let size = row.size {
                Text(size).font(.subheadline)
            }
            Text(row.when).font(.subheadline).foregroundStyle(.readableSecondary)
            if let place = row.place {
                Text(place).font(.caption).foregroundStyle(.readableSecondary)
            }
        }
        .padding(.vertical, 4)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(row.summary))
    }
}

/// The fate of a catch as a small tag; Luna's sashimi tag in its own colour.
struct CatchFateTag: View {
    let row: CatchRow
    /// On a selected row's colour the tag is drawn in white, to stay readable.
    var onSelection = false

    var body: some View {
        Text(row.fateLabel)
            .font(.caption.weight(.semibold))
            .padding(.horizontal, 8)
            .padding(.vertical, 2)
            .background(onSelection ? Color.white.opacity(0.22) : tint.opacity(0.16), in: Capsule())
            .foregroundStyle(onSelection ? Color.white : tint)
    }

    private var tint: Color {
        switch row.fate {
        case .released: Palette.success
        case .kept: Palette.accent
        case .keptForSashimi: Palette.warning
        }
    }
}

/// The empty log in the core's words, with Luna in her voice; VoiceOver reads the plain words.
private struct CatchEmptyState: View {
    let empty: CatchList.Empty
    let spoken: CatchList.Empty
    @Environment(\.voiceStyle) private var voice

    var body: some View {
        VStack(spacing: 12) {
            if voice == .luna {
                LunaArt(picture: .paddling, maxWidth: 280)
            }
            Text(empty.title)
                .font(.title3.bold())
                .multilineTextAlignment(.center)
            Text(empty.body)
                .foregroundStyle(.readableSecondary)
                .multilineTextAlignment(.center)
        }
        .frame(maxWidth: 420)
        .padding()
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text("\(spoken.title). \(spoken.body)"))
    }
}
