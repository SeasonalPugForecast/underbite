import SwiftUI
import UnderbiteKit

/// The conditions of one spot: its cards in the user's order, marine alerts on top, each card
/// with its provenance footnote and credits. Everything shown comes from the core's view model,
/// decoded once per build of the page.
struct ConditionsView: View {
    @Environment(SpotStore.self) private var store
    @Environment(\.horizontalSizeClass) private var sizeClass
    let spotID: String

    @State private var loaded: LoadedPage?
    @State private var firstShowDone = false
    /// This page's own token with the store, which builds a page for each spot on screen: two
    /// iPad windows may show two spots.
    @State private var viewer = UUID()
    @State private var customizing = false
    @State private var openCard: ViewModel.Card?

    var body: some View {
        let spot = store.spot(spotID)
        content
            .background(Color(.systemGroupedBackground))
            .navigationTitle(spot?.name ?? "Conditions")
            .navigationBarTitleDisplayMode(.inline)
            .navigationDestination(for: ViewModel.Card.self) { CardDetailView(card: $0) }
            .navigationDestination(item: $openCard) { CardDetailView(card: $0) }
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    // Outdoors, one tap away: high contrast and heavier lines for bright sun.
                    Toggle(
                        "Sun mode", systemImage: "sun.max",
                        isOn: Binding(get: { store.sunMode }, set: { store.setSunMode($0) })
                    )
                    .toggleStyle(.button)
                }
                ToolbarItem(placement: .primaryAction) {
                    Button("Customize", systemImage: "slider.horizontal.3") { customizing = true }
                }
            }
            .sheet(isPresented: $customizing) {
                CustomizeSheet(
                    waterType: spot?.waterType, waterTypeLabel: pageView?.header.waterTypeLabel)
            }
            .task(id: spotID) {
                firstShowDone = false
                await store.show(spotID, viewer: viewer)
                firstShowDone = true
            }
            .onDisappear { store.hide(viewer: viewer) }
            .task {
                // A launch argument opens the sheet once the page has finished appearing: a sheet
                // asked for while the page is still being pushed aborts the app.
                guard ConditionsLaunch.opensCustomize else { return }
                try? await Task.sleep(for: .seconds(2))
                customizing = true
            }
            .task(id: loaded?.json) {
                // Debug builds only: `-open-card <id>` opens that card's page once the page has
                // loaded, for screenshots.
                #if DEBUG
                    let arguments = ProcessInfo.processInfo.arguments
                    guard let flag = arguments.firstIndex(of: "-open-card"),
                        arguments.indices.contains(flag + 1), openCard == nil,
                        let view = pageView
                    else { return }
                    try? await Task.sleep(for: .seconds(1))
                    openCard = view.cards.first { $0.chrome?.id == arguments[flag + 1] }
                #endif
            }
            .task(id: store.pageJSON(for: spotID)) {
                let json = store.pageJSON(for: spotID)
                // Let go while a card's page was pushed over this one: the last one stays
                // until the store builds it again.
                guard !json.isEmpty else { return }
                let settings = store.settingsJSON
                loaded = await Task.detached(priority: .userInitiated) {
                    LoadedPage.make(json: json, settingsJSON: settings)
                }.value
            }
    }

    private var pageView: ViewModel.SpotView? { loaded?.view(for: spotID) }

    private var isLoading: Bool {
        !firstShowDone || loaded?.json != store.pageJSON(for: spotID)
            || store.refreshing.contains(spotID)
    }

    @ViewBuilder private var content: some View {
        ScrollView {
            if let view = pageView, let loaded {
                dashboard(view, loaded: loaded)
            } else {
                Group {
                    if isLoading {
                        LunaLoading()
                    } else if let problem = loaded?.problem(for: spotID) {
                        PageProblemView(problem: problem)
                    } else {
                        LunaEmptyState(kind: .conditions)
                    }
                }
                .containerRelativeFrame(.vertical)
            }
        }
        .refreshable { await store.refresh(only: [spotID]) }
    }

    private var columns: [GridItem] {
        sizeClass == .regular
            ? [GridItem(.adaptive(minimum: 340, maximum: 520), spacing: 12, alignment: .top)]
            : [GridItem(.flexible(), spacing: 12, alignment: .top)]
    }

    private func dashboard(_ view: ViewModel.SpotView, loaded: LoadedPage) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            PageHeader(
                header: view.header, notices: view.notices, status: loaded.status(for: spotID))
            ForEach(loaded.alerts(in: view), id: \.chrome.id) { alerts in
                NavigationLink(value: ViewModel.Card.alerts(alerts)) {
                    AlertsBanner(card: alerts)
                }
                .buttonStyle(.plain)
            }
            if let timeline = view.timeline {
                DayTimelineView(
                    timeline: timeline,
                    shownCards: Set(view.cards.compactMap { $0.chrome?.id }))
            }
            LazyVGrid(columns: columns, spacing: 12) {
                ForEach(LoadedPage.entries(in: view)) { entry in
                    NavigationLink(value: entry.card) {
                        CardView(card: entry.card)
                    }
                    .buttonStyle(.plain)
                    .contextMenu { menu(for: entry, choices: loaded.choices) }
                }
            }
        }
        .padding()
        .frame(maxWidth: 1180)
        .frame(maxWidth: .infinity)
    }

    /// Touch and hold: hide the card, switch any card on or off, or open Customize.
    @ViewBuilder private func menu(for entry: CardEntry, choices: CardChoices?) -> some View {
        Button("Hide \(entry.title)", systemImage: "eye.slash") { setShown(entry.id, false) }
        if let choices {
            Menu("Cards", systemImage: "checklist") {
                ForEach(choices.editable) { choice in
                    Toggle(
                        choice.title,
                        isOn: Binding(
                            get: { choice.shown }, set: { setShown(choice.id, $0) }))
                }
            }
        }
        Button("Customize…", systemImage: "slider.horizontal.3") { customizing = true }
    }

    /// Shows or hides a card for this spot's water type, where its layout is written.
    private func setShown(_ id: String, _ shown: Bool) {
        guard let waterType = store.spot(spotID)?.waterType,
            let settings = try? Settings(json: store.settingsJSON),
            let changed = try? CardLayoutEditing.setting(
                id, shown: shown, in: settings, waterType: waterType),
            let json = try? changed.json(),
            (try? store.updateSettings(json)) != nil
        else { return }
        Task { await store.rebuildPage() }
    }
}

/// The page of the shown spot, decoded off the main thread once per build.
nonisolated struct LoadedPage: Sendable {
    /// The page JSON this was decoded from.
    var json: String
    var page: ViewModel.Page
    /// The cards of the shown spot's water type, for the quick toggles.
    var choices: CardChoices?

    static func make(json: String, settingsJSON: String) -> LoadedPage? {
        guard !json.isEmpty, let page = try? ViewModel.decode(json) else { return nil }
        var choices: CardChoices?
        if let waterType = page.view?.header.waterType,
            let settings = try? Settings(json: settingsJSON)
        {
            choices = try? CardLayoutEditing.choices(settings: settings, waterType: waterType)
        }
        return LoadedPage(json: json, page: page, choices: choices)
    }

    /// The page's view when it is the one of spot `id` (the page holds one spot at a time).
    func view(for id: String) -> ViewModel.SpotView? {
        guard page.spots.indices.contains(page.active), page.spots[page.active].id == id
        else { return nil }
        return page.view
    }

    func status(for id: String) -> ViewModel.SpotStatus? {
        page.spots.first { $0.id == id }?.status
    }

    /// Why spot `id` has no page, when the page is its own and gives a reason.
    func problem(for id: String) -> ViewModel.PageProblem? {
        guard page.spots.indices.contains(page.active), page.spots[page.active].id == id
        else { return nil }
        return page.problem
    }

    /// The alerts cards, shown on top rather than in the grid.
    func alerts(in view: ViewModel.SpotView) -> [ViewModel.AlertsCard] {
        view.cards.compactMap { card in
            if case .alerts(let alerts) = card, !alerts.full.alerts.isEmpty { return alerts }
            return nil
        }
    }

    /// The cards of the grid: known kinds only (a kind of a later version is skipped quietly),
    /// without the alerts.
    static func entries(in view: ViewModel.SpotView) -> [CardEntry] {
        view.cards.compactMap { card in
            if case .alerts = card { return nil }
            guard card.isKnown, let chrome = card.chrome, card.compact != nil else { return nil }
            return CardEntry(id: chrome.id, title: chrome.title, card: card)
        }
    }
}

nonisolated struct CardEntry: Identifiable {
    var id: String
    var title: String
    var card: ViewModel.Card
}

/// One card's compact form in its frame.
struct CardView: View {
    let card: ViewModel.Card

    var body: some View {
        if let chrome = card.chrome, let compact = card.compact {
            CardFrame(chrome: chrome) { CompactBody(compact: compact) }
        }
    }
}

/// The water, when the data was updated, the errors line and the page's notices.
private struct PageHeader: View {
    let header: ViewModel.SpotHeader
    let notices: [ViewModel.SpotNotice]
    let status: ViewModel.SpotStatus?

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 6) {
                Text(header.waterTypeLabel)
                Text("·").accessibilityHidden(true)
                Text(status?.updated ?? header.updated)
                if status?.refreshing == true { ProgressView().controlSize(.mini) }
            }
            .font(.subheadline)
            .foregroundStyle(.readableSecondary)
            if let error = status?.error {
                Label(error, systemImage: "exclamationmark.triangle")
                    .font(.caption)
                    .foregroundStyle(Palette.warning)
                    .fixedSize(horizontal: false, vertical: true)
            }
            ForEach(Array(notices.enumerated()), id: \.offset) { _, notice in
                Text(notice.text)
                    .font(.caption)
                    .foregroundStyle(.readableSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// A spot with no conditions to show and a reason: the core's title and the reason (the last
/// fetch's error, or why the saved conditions could not be read), whole. Pull to refresh tries
/// again.
private struct PageProblemView: View {
    let problem: ViewModel.PageProblem

    var body: some View {
        VStack(spacing: 12) {
            Image(systemName: "exclamationmark.triangle")
                .font(.largeTitle)
                .foregroundStyle(Palette.warning)
                .accessibilityHidden(true)
            Text(problem.title)
                .font(.title3.bold())
                .multilineTextAlignment(.center)
            Text(problem.reason)
                .foregroundStyle(.readableSecondary)
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: 420)
        .padding()
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .accessibilityElement(children: .combine)
    }
}

/// Marine alerts and advisories at the top of the page: the event, the National Weather
/// Service's own headline and times, whole. Safety text, never in Luna's voice; the full
/// description and instruction are one tap away.
struct AlertsBanner: View {
    let card: ViewModel.AlertsCard

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            ForEach(card.full.alerts) { alert in
                HStack(alignment: .top, spacing: 10) {
                    Image(systemName: Self.symbol(alert.severity))
                        .foregroundStyle(Self.tint(alert.severity))
                        .accessibilityHidden(true)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(alert.headline ?? alert.event).font(.headline)
                        // Primary on the tinted banner, where secondary text would be faint.
                        if let span = Self.span(alert) {
                            Text(span).font(.subheadline)
                        }
                        Text(alert.severityLabel).font(.caption.weight(.medium))
                    }
                    .fixedSize(horizontal: false, vertical: true)
                    Spacer(minLength: 0)
                    Image(systemName: "chevron.right")
                        .font(.footnote.weight(.semibold))
                        .foregroundStyle(.readableTertiary)
                        .accessibilityHidden(true)
                }
            }
            NoticeList(notices: card.chrome.notices, onTint: true)
            CreditsLine(credits: card.chrome.credits, onTint: true)
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            Self.tint(card.full.alerts.first?.severity ?? "").opacity(0.14),
            in: RoundedRectangle(cornerRadius: 16, style: .continuous)
        )
        .overlay(
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .strokeBorder(Self.tint(card.full.alerts.first?.severity ?? "").opacity(0.5))
        )
        .accessibilityElement(children: .combine)
        .accessibilityLabel(Text(card.chrome.accessibility))
    }

    /// The span the alert is in effect, as the view wrote the times.
    static func span(_ alert: ViewModel.Alert) -> String? {
        let end = alert.ends ?? alert.expires
        switch (alert.onset, end) {
        case (let start?, let end?): return "\(start) – \(end)"
        case (let start?, nil): return start
        case (nil, let end?): return end
        case (nil, nil): return nil
        }
    }

    /// A warning's shape as well as its colour: an octagon for the severe ones.
    static func symbol(_ severity: String) -> String {
        switch severity {
        case "extreme", "severe": "exclamationmark.octagon.fill"
        default: "exclamationmark.triangle.fill"
        }
    }

    static func tint(_ severity: String) -> Color {
        switch severity {
        case "extreme", "severe": .red
        default: .orange
        }
    }
}
