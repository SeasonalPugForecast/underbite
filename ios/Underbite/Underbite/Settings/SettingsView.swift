import SwiftUI
import UnderbiteKit

/// A page of the Settings root.
enum SettingsDestination: Hashable {
    case sources, keys, cards, about, attribution, acknowledgements
    case preference(PreferencePageID)

    /// The page `-settings <name>` asks for, in debug builds only (screenshots). `-preferences
    /// <page>` asks for one of the preference pages.
    static var launchRequest: SettingsDestination? {
        #if DEBUG
            if let page = PreferencePageID.launchRequest { return .preference(page) }
            switch LaunchArguments.value(after: "-settings") {
            case "sources": return .sources
            case "keys": return .keys
            case "cards": return .cards
            case "about": return .about
            case "attribution": return .attribution
            case "acknowledgements": return .acknowledgements
            default: return nil
            }
        #else
            nil
        #endif
    }
}

/// Sources, units and limits, cards, voice, privacy and attribution. A split view: the list
/// in the first column and the page in the detail column on the iPad (both show at once), a
/// stack on the iPhone and in a narrow iPad window, where choosing a row pushes its page.
struct SettingsView: View {
    @Environment(\.horizontalSizeClass) private var sizeClass
    @State private var selection: SettingsDestination?
    @State private var columns: NavigationSplitViewVisibility = .all
    @State private var compactColumn: NavigationSplitViewColumn = .sidebar
    @State private var showsWelcome = false

    var body: some View {
        NavigationSplitView(columnVisibility: $columns, preferredCompactColumn: $compactColumn) {
            list
        } detail: {
            NavigationStack {
                page
                    .preferenceDestinations()
            }
        }
        .navigationSplitViewStyle(.balanced)
        .onAppear {
            #if DEBUG
                if let asked = SettingsDestination.launchRequest {
                    selection = asked
                    return
                }
                if LaunchArguments.value(after: "-settings") == "welcome" {
                    showsWelcome = true
                    return
                }
            #endif
            if sizeClass == .regular, selection == nil { selection = .sources }
        }
        .onChange(of: sizeClass) { _, now in
            if now == .regular, selection == nil { selection = .sources }
        }
        .onChange(of: selection) { _, chosen in
            compactColumn = chosen == nil ? .sidebar : .detail
        }
        .onChange(of: compactColumn) { _, column in
            // Coming back to the list in the compact layout leaves nothing chosen, so the
            // same row can be chosen again.
            if column == .sidebar, sizeClass != .regular { selection = nil }
        }
        .sheet(isPresented: $showsWelcome) {
            WelcomeView { showsWelcome = false }
        }
    }

    private var list: some View {
        List(selection: $selection) {
            Section {
                Label("Sources", systemImage: "list.number").tag(SettingsDestination.sources)
                Label("Keys and accounts", systemImage: "key").tag(SettingsDestination.keys)
            } header: {
                Text("Data")
            }
            Section {
                row(.units)
                row(.limits)
                row(.rivers)
                row(.fishing)
                row(.freshness)
                row(.refresh)
            } header: {
                Text("Units and limits")
            }
            Section {
                Label("Cards", systemImage: "rectangle.grid.1x2").tag(SettingsDestination.cards)
                row(.voice)
                row(.privacy)
            } header: {
                Text("Look and privacy")
            }
            Section {
                Label("About", systemImage: "pawprint").tag(SettingsDestination.about)
                Button {
                    showsWelcome = true
                } label: {
                    Label("Welcome", systemImage: "hand.wave")
                }
                .foregroundStyle(.primary)
                Label("Attribution", systemImage: "text.badge.checkmark")
                    .tag(SettingsDestination.attribution)
                Label("Acknowledgements", systemImage: "doc.text")
                    .tag(SettingsDestination.acknowledgements)
            } header: {
                Text("About")
            }
        }
        .navigationTitle("Settings")
    }

    private func row(_ page: PreferencePageID) -> some View {
        Label(page.title, systemImage: page.systemImage).tag(SettingsDestination.preference(page))
    }

    @ViewBuilder
    private var page: some View {
        switch selection {
        case .sources: SourcesView()
        case .keys: KeysView()
        case .cards: CustomizeView()
        case .about: AboutView()
        case .attribution: AttributionView()
        case .acknowledgements: AcknowledgementsView()
        case .preference(let page): page.destination
        case nil:
            ContentUnavailableView(
                "Choose a setting", systemImage: "gearshape",
                description: Text("Its page shows here."))
        }
    }
}
