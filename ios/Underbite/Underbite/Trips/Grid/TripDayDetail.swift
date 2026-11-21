import SwiftUI
import UnderbiteKit

/// One day of a trip: for each place, its verdict with every reason (a storm's or a hurricane's
/// with its official link), its fishing windows with the factors that lined up, and every cell
/// with all its lines, what it rests on and its source. The sources and the fixed notices close
/// the page. Every word is the core's.
struct TripDayDetail: View {
    let page: TripPage
    /// The day's date, `YYYY-MM-DD`.
    let date: String

    var body: some View {
        let view = page.view
        let day = view?.days.first { $0.date == date }
        List {
            if let view, let day {
                ForEach(Array(view.places.enumerated()), id: \.offset) { index, place in
                    let placeDay = day.places.indices.contains(index) ? day.places[index] : nil
                    PlaceDaySections(place: place, placeDay: placeDay)
                }
                Section {
                    NoticeList(notices: view.notices)
                    CreditsLine(credits: view.attribution)
                }
            }
        }
        .listStyle(.insetGrouped)
        .readableText()
        .navigationTitle(day?.label ?? "")
        .navigationBarTitleDisplayMode(.inline)
    }
}

/// A place's part of the day.
private struct PlaceDaySections: View {
    let place: ViewModel.TripPlaceColumn
    let placeDay: ViewModel.TripPlaceDay?

    var body: some View {
        Section {
            if let placeDay {
                // The verdict first, then why, then the rest of the day.
                let reasons = placeDay.reasons.map(\.text)
                ForEach(
                    Array(placeDay.cells.filter { $0.kind == "verdict" }.enumerated()),
                    id: \.offset
                ) { _, cell in
                    CellRow(cell: cell, verdict: placeDay.verdict, reasons: reasons)
                }
                ForEach(Array(placeDay.reasons.enumerated()), id: \.offset) { _, reason in
                    ReasonRow(reason: reason)
                }
                ForEach(
                    Array(placeDay.cells.filter { $0.kind != "verdict" }.enumerated()),
                    id: \.offset
                ) { _, cell in
                    CellRow(cell: cell, verdict: placeDay.verdict)
                }
            } else if let problem = place.problem {
                Text(problem).foregroundStyle(Palette.warning)
            }
        } header: {
            VStack(alignment: .leading, spacing: 2) {
                Text(place.name).font(.headline).foregroundStyle(.primary)
                if let approximate = place.approximate {
                    Text(approximate).font(.caption)
                }
            }
            .textCase(nil)
            .accessibilityElement(children: .combine)
        }
        if let placeDay, !placeDay.windows.isEmpty {
            Section {
                ForEach(placeDay.windows, id: \.startT) { window in
                    WindowLine(window: window)
                }
            } header: {
                if let label = placeDay.cell("windows")?.label {
                    Text(label).textCase(nil)
                }
            }
        }
    }
}

/// One reason of the verdict; a safety reason is never shortened, and one with an official page
/// links to it.
private struct ReasonRow: View {
    let reason: ViewModel.GoNoGoReasonVm

    var body: some View {
        if let link = reason.link.flatMap(URL.init(string:)) {
            Link(destination: link) {
                Label {
                    Text(reason.text).fixedSize(horizontal: false, vertical: true)
                } icon: {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .foregroundStyle(Palette.warning)
                        .accessibilityHidden(true)
                }
            }
        } else {
            Text(reason.text).fixedSize(horizontal: false, vertical: true)
        }
    }
}

/// One cell in full: its label and value, every line, what it rests on and its source.
private struct CellRow: View {
    let cell: ViewModel.TripCell
    let verdict: String?
    /// The reasons listed under the verdict: the cell does not say them again.
    var reasons: [String] = []

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(cell.label)
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(.readableSecondary)
            if cell.kind == "verdict", let verdict, !cell.isUnavailable {
                Label {
                    Text(cell.value).font(.headline)
                } icon: {
                    Image(systemName: GoNoGoDetail.symbol(verdict))
                        .foregroundStyle(GoNoGoDetail.color(verdict))
                }
            } else {
                Text(cell.value).font(.headline).greyed(cell.isUnavailable)
            }
            // The value is the first line of some cells (the first tide): it is not repeated.
            let lines =
                cell.kind == "verdict"
                ? TripGridLayout.linesWithoutReasons(cell.lines, reasons: reasons) : cell.lines
            ForEach(Array(lines.enumerated()), id: \.offset) { index, line in
                if !(index == 0 && line == cell.value) {
                    Text(line).fixedSize(horizontal: false, vertical: true)
                }
            }
            BasisLine(cell: cell)
            if let source = cell.source {
                Text(source).font(.caption2).foregroundStyle(.readableSecondary)
            }
        }
        .padding(.vertical, 2)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(cell.accessibility))
    }
}

/// One fishing window: its range, the factors that lined up and those that had no data.
private struct WindowLine: View {
    let window: ViewModel.WindowVm

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(window.range).font(.headline)
            ForEach(Array(window.factors.enumerated()), id: \.offset) { _, factor in
                VStack(alignment: .leading, spacing: 0) {
                    Text(factor.label).font(.subheadline.weight(.semibold))
                    Text(factor.detail).font(.subheadline).foregroundStyle(.readableSecondary)
                }
            }
            if let text = window.noDataText {
                Text(text).font(.caption).foregroundStyle(.readableSecondary)
            }
        }
        .padding(.vertical, 2)
        .accessibilityElement(children: .combine)
    }
}
