import SwiftUI
import UnderbiteKit

/// A trip's page: its dates and status, then its days. On a regular width with room for every
/// place, the days are rows and the places columns; otherwise each day is a card with its
/// places one under the other. Every cell says what it rests on. A day opens its own page.
/// The sources and the fixed notices close the page. Every word is the core's.
struct TripGridView: View {
    @Environment(SpotStore.self) private var spots
    @Environment(\.horizontalSizeClass) private var sizeClass
    @Environment(\.dynamicTypeSize) private var typeSize
    let trips: TripStore
    let tripID: String
    let edit: () -> Void

    @State private var page: TripPage
    @State private var openDay: String?
    @State private var width: CGFloat = 0
    /// The narrowest a place's column may be before the page stacks: it grows with the text.
    @ScaledMetric(relativeTo: .subheadline) private var minimumColumn: CGFloat = 220
    private let spacing: CGFloat = 12

    init(trips: TripStore, tripID: String, edit: @escaping () -> Void) {
        self.trips = trips
        self.tripID = tripID
        self.edit = edit
        _page = State(initialValue: TripPage(tripID: tripID))
    }

    private var trip: SavedTrip? { trips.trip(tripID) }

    /// What the page is built from: the trip, the settings and each place's last fetch.
    private var buildKey: String {
        let fetched = (trips.listing(tripID)?.places ?? []).map { place in
            "\(place.key)=\(spots.listing(for: place.key)?.status.lastSuccess ?? 0)"
        }
        return [trip?.json ?? "", spots.settingsJSON, fetched.joined(separator: ",")]
            .joined(separator: "\n")
    }

    var body: some View {
        ScrollView {
            if let view = page.view {
                content(view)
                    .padding(.horizontal)
                    .padding(.bottom, 24)
            } else if let problem = page.problem {
                Text(problem)
                    .foregroundStyle(.readableSecondary)
                    .padding()
                    .containerRelativeFrame(.vertical)
            } else {
                LunaLoading().containerRelativeFrame(.vertical)
            }
        }
        .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width = $0 }
        .background(Color(.systemGroupedBackground))
        .readableText()
        .navigationTitle(page.view?.title ?? trips.listing(tripID)?.title ?? "")
        .navigationBarTitleDisplayMode(.inline)
        .navigationDestination(item: $openDay) { date in
            TripDayDetail(page: page, date: date)
        }
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button("Edit", action: edit)
            }
            if let trip {
                ToolbarItem(placement: .primaryAction) {
                    TripShareButton(tripJSON: trip.json)
                }
            }
        }
        .refreshable {
            if let trip { await page.refresh(trip, spots: spots, all: true) }
        }
        .task(id: buildKey) {
            if let trip { await page.build(trip, spots: spots) }
        }
        .task(id: tripID) {
            if let trip { await page.refresh(trip, spots: spots, all: false) }
        }
        #if DEBUG
            .task(id: page.view?.days.count) {
                // Once the page has finished appearing: a push asked for while the split view
                // is still settling is lost.
                guard let view = page.view, let date = TripGridLaunch.day(in: view) else { return }
                try? await Task.sleep(for: .seconds(2))
                openDay = date
            }
        #endif
    }

    private func content(_ view: ViewModel.TripView) -> some View {
        VStack(alignment: .leading, spacing: 16) {
            TripHeader(view: view)
            switch form(view) {
            case .grid:
                TripDayGrid(view: view, spacing: spacing) { openDay = $0 }
            case .stacked:
                TripDayCards(view: view) { openDay = $0 }
            }
            if !view.notes.isEmpty {
                Text(view.notes)
                    .font(.subheadline)
                    .foregroundStyle(.readableSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            VStack(alignment: .leading, spacing: 6) {
                NoticeList(notices: view.notices)
                CreditsLine(credits: view.attribution)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func form(_ view: ViewModel.TripView) -> TripGridLayout.Form {
        TripGridLayout.form(
            regular: sizeClass == .regular, accessibilitySize: typeSize.isAccessibilitySize,
            // The page's own padding is not the grid's.
            width: Double(width) - 32, places: view.places.count,
            minimumColumn: Double(minimumColumn), spacing: Double(spacing))
    }
}

/// The trip's dates and status, as the core words them.
private struct TripHeader: View {
    let view: ViewModel.TripView

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            if !view.dates.isEmpty { Text(view.dates).font(.title3.bold()) }
            if !view.status.isEmpty {
                Text(view.status).foregroundStyle(.readableSecondary)
            }
        }
        .accessibilityElement(children: .combine)
        .padding(.top, 8)
    }
}

// MARK: - Places as columns

/// Days as rows and places as columns: a header row naming the places, then for each day its
/// label across the width and one cell per place under it.
private struct TripDayGrid: View {
    let view: ViewModel.TripView
    let spacing: CGFloat
    let open: (String) -> Void

    var body: some View {
        Grid(alignment: .topLeading, horizontalSpacing: spacing, verticalSpacing: 8) {
            GridRow {
                ForEach(view.places) { place in
                    PlaceHeader(place: place)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            ForEach(view.days) { day in
                GridRow {
                    DayTitle(day: day, open: open)
                        .gridCellColumns(max(view.places.count, 1))
                        .padding(.top, 8)
                }
                GridRow(alignment: .top) {
                    ForEach(Array(day.places.enumerated()), id: \.offset) { _, placeDay in
                        if let placeDay {
                            PlaceDayButton(day: day, placeDay: placeDay, open: open)
                        } else {
                            Color.clear.frame(height: 1).accessibilityHidden(true)
                        }
                    }
                }
            }
        }
    }
}

/// A place's column header: its name, area and kind of water, and whether it is approximate or
/// has no days.
private struct PlaceHeader: View {
    let place: ViewModel.TripPlaceColumn

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(place.name).font(.headline)
            if let area = place.area {
                Text(area).font(.caption).foregroundStyle(.readableSecondary)
            }
            Text(place.waterTypeLabel).font(.caption).foregroundStyle(.readableSecondary)
            if let approximate = place.approximate {
                Text(approximate).font(.caption).foregroundStyle(.readableSecondary)
            }
            if let problem = place.problem {
                Text(problem)
                    .font(.caption)
                    .foregroundStyle(Palette.warning)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isHeader)
    }
}

/// A day's label, which opens the day.
private struct DayTitle: View {
    let day: ViewModel.TripDayRow
    let open: (String) -> Void

    var body: some View {
        Button {
            open(day.date)
        } label: {
            HStack(spacing: 4) {
                Text(day.label).font(.title3.weight(.semibold))
                Image(systemName: "chevron.right")
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(.readableTertiary)
                    .accessibilityHidden(true)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(Text(day.accessibility))
        .accessibilityAddTraits(.isHeader)
    }
}

/// One place on one day, as a tile that opens the day.
private struct PlaceDayButton: View {
    let day: ViewModel.TripDayRow
    let placeDay: ViewModel.TripPlaceDay
    /// The place's name, shown above its cells when the places are stacked.
    var title: String?
    let open: (String) -> Void
    @Environment(\.sunMode) private var sunMode

    var body: some View {
        Button {
            open(day.date)
        } label: {
            VStack(alignment: .leading, spacing: 8) {
                if let title { Text(title).font(.headline) }
                PlaceDayCells(placeDay: placeDay)
            }
            .padding(12)
            .frame(maxWidth: .infinity, alignment: .topLeading)
            .background(
                Color(.secondarySystemGroupedBackground),
                in: RoundedRectangle(cornerRadius: 14, style: .continuous))
            .overlay {
                if placeDay.isToday || sunMode {
                    RoundedRectangle(cornerRadius: 14, style: .continuous)
                        .strokeBorder(
                            placeDay.isToday ? Palette.accent : Palette.cardStroke(sunMode),
                            lineWidth: 2)
                }
            }
            .opacity(placeDay.isPast ? 0.6 : 1)
            .contentShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
        }
        .buttonStyle(.plain)
        // One sentence for the whole tile, in the plain voice.
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(placeDay.accessibility))
        .accessibilityAddTraits(.isButton)
    }
}

// MARK: - Days as cards

/// One card per day, its places one under the other.
private struct TripDayCards: View {
    let view: ViewModel.TripView
    let open: (String) -> Void

    var body: some View {
        LazyVStack(alignment: .leading, spacing: 16) {
            ForEach(view.days) { day in
                VStack(alignment: .leading, spacing: 8) {
                    DayTitle(day: day, open: open)
                    ForEach(Array(day.places.enumerated()), id: \.offset) { index, placeDay in
                        if let placeDay {
                            PlaceDayButton(
                                day: day, placeDay: placeDay,
                                title: view.places.indices.contains(index)
                                    ? view.places[index].name : nil,
                                open: open)
                        } else if view.places.indices.contains(index) {
                            // A place without this day says why, under its name.
                            PlaceHeader(place: view.places[index])
                        }
                    }
                }
            }
        }
    }
}

// MARK: - A place's day

/// The cells of one place on one day, two side by side (one at the accessibility sizes), each
/// with its label, its value and what it rests on. The verdict is a symbol and a colour as well
/// as its word; a storm's reasons are shown whole.
struct PlaceDayCells: View {
    let placeDay: ViewModel.TripPlaceDay
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        let cells = placeDay.cells
        let columns = typeSize.isAccessibilitySize ? 1 : 2
        let rows = TripGridLayout.rows(cells.map(\.kind), columns: columns)
        VStack(alignment: .leading, spacing: 10) {
            if placeDay.storm {
                StormReasons(reasons: placeDay.reasons)
            }
            Grid(alignment: .topLeading, horizontalSpacing: 12, verticalSpacing: 10) {
                ForEach(Array(rows.enumerated()), id: \.offset) { _, row in
                    GridRow(alignment: .top) {
                        ForEach(row, id: \.self) { index in
                            CellTile(cell: cells[index], verdict: placeDay.verdict)
                                .gridCellColumns(row.count == 1 ? columns : 1)
                                .frame(maxWidth: .infinity, alignment: .topLeading)
                        }
                    }
                }
            }
        }
    }
}

/// A storm ahead or a hurricane cone: the day's reasons, whole, as the warnings they are.
private struct StormReasons: View {
    let reasons: [ViewModel.GoNoGoReasonVm]

    var body: some View {
        ForEach(Array(reasons.enumerated()), id: \.offset) { _, reason in
            Label {
                Text(reason.text).fixedSize(horizontal: false, vertical: true)
            } icon: {
                Image(systemName: "exclamationmark.triangle.fill")
                    .foregroundStyle(Palette.warning)
                    .accessibilityHidden(true)
            }
            .font(.subheadline.weight(.semibold))
        }
    }
}

/// One cell: its label, its value (every high and low for the tides) and its basis, always
/// shown. Past years' values carry a history symbol beside their basis.
private struct CellTile: View {
    let cell: ViewModel.TripCell
    let verdict: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 1) {
            Text(cell.label)
                .font(.caption.weight(.semibold))
                .foregroundStyle(.readableSecondary)
            if cell.kind == "verdict", let verdict, !cell.isUnavailable {
                Label {
                    Text(cell.value)
                } icon: {
                    Image(systemName: GoNoGoDetail.symbol(verdict))
                        .foregroundStyle(GoNoGoDetail.color(verdict))
                }
                .font(.subheadline.weight(.semibold))
            } else {
                ForEach(
                    Array(
                        TripGridLayout.shownLines(kind: cell.kind, value: cell.value, lines: cell.lines)
                            .enumerated()), id: \.offset
                ) { _, line in
                    Text(line)
                        .font(.subheadline.weight(.semibold))
                        .fixedSize(horizontal: false, vertical: true)
                }
                .greyed(cell.isUnavailable)
            }
            BasisLine(cell: cell)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(cell.accessibility))
    }
}

/// What a cell rests on, in the core's words: certain, a forecast and its issue, past years
/// with their years, or unavailable and why.
struct BasisLine: View {
    let cell: ViewModel.TripCell

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 3) {
            if cell.isTypical {
                Image(systemName: "clock.arrow.circlepath")
                    .accessibilityHidden(true)
            }
            Text(cell.basisLabel).fixedSize(horizontal: false, vertical: true)
        }
        .font(.caption2)
        .foregroundStyle(cell.isTypical ? AnyShapeStyle(Palette.accent) : AnyShapeStyle(.readableSecondary))
    }
}

#if DEBUG
    /// Launch arguments of the screenshot runs: `-openTripDay <n>` opens the trip's day `n`
    /// (from 0) once its page is built.
    enum TripGridLaunch {
        private static var applied = false

        static func day(in view: ViewModel.TripView) -> String? {
            guard !applied, let text = LaunchArguments.value(after: "-openTripDay"),
                let n = Int(text), view.days.indices.contains(n)
            else { return nil }
            applied = true
            return view.days[n].date
        }
    }
#endif
