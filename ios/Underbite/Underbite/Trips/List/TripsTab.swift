import Combine
import SwiftUI
@preconcurrency import UnderbiteKit

/// What the Trips screen has open over it.
private enum TripsSheet: Identifiable {
    case new
    case edit(String)

    var id: String {
        switch self {
        case .new: "new"
        case .edit(let id): "edit-" + id
        }
    }
}

/// The trips, upcoming and past. A split view: the list beside the chosen trip on the iPad, a
/// stack on the iPhone and in a narrow iPad window. The selection is the trip's id, so it
/// survives the collapse to the compact layout and back.
struct TripsTab: View {
    @Environment(SpotStore.self) private var spots
    @Environment(\.horizontalSizeClass) private var sizeClass
    @Environment(\.scenePhase) private var phase
    var trips: TripStore = .shared
    @State private var selection: String?
    @State private var columns: NavigationSplitViewVisibility = .all
    @State private var compactColumn: NavigationSplitViewColumn = .sidebar
    @State private var sheet: TripsSheet?
    /// The phone's date, `YYYY-MM-DD`: a trip's status and today's day follow it.
    @State private var today = TripDates.day(of: Date())

    var body: some View {
        NavigationSplitView(columnVisibility: $columns, preferredCompactColumn: $compactColumn) {
            TripList(trips: trips, selection: $selection, sheet: $sheet)
        } detail: {
            NavigationStack {
                if let id = selection, trips.trip(id) != nil {
                    TripGridView(trips: trips, tripID: id, today: today) { sheet = .edit(id) }
                        .id(id)
                } else {
                    ContentUnavailableView(
                        "Choose a trip", systemImage: "calendar",
                        description: Text("Its places and days show here."))
                }
            }
        }
        .navigationSplitViewStyle(.balanced)
        // The listings follow the trips, the settings (units, voice) and the day.
        .task(id: "\(trips.revision) \(spots.settingsJSON.hashValue) \(today)") {
            await trips.rebuild(settingsJSON: spots.settingsJSON)
        }
        // A new day, while the app is open or while it was away.
        .onReceive(
            NotificationCenter.default.publisher(for: .NSCalendarDayChanged)
                .receive(on: RunLoop.main)
        ) { _ in
            today = TripDates.day(of: Date())
        }
        .onChange(of: phase) { _, phase in
            if phase == .active { today = TripDates.day(of: Date()) }
        }
        .onAppear {
            if sizeClass == .regular, selection == nil { selection = firstShown }
            #if DEBUG
                TripsLaunch.apply(trips: trips, open: { selection = $0 }, edit: { sheet = .edit($0) })
            #endif
        }
        .onChange(of: sizeClass) { _, now in
            if now == .regular, selection == nil { selection = firstShown }
        }
        .onChange(of: trips.listings) { _, _ in
            if sizeClass == .regular, selection == nil { selection = firstShown }
        }
        .onChange(of: selection) { _, id in
            compactColumn = id == nil ? .sidebar : .detail
        }
        .onChange(of: trips.trips) { _, saved in
            // A trip deleted while it is open leaves the detail column with nothing.
            if let id = selection, !saved.contains(where: { $0.id == id }) { selection = nil }
        }
        .sheet(item: $sheet) { sheet in
            NavigationStack {
                switch sheet {
                case .new:
                    TripEditorView(trips: trips, tripID: nil) { selection = $0 }
                case .edit(let id):
                    TripEditorView(trips: trips, tripID: id) { selection = $0 }
                }
            }
        }
    }

    /// The trip the iPad shows before one is chosen: the next one, else the latest past one.
    private var firstShown: String? {
        (trips.upcoming.first ?? trips.past.first)?.id
    }
}

/// The list column: upcoming trips soonest first, then past ones latest first; swipe to delete.
/// No trip can be added while the store cannot save one.
private struct TripList: View {
    @Environment(SpotStore.self) private var spots
    let trips: TripStore
    @Binding var selection: String?
    @Binding var sheet: TripsSheet?
    @State private var deleting: TripListing?

    var body: some View {
        let upcoming = trips.upcoming
        let past = trips.past
        List(selection: $selection) {
            if trips.unreadable {
                Section { unreadableNotice }
            }
            if !upcoming.isEmpty {
                Section("Upcoming") { rows(upcoming) }
            }
            if !past.isEmpty {
                Section("Past") { rows(past) }
            }
        }
        .overlay {
            if trips.trips.isEmpty { emptyState }
        }
        .navigationTitle("Trips")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button("New trip", systemImage: "plus") { sheet = .new }
                    .disabled(!trips.canSave)
            }
        }
        .confirmationDialog(
            deleting.map { String(localized: "Delete \($0.title)?", comment: "Question asking to delete a spot or a trip; the argument is its name") } ?? String(localized: "Delete trip?", comment: "Question asking to delete a trip"),
            isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }),
            titleVisibility: .visible, presenting: deleting
        ) { trip in
            Button("Delete trip", role: .destructive) { trips.remove(trip.id) }
        } message: { _ in
            Text("Its places, days and notes are deleted from this device. The spots stay.")
        }
    }

    private func rows(_ listings: [TripListing]) -> some View {
        ForEach(listings) { trip in
            TripRow(trip: trip)
                .tag(trip.id)
                .contextMenu {
                    Button("Edit", systemImage: "pencil") { sheet = .edit(trip.id) }
                    Divider()
                    Button("Delete", systemImage: "trash", role: .destructive) { deleting = trip }
                }
                .swipeActions(edge: .trailing) {
                    Button(role: .destructive) {
                        deleting = trip
                    } label: {
                        Label("Delete", systemImage: "trash")
                    }
                    Button {
                        sheet = .edit(trip.id)
                    } label: {
                        Label("Edit", systemImage: "pencil")
                    }
                }
        }
    }

    private var unreadableNotice: some View {
        Label {
            Text(fileProblemText(file: .trips, keptAs: trips.keptAs))
                .fixedSize(horizontal: false, vertical: true)
        } icon: {
            Image(systemName: "exclamationmark.triangle.fill")
                .foregroundStyle(Palette.warning)
                .accessibilityHidden(true)
        }
        .font(.subheadline)
    }

    private var emptyState: some View {
        VStack(spacing: 0) {
            LunaEmptyState(kind: .trips)
            Button("New trip") { sheet = .new }
                .disabled(!trips.canSave)
                .buttonStyle(.borderedProminent)
                .controlSize(.large)
                .padding(.bottom, 32)
        }
    }
}

/// One trip of the list: its name, its dates and status, and its places, as the core words them.
private struct TripRow: View {
    let trip: TripListing

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(trip.title).font(.headline)
            if !trip.dates.isEmpty {
                Text(trip.dates).font(.subheadline)
            }
            if !trip.status.isEmpty {
                Text(trip.status).font(.subheadline).foregroundStyle(.readableSecondary)
            }
            if !trip.places.isEmpty {
                Text(trip.places.map(\.name).joined(separator: ", "))
                    .font(.caption)
                    .foregroundStyle(.readableSecondary)
            }
            if let problem = trip.problem {
                Label {
                    Text(problem).fixedSize(horizontal: false, vertical: true)
                } icon: {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .foregroundStyle(Palette.warning)
                        .accessibilityHidden(true)
                }
                .font(.caption)
            }
        }
        .padding(.vertical, 2)
        .accessibilityElement(children: .combine)
    }
}

#if DEBUG
    /// Launch arguments of the screenshot runs: `-openTrip <id>` opens that trip, `-editTrip <id>`
    /// opens it in the editor.
    enum TripsLaunch {
        private static var applied = false

        static func apply(trips: TripStore, open: (String) -> Void, edit: (String) -> Void) {
            guard !applied else { return }
            applied = true
            if let id = LaunchArguments.value(after: "-openTrip"), trips.trip(id) != nil { open(id) }
            if let id = LaunchArguments.value(after: "-editTrip"), trips.trip(id) != nil {
                open(id)
                edit(id)
            }
        }
    }
#endif
