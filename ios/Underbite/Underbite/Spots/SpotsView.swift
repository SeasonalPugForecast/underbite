import SwiftUI

/// The spots, listed from the store; each opens its conditions. A plain list for now: areas,
/// search and editing come next. In a wide window the list and the conditions sit side by side.
struct SpotsView: View {
    @Environment(SpotStore.self) private var store
    @Environment(\.horizontalSizeClass) private var sizeClass
    @State private var selection: String?

    var body: some View {
        if sizeClass == .regular {
            HStack(spacing: 0) {
                NavigationStack { list { id in selection = id } }
                    .frame(maxWidth: 380)
                Divider()
                NavigationStack {
                    if let id = selection ?? store.activeID {
                        ConditionsView(spotID: id)
                    } else {
                        ContentUnavailableView("Choose a spot", systemImage: "water.waves")
                    }
                }
            }
        } else {
            NavigationStack {
                list { _ in }
                    .navigationDestination(item: $selection) { ConditionsView(spotID: $0) }
            }
        }
    }

    private func list(choose: @escaping (String) -> Void) -> some View {
        List {
            ForEach(store.arranged) { spot in
                Button {
                    choose(spot.id)
                    selection = spot.id
                } label: {
                    SpotRow(spot: spot, listing: store.listing(for: spot.id))
                }
                .buttonStyle(.plain)
            }
        }
        .overlay {
            if store.spots.isEmpty {
                ContentUnavailableView {
                    Label("No spots yet", systemImage: "mappin.and.ellipse")
                } description: {
                    Text("The places you fish will be listed here, each with its conditions.")
                }
            }
        }
        .refreshable { await store.refresh() }
        .navigationTitle("Spots")
    }
}

/// One spot of the list: its name, the kind of water and how fresh its data is, all as the
/// core words them.
private struct SpotRow: View {
    let spot: Spot
    let listing: SpotListing?

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(spot.name).font(.headline)
            if let label = listing?.waterTypeLabel {
                Text(label).font(.subheadline).foregroundStyle(.secondary)
            }
            SpotStatusLine(status: listing?.status)
        }
        .accessibilityElement(children: .combine)
    }
}

/// "updated 6 min ago", or the error, or a spinner while it fetches.
struct SpotStatusLine: View {
    let status: SpotListing.Status?

    var body: some View {
        HStack(spacing: 6) {
            if status?.refreshing == true { ProgressView().controlSize(.mini) }
            if let updated = status?.updated {
                Text(updated)
            } else if status?.refreshing != true {
                Text("Not updated yet")
            }
            if let error = status?.error {
                Text(error).foregroundStyle(.orange).lineLimit(2)
            }
        }
        .font(.caption)
        .foregroundStyle(.secondary)
    }
}
