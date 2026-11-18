import SwiftUI

/// A trip's page: its dates and status, its places and notes, as the core words them, with the
/// editor a tap away.
struct TripSummaryView: View {
    let trips: TripStore
    let tripID: String
    let edit: () -> Void

    var body: some View {
        List {
            if let trip = trips.listing(tripID) {
                Section {
                    VStack(alignment: .leading, spacing: 4) {
                        if !trip.dates.isEmpty { Text(trip.dates).font(.title3.bold()) }
                        if !trip.status.isEmpty {
                            Text(trip.status).foregroundStyle(.readableSecondary)
                        }
                    }
                    .accessibilityElement(children: .combine)
                    if let problem = trip.problem {
                        Label {
                            Text(problem).fixedSize(horizontal: false, vertical: true)
                        } icon: {
                            Image(systemName: "exclamationmark.triangle.fill")
                                .foregroundStyle(Palette.warning)
                                .accessibilityHidden(true)
                        }
                    }
                }
                Section("Places") {
                    ForEach(trip.places, id: \.key) { place in
                        PlaceLine(place: place)
                    }
                }
                if !trip.notes.isEmpty {
                    Section("Notes") {
                        Text(trip.notes)
                    }
                }
            } else {
                ProgressView().frame(maxWidth: .infinity)
            }
        }
        .readableText()
        .navigationTitle(trips.listing(tripID)?.title ?? "")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button("Edit", action: edit)
            }
        }
    }
}

/// A place of a trip: its name, its area and kind of water, and whether it is approximate.
private struct PlaceLine: View {
    let place: TripListing.Place

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(place.name).font(.headline)
            if let area = place.area {
                Text(area).font(.subheadline).foregroundStyle(.readableSecondary)
            }
            Text(place.waterTypeLabel).font(.subheadline).foregroundStyle(.readableSecondary)
            if let approximate = place.approximate {
                Text(approximate).font(.caption).foregroundStyle(.readableSecondary)
            }
            if let problem = place.problem {
                Text(problem).font(.caption).foregroundStyle(Palette.warning)
            }
        }
        .accessibilityElement(children: .combine)
    }
}
