import SwiftUI
@preconcurrency import UnderbiteKit

/// "What worked": the conditions of the angler's own catches, counted by the core for the
/// species, place and days the filters choose. The page's words, the bars' labels and the
/// sentences are all the core's; the notice that counts are catches, not effort, stays on top.
/// One chart per row on the iPhone and in a narrow window, two side by side in a wide one.
struct InsightsView: View {
    @Environment(SpotStore.self) private var spots
    @Environment(\.horizontalSizeClass) private var sizeClass
    @Environment(\.calendar) private var calendar
    var catches: CatchStore = .shared
    @State private var filter = InsightsFilter()
    @State private var page: InsightsPage?
    @State private var loaded = false
    @State private var catalogue: [SpeciesChoice] = []

    var body: some View {
        let choices = InsightsChoices(
            rows: catches.list?.rows ?? [], catalogue: catalogue, spots: places)
        Group {
            if let page {
                content(page, choices: choices)
            } else if loaded {
                ContentUnavailableView(
                    "What worked", systemImage: "chart.bar.xaxis",
                    description: Text("The log could not be read."))
            } else {
                ProgressView()
            }
        }
        .navigationTitle("What worked")
        .navigationBarTitleDisplayMode(.inline)
        .task(id: spots.settingsJSON) {
            catalogue = SpeciesChoice.catalogue(settingsJSON: spots.settingsJSON)
            #if DEBUG
                if let species = InsightsLaunch.species() { filter.species = species }
            #endif
        }
        .task(id: "\(catches.revision) \(spots.settingsJSON.hashValue) \(filter.hashValue)") {
            let book = catches.book
            let settings = spots.settingsJSON
            let core = filter.catchFilter(calendar: calendar)
            let built = await Task.detached(priority: .userInitiated) {
                InsightsPage.build(book: book, filter: core, settingsJSON: settings)
            }.value
            guard !Task.isCancelled else { return }
            page = built
            loaded = true
        }
        .onChange(of: choices) { _, now in
            // Until the list and the catalogue are read there is nothing to prune against.
            guard catches.list != nil, !catalogue.isEmpty else { return }
            filter = filter.pruned(for: now)
        }
    }

    /// The saved spots' names, for the catches filed under them.
    private var places: [PlaceName] {
        spots.spots.map { PlaceName(spotId: $0.id, name: $0.name) }
    }

    @ViewBuilder
    private func content(_ page: InsightsPage, choices: InsightsChoices) -> some View {
        if let empty = page.empty, !filter.isActive {
            ContentUnavailableView(
                "What worked", systemImage: "chart.bar.xaxis", description: Text(empty))
        } else {
            ScrollViewReader { proxy in
                ScrollView {
                    VStack(alignment: .leading, spacing: 14) {
                        InsightsFilterBar(choices: choices, filter: $filter)
                        header(page)
                        grid(page.arranged)
                    }
                    .padding()
                    .frame(maxWidth: sizeClass == .regular ? 1_000 : .infinity)
                    .frame(maxWidth: .infinity)
                }
                .background(Color(.systemGroupedBackground))
                #if DEBUG
                    .task(id: page.sections.map(\.id)) {
                        guard let kind = InsightsLaunch.scrollTarget(in: page) else { return }
                        try? await Task.sleep(for: .milliseconds(300))
                        proxy.scrollTo(kind, anchor: .bottom)
                    }
                #endif
            }
        }
    }

    private func header(_ page: InsightsPage) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(page.headline).font(.headline)
            if let note = page.empty ?? page.tooFew {
                Text(note)
                    .font(.subheadline)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Label {
                Text(page.notice).fixedSize(horizontal: false, vertical: true)
            } icon: {
                Image(systemName: "info.circle").accessibilityHidden(true)
            }
            .font(.footnote)
            .foregroundStyle(.readableSecondary)
        }
        .accessibilityElement(children: .combine)
    }

    /// The sections, one per row in compact width and two per row in regular width, each row's
    /// cards top-aligned.
    private func grid(_ sections: [InsightsPage.Section]) -> some View {
        let columns = sizeClass == .regular ? 2 : 1
        return Grid(alignment: .topLeading, horizontalSpacing: 14, verticalSpacing: 14) {
            ForEach(InsightsPage.rows(sections, columns: columns), id: \.first?.id) { row in
                GridRow {
                    ForEach(row) { section in InsightChart(section: section).id(section.id) }
                    if row.count < columns {
                        Color.clear.gridCellUnsizedAxes([.horizontal, .vertical])
                    }
                }
            }
        }
    }
}

/// The filters: a species and a place the log's catches have, and the first and last day.
private struct InsightsFilterBar: View {
    let choices: InsightsChoices
    @Binding var filter: InsightsFilter

    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 12) {
                pickers
                days
                Spacer(minLength: 0)
                reset
            }
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 10) {
                    pickers
                    Spacer(minLength: 0)
                    reset
                }
                HStack(spacing: 16) { days }
            }
            VStack(alignment: .leading, spacing: 2) {
                pickers
                days
                reset
            }
        }
    }

    /// Clears every filter; shown only while one is set.
    @ViewBuilder private var reset: some View {
        if filter.isActive {
            Button("Count all catches", systemImage: "xmark.circle") {
                filter = InsightsFilter()
            }
            .labelStyle(.iconOnly)
        }
    }

    @ViewBuilder private var pickers: some View {
        Picker("Species", selection: $filter.species) {
            Text("All species").tag(String?.none)
            ForEach(choices.species) { Text($0.name).tag(String?.some($0.id)) }
        }
        .pickerStyle(.menu)
        .fixedSize()
        Picker("Place", selection: $filter.place) {
            Text("All places").tag(InsightsFilter.Place?.none)
            ForEach(choices.places, id: \.self) {
                Text($0.name).tag(InsightsFilter.Place?.some($0.place))
            }
        }
        .pickerStyle(.menu)
        .fixedSize()
    }

    @ViewBuilder private var days: some View {
        DayToggle(title: "From", day: $filter.from, other: filter.until, earliest: true)
        DayToggle(title: "Until", day: $filter.until, other: filter.from, earliest: false)
    }
}

/// A day limit of the filters: off, or on with the day to choose.
private struct DayToggle: View {
    let title: String
    @Binding var day: Date?
    /// The other limit, which this one may not pass.
    let other: Date?
    /// Whether this is the first day (it may not come after `other`).
    let earliest: Bool

    var body: some View {
        HStack(spacing: 8) {
            Toggle(
                title,
                isOn: Binding(
                    get: { day != nil },
                    set: { day = $0 ? (other ?? Date()) : nil })
            )
            .toggleStyle(.button)
            .font(.subheadline)
            if let current = day {
                DatePicker(
                    title,
                    selection: Binding(get: { current }, set: { day = $0 }),
                    in: range,
                    displayedComponents: .date
                )
                .labelsHidden()
                .fixedSize()
            }
        }
    }

    private var range: ClosedRange<Date> {
        if earliest {
            return Date.distantPast...(other ?? Date.distantFuture)
        }
        return (other ?? Date.distantPast)...Date.distantFuture
    }
}

#if DEBUG
    /// Launch arguments of the screenshot runs: `-whatWorked <species id>` counts that species
    /// the first time the page opens, and `-insightsScroll <kind>` scrolls once so that
    /// condition's card ends at the bottom of the screen.
    enum InsightsLaunch {
        private static var applied = false
        private static var scrolled = false

        static func scrollTarget(in page: InsightsPage) -> String? {
            guard !scrolled, let kind = LaunchArguments.value(after: "-insightsScroll"),
                page.sections.contains(where: { $0.id == kind && $0.charted })
            else { return nil }
            scrolled = true
            return kind
        }

        static func species() -> String? {
            guard !applied else { return nil }
            applied = true
            guard let id = LaunchArguments.value(after: "-whatWorked"), !id.hasPrefix("-")
            else { return nil }
            return id
        }
    }
#endif
