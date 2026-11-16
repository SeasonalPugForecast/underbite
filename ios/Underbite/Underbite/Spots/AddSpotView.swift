import CoreLocation
import MapKit
import SwiftUI

/// Adds a spot four ways: where the phone is, a place found by search, a pin dropped on a small
/// map, or typed coordinates. The exact position stays in the store; the core rounds what a
/// fetch sends. Location is asked for only when the user taps "Use my location".
struct AddSpotView: View {
    @Environment(SpotStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    /// Called with the new spot's id once it is saved.
    let added: (String) -> Void

    private enum Way: String, CaseIterable, Identifiable {
        case here = "Here", search = "Search", map = "Map", typed = "Type"
        var id: String { rawValue }
    }

    @State private var way = Way.here
    @State private var chosen: CLLocationCoordinate2D?
    @State private var name = ""
    /// `nil`: let the core suggest one after the first fetch.
    @State private var waterType: String?
    @State private var areaID: String?

    @State private var finding = false
    @State private var findError: String?
    @State private var query = ""
    @State private var results: [MKMapItem] = []
    @State private var searching = false
    @State private var searchError: String?
    @State private var typed = ""
    @State private var camera = MapCameraPosition.automatic

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Picker("Add by", selection: $way) {
                        ForEach(Way.allCases) { Text($0.rawValue).tag($0) }
                    }
                    .pickerStyle(.segmented)
                    .accessibilityLabel("Add by")
                }
                switch way {
                case .here: here
                case .search: search
                case .map: map
                case .typed: typedCoordinates
                }
                Section("Spot") {
                    TextField("Name", text: $name)
                        .textInputAutocapitalization(.words)
                    Picker("Water", selection: $waterType) {
                        Text("Suggest one for me").tag(String?.none)
                        ForEach(store.waterTypeOptions, id: \.id) { option in
                            Text(option.label)
                                .accessibilityLabel(option.plainLabel ?? option.label)
                                .tag(Optional(option.id))
                        }
                    }
                    if !store.areas.isEmpty {
                        Picker("Area", selection: $areaID) {
                            Text("None").tag(String?.none)
                            ForEach(store.arrangedAreas) { Text($0.name).tag(Optional($0.id)) }
                        }
                    }
                }
            }
            .navigationTitle("Add a spot")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Add", action: save).disabled(!canSave)
                }
            }
            .onChange(of: typed) { _, text in
                chosen = TypedCoordinates.parse(text).map {
                    CLLocationCoordinate2D(latitude: $0.latitude, longitude: $0.longitude)
                }
            }
            .onChange(of: way) { _, now in
                // Each way keeps its own position: switching never carries a stale one over.
                chosen = nil
                if now == .typed {
                    chosen = TypedCoordinates.parse(typed).map {
                        CLLocationCoordinate2D(latitude: $0.latitude, longitude: $0.longitude)
                    }
                }
            }
        }
    }

    private var canSave: Bool {
        chosen != nil && !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    // MARK: - The four ways

    @ViewBuilder
    private var here: some View {
        Section {
            Button {
                Task { await findHere() }
            } label: {
                HStack {
                    Label("Use my location", systemImage: "location")
                    if finding { Spacer(); ProgressView() }
                }
            }
            .disabled(finding)
            if chosen != nil {
                Label("Location found", systemImage: "checkmark.circle")
                    .foregroundStyle(.green)
            }
            if let findError {
                Text(findError).foregroundStyle(.orange)
            }
        } footer: {
            Text("Your location is read once, and only kept if you save it as a spot.")
        }
    }

    @ViewBuilder
    private var search: some View {
        Section {
            HStack {
                TextField("Place or address", text: $query)
                    .submitLabel(.search)
                    .onSubmit { Task { await runSearch() } }
                if searching { ProgressView() }
            }
            ForEach(Array(results.enumerated()), id: \.offset) { _, item in
                Button {
                    chosen = item.location.coordinate
                    if name.trimmingCharacters(in: .whitespaces).isEmpty { name = item.name ?? "" }
                } label: {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(item.name ?? "Place")
                        if let address = item.address?.shortAddress {
                            Text(address).font(.caption).foregroundStyle(.secondary)
                        }
                    }
                }
                .foregroundStyle(.primary)
                .accessibilityAddTraits(
                    chosen.map { sameSpot($0, item.location.coordinate) } == true ? .isSelected : [])
            }
            if let searchError {
                Text(searchError).foregroundStyle(.orange)
            }
        } footer: {
            Text("What you type goes to Apple's Maps service to find the place. Underbite keeps nothing of it.")
        }
    }

    @ViewBuilder
    private var map: some View {
        Section {
            MapReader { proxy in
                Map(position: $camera) {
                    if let chosen {
                        Marker(name.isEmpty ? "Spot" : name, coordinate: chosen)
                    }
                }
                .onTapGesture { point in
                    if let coordinate = proxy.convert(point, from: .local) { chosen = coordinate }
                }
            }
            .frame(height: 280)
            .listRowInsets(EdgeInsets())
            .accessibilityLabel("Map: tap to drop a pin")
            if chosen != nil {
                Label("Pin dropped", systemImage: "mappin")
            }
        } footer: {
            Text("Tap the map to drop the pin; tap again to move it. Zoom in for the exact spot.")
        }
    }

    @ViewBuilder
    private var typedCoordinates: some View {
        Section {
            TextField("Latitude, longitude", text: $typed)
                .keyboardType(.numbersAndPunctuation)
                .autocorrectionDisabled()
            if !typed.isEmpty, chosen == nil {
                Text("Type latitude and longitude in decimal degrees, like 40.7033, -74.0170.")
                    .foregroundStyle(.orange)
            }
        } footer: {
            Text("Decimal degrees: north and east are positive.")
        }
    }

    // MARK: - Actions

    private func findHere() async {
        finding = true
        findError = nil
        defer { finding = false }
        do {
            try await store.locate()
            if let here = store.currentPlace {
                chosen = CLLocationCoordinate2D(latitude: here.latitude, longitude: here.longitude)
                if name.trimmingCharacters(in: .whitespaces).isEmpty { name = "Current location" }
            }
        } catch {
            findError = error.localizedDescription
        }
    }

    private func runSearch() async {
        let text = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        searching = true
        searchError = nil
        defer { searching = false }
        let request = MKLocalSearch.Request()
        request.naturalLanguageQuery = text
        if let here = store.currentPlace {
            request.region = MKCoordinateRegion(
                center: CLLocationCoordinate2D(latitude: here.latitude, longitude: here.longitude),
                latitudinalMeters: 200_000, longitudinalMeters: 200_000)
        }
        do {
            results = try await MKLocalSearch(request: request).start().mapItems
            if results.isEmpty { searchError = "Nothing found for \"\(text)\"." }
        } catch {
            results = []
            searchError = "The search did not answer. Check the connection and try again."
        }
    }

    private func sameSpot(_ a: CLLocationCoordinate2D, _ b: CLLocationCoordinate2D) -> Bool {
        a.latitude == b.latitude && a.longitude == b.longitude
    }

    private func save() {
        guard let chosen else { return }
        let spot = Spot(
            name: name.trimmingCharacters(in: .whitespacesAndNewlines),
            latitude: chosen.latitude, longitude: chosen.longitude,
            waterType: waterType ?? "river_stream", waterTypeChosen: waterType != nil,
            areaID: areaID)
        store.add(spot)
        // A spot with no chosen water type is fetched with inference on: the core suggests one.
        Task { await store.refresh(only: [spot.id]) }
        added(spot.id)
        dismiss()
    }
}
