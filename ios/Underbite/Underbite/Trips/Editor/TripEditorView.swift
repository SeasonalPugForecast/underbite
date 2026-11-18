import SwiftUI
import UnderbiteKit

/// Edits a trip, or starts one: its name, places from the saved spots, its days, species,
/// method, limits and notes. Nothing is kept until Save, which is there once the core finds
/// nothing wrong with the trip (`trip_problem`, whose words it shows).
struct TripEditorView: View {
    @Environment(SpotStore.self) private var spots
    @Environment(\.dismiss) private var dismiss
    let trips: TripStore
    /// The trip to edit; `nil` for a new one.
    let tripID: String?
    /// Called with the trip's id once it is saved.
    let saved: (String) -> Void

    @State private var draft = TripDraft(first: TripDates.day(of: Date()))
    @State private var loaded = false
    /// The core's words for the draft's days ("Nov 8 to Nov 10", "Starts in 3 days").
    @State private var preview: TripListing?
    @State private var saveError: String?

    var body: some View {
        let json = draft.json(spots: spots.spots, areas: spots.areas, settingsJSON: spots.settingsJSON)
        let problem = tripProblem(tripJson: json)
        Form {
            Section("Name") {
                TextField("Name", text: $draft.name, prompt: Text("Weekend on the Hudson"))
                    .textInputAutocapitalization(.words)
            }
            places
            dates(problem: problem)
            Section {
                TextField("Species", text: $draft.species, prompt: Text("Morone saxatilis"))
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .onSubmit { tidySpecies() }
            } header: {
                Text("Species")
            } footer: {
                Text("Scientific names, separated by commas.")
            }
            Section("How you fish") {
                Picker("Method", selection: $draft.method) {
                    ForEach(TripDraft.methods, id: \.self) { method in
                        Text(Self.methodName(method)).tag(method)
                    }
                }
                Picker("Limits", selection: $draft.limits) {
                    Text("From the settings").tag(String?.none)
                    ForEach(limitNames, id: \.self) { Text($0).tag(Optional($0)) }
                }
            }
            Section("Notes") {
                TextField("Notes", text: $draft.notes, axis: .vertical)
                    .lineLimit(3...)
            }
            if let problem {
                Section {
                    ProblemLine(text: problem)
                }
            }
            if let saveError {
                Section {
                    ProblemLine(text: saveError)
                }
            }
        }
        .readableText()
        .navigationTitle(tripID == nil ? "New trip" : "Edit trip")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                Button("Cancel") { dismiss() }
            }
            ToolbarItem(placement: .confirmationAction) {
                Button("Save") { save() }
                    .disabled(
                        problem != nil || draft.name.trimmingCharacters(in: .whitespaces).isEmpty)
            }
        }
        .onAppear(perform: load)
        // The days' words depend on the dates and the places' zones only.
        .task(id: problem == nil ? "\(draft.first) \(draft.last) \(draft.placeIDs)" : "") {
            guard problem == nil else {
                preview = nil
                return
            }
            let settings = spots.settingsJSON
            let now = Int64(Date().timeIntervalSince1970)
            let trip = SavedTrip(id: draft.id, json: json)
            preview = await Task.detached(priority: .userInitiated) {
                TripListing.build(trip, settingsJSON: settings, now: now)
            }.value
        }
    }

    // MARK: - Sections

    /// The saved spots to choose from, and any place of the trip that is not one of them.
    private var places: some View {
        Section {
            ForEach(spots.arranged) { spot in
                PlaceToggle(
                    name: spot.name, detail: waterLabel(spot.waterType),
                    chosen: draft.placeIDs.contains(spot.id)
                ) { draft.toggle(spot.id) }
            }
            ForEach(draft.otherPlaces(besides: spots.spots), id: \.id) { place in
                PlaceToggle(
                    name: place.name, detail: nil, chosen: draft.placeIDs.contains(place.id)
                ) { draft.toggle(place.id) }
            }
        } header: {
            Text("Places")
        } footer: {
            if spots.spots.isEmpty {
                Text("Add the places you fish on the Spots tab first.")
            } else {
                Text("Choose one or more of your spots.")
            }
        }
    }

    private func dates(problem: String?) -> some View {
        Section {
            DatePicker("First day", selection: day(\.first), displayedComponents: .date)
            DatePicker("Last day", selection: day(\.last), displayedComponents: .date)
        } header: {
            Text("Days")
        } footer: {
            if problem == nil, let preview, !preview.dates.isEmpty {
                VStack(alignment: .leading, spacing: 2) {
                    Text(preview.dates)
                    if !preview.status.isEmpty { Text(preview.status) }
                }
            } else {
                Text("Up to 31 days.")
            }
        }
    }

    /// A date picker's binding to a day of the draft: the day the picker shows, in the phone's
    /// calendar and zone, is the trip's date, whatever the places' zones are. Moving the first
    /// day moves the last with it, so the trip keeps its length.
    private func day(_ key: WritableKeyPath<TripDraft, String>) -> Binding<Date> {
        Binding {
            TripDates.date(of: draft[keyPath: key]) ?? Date()
        } set: { picked in
            let day = TripDates.day(of: picked)
            if key == \TripDraft.first, let span = TripDates.days(from: draft.first, to: draft.last),
                span >= 0, let last = TripDates.adding(span, to: day)
            {
                draft.last = last
            }
            draft[keyPath: key] = day
        }
    }

    // MARK: - Words

    /// The core's name for a kind of water.
    private func waterLabel(_ id: String) -> String? {
        spots.waterTypeOptions.first { $0.id == id }?.label
    }

    /// The limits profiles of the settings, and the copy the trip carries when its profile is
    /// gone from them.
    private var limitNames: [String] {
        var names = TripDraft.profileNames(spots.settingsJSON)
        if let kept = draft.limits, !names.contains(kept) { names.append(kept) }
        return names
    }

    private static func methodName(_ method: String) -> String {
        switch method {
        case "shore": "From shore"
        case "wade": "Wading"
        case "kayak": "Kayak"
        case "boat": "Boat"
        case "ice": "On the ice"
        default: method
        }
    }

    // MARK: - Actions

    private func load() {
        guard !loaded else { return }
        loaded = true
        if let tripID, let trip = trips.trip(tripID) {
            draft = TripDraft(json: trip.json)
        } else {
            draft = TripDraft(
                first: TripDates.day(of: Date()), places: spots.activeID.map { [$0] } ?? [])
        }
    }

    /// The species as the ids the trip keeps.
    private func tidySpecies() {
        draft.species = TripDraft.speciesIDs(draft.species).joined(separator: ", ")
    }

    private func save() {
        var final = draft
        final.species = TripDraft.speciesIDs(draft.species).joined(separator: ", ")
        let json = final.json(spots: spots.spots, areas: spots.areas, settingsJSON: spots.settingsJSON)
        do {
            let id = try trips.save(json)
            saved(id)
            dismiss()
        } catch {
            saveError = Self.message(of: error)
        }
    }

    /// The core's words for why a trip could not be saved.
    private static func message(of error: any Error) -> String {
        switch error {
        case ShareError.NeedsNewerVersion(let message), ShareError.DatesBackwards(let message),
            ShareError.TooLong(let message), ShareError.Unreadable(let message),
            ShareError.TooLarge(let message), ShareError.Unsupported(_, let message),
            ShareError.Invalid(_, let message):
            message
        default: error.localizedDescription
        }
    }
}

/// A place to choose: a row with a check mark when it is chosen.
private struct PlaceToggle: View {
    let name: String
    let detail: String?
    let chosen: Bool
    let toggle: () -> Void

    var body: some View {
        Button(action: toggle) {
            HStack {
                VStack(alignment: .leading, spacing: 2) {
                    Text(name).foregroundStyle(Color.primary)
                    if let detail {
                        Text(detail).font(.caption).foregroundStyle(.readableSecondary)
                    }
                }
                Spacer()
                if chosen {
                    Image(systemName: "checkmark")
                        .foregroundStyle(Palette.accent)
                        .accessibilityHidden(true)
                }
            }
            .contentShape(Rectangle())
        }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(chosen ? .isSelected : [])
    }
}

/// What is wrong with the trip, in the core's words.
private struct ProblemLine: View {
    let text: String

    var body: some View {
        Label {
            Text(text).fixedSize(horizontal: false, vertical: true)
        } icon: {
            Image(systemName: "exclamationmark.triangle.fill")
                .foregroundStyle(Palette.warning)
                .accessibilityHidden(true)
        }
    }
}
