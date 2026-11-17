import SwiftUI
import UnderbiteKit

/// A card's full form: the headline, the card's charts, the notices, what the card says in
/// words, the view's own sections of label and value rows and every credit with its link. All
/// text is the view model's; the charts only lay it out.
struct CardDetailView: View {
    @Environment(SpotStore.self) private var store
    let card: ViewModel.Card

    var body: some View {
        List {
            if let compact = card.compact {
                // A header, not a row: a row's rounded corners clip the first letters of
                // the lines that reach its leading edge.
                Section {
                } header: {
                    CompactBody(compact: compact)
                        .foregroundStyle(Color.primary)
                        .textCase(nil)
                        .padding(.bottom, 4)
                        .accessibilityElement(children: .combine)
                        .accessibilityLabel(
                            Text(card.chrome?.accessibility ?? compact.headline.text))
                }
            }
            if let chrome = card.chrome { charts(chrome) }
            details
            if let chrome = card.chrome { footer(chrome) }
        }
        .readableText()
        .navigationTitle(card.chrome?.title ?? "")
        .navigationBarTitleDisplayMode(.inline)
    }

    // MARK: - Charts

    /// The card's charts, then its notices ("Not for navigation.") right under them.
    @ViewBuilder private func charts(_ chrome: ViewModel.CardChrome) -> some View {
        let blocks = CardCharts.blocks(for: card, settingsJSON: store.settingsJSON)
        if !blocks.isEmpty {
            Section {
                ForEach(blocks) { block in
                    ChartBlock(chart: block.chart, valueBand: block.band, notes: block.notes)
                }
            }
        }
        if !chrome.notices.isEmpty {
            Section { NoticeList(notices: chrome.notices) }
        }
    }

    // MARK: - By kind

    @ViewBuilder private var details: some View {
        switch card {
        case .now(let c): nowRows(c.full)
        case .tide(let c): tideRows(c.full)
        case .current(let c): currentRows(c.full)
        case .flow(let c): flowRows(c.full)
        case .waterTemp(let c): waterTempRows(c.full)
        case .clarity(let c): clarityRows(c.full)
        case .waves(let c): wavesRows(c.full)
        case .wind(let c): windRows(c.full)
        case .pressure(let c): sections(c.full.sections, trend: c.full.trend?.text)
        case .sunMoon(let c): sunMoonRows(c.full)
        case .solunar(let c): solunarRows(c.full)
        case .marineForecast(let c): forecastRows(c.full)
        case .stormAhead(let c): stormRows(c.full)
        case .alerts(let c): alertRows(c.full)
        case .sources(let c): sourcesRows(c.full)
        case .windows(let c):
            WindowsDetail(full: c.full)
            sections(c.full.sections)
        case .goNoGo(let c): GoNoGoDetail(full: c.full)
        case .unknown: EmptyView()
        }
    }

    private func nowRows(_ full: ViewModel.NowFull) -> some View {
        Section {
            ForEach(Array(full.items.enumerated()), id: \.offset) { _, item in
                LabeledContent(item.label) {
                    VStack(alignment: .trailing) {
                        QuantityText(quantity: item.headline, font: .headline)
                        if let line = item.line {
                            Text(line).font(.caption).foregroundStyle(.readableSecondary)
                        }
                    }
                }
                .greyed(CardStyle.isGreyed(item.state))
            }
        }
    }

    @ViewBuilder private func tideRows(_ full: ViewModel.TideFull) -> some View {
        Section {
            Text(full.stageLabel)
            if let label = full.datumLabel { Text(label).foregroundStyle(.readableSecondary) }
            ForEach(full.next, id: \.t) { event in tideEvent(event) }
        }
        ForEach(full.days, id: \.date) { day in
            Section(day.date) {
                ForEach(day.events, id: \.t) { event in tideEvent(event) }
                if let sunrise = day.sunrise { timed(sunrise) }
                if let sunset = day.sunset { timed(sunset) }
                if let moon = day.moon { moonRow(moon) }
            }
        }
        sections(full.sections)
    }

    private func tideEvent(_ event: ViewModel.TideEvent) -> some View {
        LabeledContent(event.label) {
            HStack {
                Text(event.time)
                if let height = event.height { QuantityText(quantity: height, font: .body) }
            }
        }
    }

    private func timed(_ item: ViewModel.TimedLabel) -> some View {
        LabeledContent(item.label, value: item.time)
    }

    private func moonRow(_ moon: ViewModel.MoonPhase) -> some View {
        LabeledContent(moon.label) { QuantityText(quantity: moon.illumination, font: .body) }
    }

    @ViewBuilder private func currentRows(_ full: ViewModel.CurrentFull) -> some View {
        Section {
            Text(full.phaseLabel)
            if let direction = full.directionNow { Text(direction) }
            if let next = full.nextSlack {
                LabeledContent(next.label, value: next.time)
            }
            ForEach(full.events, id: \.t) { event in
                LabeledContent(event.label) {
                    HStack {
                        Text(event.time)
                        if let speed = event.speed { QuantityText(quantity: speed, font: .body) }
                    }
                }
            }
            if let flood = full.floodDirection { Text(flood) }
            if let ebb = full.ebbDirection { Text(ebb) }
        }
        sections(full.sections)
    }

    @ViewBuilder private func flowRows(_ full: ViewModel.FlowFull) -> some View {
        Section {
            if let trend = full.trend { Text(trend.text) }
            if let range = full.rangeStatus { Text(range.text) }
            if let percentile = full.percentile {
                Text(percentile.text)
                Text(percentile.classLabel).foregroundStyle(.readableSecondary)
            }
            ForEach(Array(full.floodCategories.enumerated()), id: \.offset) { _, category in
                LabeledContent(category.label) { QuantityText(quantity: category.stage, font: .body) }
            }
        }
        sections(full.sections)
    }

    @ViewBuilder private func waterTempRows(_ full: ViewModel.WaterTempFull) -> some View {
        Section {
            if let trend = full.trend { Text(trend.text) }
            if let stress = full.troutStress { Text(stress.text) }
        }
        sections(full.sections)
    }

    @ViewBuilder private func clarityRows(_ full: ViewModel.ClarityFull) -> some View {
        Section {
            Text(full.kd490.note)
        }
        sections(full.sections)
    }

    @ViewBuilder private func wavesRows(_ full: ViewModel.WavesFull) -> some View {
        if let observed = full.observed { waveNow(observed) }
        if let forecast = full.forecast { waveNow(forecast) }
        sections(full.sections)
    }

    private func waveNow(_ wave: ViewModel.WaveNow) -> some View {
        Section(wave.time) {
            if let height = wave.significantHeight {
                QuantityText(quantity: height, font: .headline)
            }
            if let period = wave.dominantPeriod { QuantityText(quantity: period, font: .body) }
            if let direction = wave.direction { Text(direction) }
            ForEach(Array(wave.swells.enumerated()), id: \.offset) { _, train in
                LabeledContent(train.label) {
                    HStack {
                        if let height = train.height { QuantityText(quantity: height, font: .body) }
                        if let period = train.period { QuantityText(quantity: period, font: .body) }
                    }
                }
            }
            if let steepness = wave.steepness { Text(steepness) }
        }
    }

    @ViewBuilder private func windRows(_ full: ViewModel.WindFull) -> some View {
        Section {
            if let direction = full.direction { Text(direction) }
            ForEach(full.hours, id: \.t) { hour in
                LabeledContent(hour.time) {
                    HStack {
                        QuantityText(quantity: hour.speed, font: .body)
                        if let gust = hour.gust { QuantityText(quantity: gust, font: .body) }
                        if let direction = hour.direction { Text(direction).foregroundStyle(.readableSecondary) }
                    }
                }
            }
        }
        sections(full.sections)
    }

    @ViewBuilder private func sunMoonRows(_ full: ViewModel.SunMoonFull) -> some View {
        ForEach(full.days, id: \.date) { day in
            Section(day.date) {
                ForEach(day.sun, id: \.t) { timed($0) }
                if let length = day.dayLength { Text(length) }
                if let polar = day.polar { Text(polar) }
                moonRow(day.moon.phase)
                if let rise = day.moon.rise { timed(rise) }
                if let set = day.moon.set { timed(set) }
            }
        }
        sections(full.sections)
    }

    @ViewBuilder private func solunarRows(_ full: ViewModel.SolunarFull) -> some View {
        ForEach(full.days, id: \.date) { day in
            Section(day.date) {
                ForEach(day.periods, id: \.startT) { period in
                    VStack(alignment: .leading) {
                        LabeledContent(period.label, value: period.range)
                        Text(period.causeLabel).font(.caption).foregroundStyle(.readableSecondary)
                    }
                }
            }
        }
        sections(full.sections)
    }

    private func forecastRows(_ full: ViewModel.MarineForecastFull) -> some View {
        Section {
            Text(full.zoneName).font(.headline)
            Text("\(full.product) · \(full.issued)").foregroundStyle(.readableSecondary)
            ForEach(Array(full.periods.enumerated()), id: \.offset) { _, period in
                VStack(alignment: .leading, spacing: 2) {
                    Text(period.name).font(.subheadline.weight(.semibold))
                    Text(period.text)
                }
            }
        }
    }

    @ViewBuilder private func stormRows(_ full: ViewModel.StormAheadFull) -> some View {
        if let event = full.event {
            Section(event.title) {
                Text(event.window)
                Text(event.headline.text)
                if let agreement = event.agreement { Text(agreement.sentence) }
            }
        }
        ForEach(full.hurricanes, id: \.id) { storm in
            Section(storm.title) {
                Text(storm.strength)
                Text(storm.movement)
                Text(storm.reaches)
                if let link = storm.advisoryUrl.flatMap(URL.init(string:)) {
                    Link(storm.advisory, destination: link)
                } else {
                    Text(storm.advisory)
                }
            }
        }
        sections(full.sections)
    }

    /// The warnings and advisories: the National Weather Service's words, whole.
    private func alertRows(_ full: ViewModel.AlertsFull) -> some View {
        ForEach(full.alerts) { alert in
            Section(alert.event) {
                if let headline = alert.headline { Text(headline).font(.headline) }
                if let span = alert.span { Text(span).foregroundStyle(.readableSecondary) }
                Text(alert.severityLabel).foregroundStyle(.readableSecondary)
                if let description = alert.description { Text(description) }
                if let instruction = alert.instruction { Text(instruction) }
                if !alert.zones.isEmpty {
                    Text(alert.zones.joined(separator: ", ")).font(.caption).foregroundStyle(.readableSecondary)
                }
            }
        }
    }

    @ViewBuilder private func sourcesRows(_ full: ViewModel.SourcesFull) -> some View {
        Section {
            ForEach(Array(full.stations.enumerated()), id: \.offset) { _, use in
                VStack(alignment: .leading, spacing: 2) {
                    LabeledContent(use.measurement, value: use.station)
                    Text([use.distance, use.age, use.relation].compactMap { $0 }.joined(separator: " · "))
                        .font(.caption)
                        .foregroundStyle(.readableSecondary)
                    StateReason(state: use.state)
                }
                .greyed(CardStyle.isGreyed(use.state))
            }
        }
        if !full.errors.isEmpty {
            Section {
                ForEach(Array(full.errors.enumerated()), id: \.offset) { _, error in
                    Label("\(error.source): \(error.message)", systemImage: "exclamationmark.triangle")
                        .foregroundStyle(Palette.warning)
                }
            }
        }
    }

    // MARK: - Shared pieces

    @ViewBuilder private func sections(_ list: [ViewModel.Section], trend: String? = nil) -> some View {
        if let trend {
            Section { Text(trend) }
        }
        ForEach(Array(list.enumerated()), id: \.offset) { _, section in
            Section(section.title) {
                ForEach(Array(section.rows.enumerated()), id: \.offset) { _, row in
                    VStack(alignment: .leading, spacing: 2) {
                        LabeledContent(row.label, value: row.value)
                        if let detail = row.detail {
                            Text(detail).font(.caption).foregroundStyle(.readableSecondary)
                        }
                    }
                }
            }
        }
    }

    /// The reason a value is greyed, the notices, the footnote and every credit with its link.
    @ViewBuilder private func footer(_ chrome: ViewModel.CardChrome) -> some View {
        Section {
            StateReason(state: chrome.state)
            if let footnote = chrome.footnote {
                Text(footnote).font(.footnote).foregroundStyle(.readableSecondary)
            }
        }
        if case .sources(let sources) = card {
            credits(sources.full.credits)
        } else {
            credits(chrome.credits)
        }
    }

    @ViewBuilder private func credits(_ list: [ViewModel.Credit]) -> some View {
        if !list.isEmpty {
            Section {
                ForEach(Array(list.enumerated()), id: \.offset) { _, credit in
                    VStack(alignment: .leading, spacing: 2) {
                        if let link = credit.link.flatMap(URL.init(string:)) {
                            Link(credit.text, destination: link)
                        } else {
                            Text(credit.text)
                        }
                        if let licence = credit.licence {
                            Text(licence).font(.caption).foregroundStyle(.readableSecondary)
                        }
                    }
                }
            }
        }
    }
}
