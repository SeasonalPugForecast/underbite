import SwiftUI
import UIKit
@preconcurrency import UnderbiteKit

/// One catch in full, as the core's detail words it: size, time, place and fate, its photos, the
/// lure, method and notes, and the conditions at the time, each with its source.
struct CatchDetailScreen: View {
    @Environment(SpotStore.self) private var spots
    let catches: CatchStore
    let id: String
    /// Opens the catch in the editor.
    var edit: (() -> Void)?
    @State private var detail: CatchDetail?
    @State private var loaded = false
    @State private var deleting = false

    var body: some View {
        Group {
            if let detail {
                content(detail)
            } else if loaded {
                ContentUnavailableView(
                    "This catch is gone", systemImage: "fish",
                    description: Text("It was deleted, or the log could not be read."))
            } else {
                ProgressView()
            }
        }
        .navigationTitle(detail?.row.species ?? "")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            if detail != nil, let edit {
                ToolbarItem(placement: .primaryAction) {
                    Button("Edit", systemImage: "pencil", action: edit)
                }
            }
            if detail != nil {
                ToolbarItem(placement: .destructiveAction) {
                    Button("Delete", systemImage: "trash", role: .destructive) { deleting = true }
                }
            }
        }
        .confirmationDialog("Delete this catch?", isPresented: $deleting, titleVisibility: .visible) {
            Button("Delete catch", role: .destructive) { catches.delete(id) }
        } message: {
            Text("Its photos, notes and conditions are deleted from this device.")
        }
        .task(id: "\(catches.revision) \(spots.settingsJSON.hashValue)") {
            let places = spots.spots.map { PlaceName(spotId: $0.id, name: $0.name) }
            detail = catches.detail(id, settingsJSON: spots.settingsJSON, places: places)
            loaded = true
        }
    }

    private func content(_ detail: CatchDetail) -> some View {
        List {
            Section { header(detail.row) }
            if !detail.photos.isEmpty {
                Section {
                    ScrollView(.horizontal) {
                        HStack(spacing: 8) {
                            ForEach(detail.photos, id: \.self) { name in
                                CatchPhoto(url: catches.photoURL(name))
                            }
                        }
                    }
                    .scrollIndicators(.hidden)
                }
            }
            if !detail.details.isEmpty {
                Section {
                    ForEach(detail.details, id: \.self) { line in
                        LabeledContent {
                            Text(line.value).multilineTextAlignment(.trailing)
                        } label: {
                            Text(line.label)
                        }
                    }
                }
            }
            Section(detail.conditionsTitle) {
                if let none = detail.noConditions {
                    Text(none).foregroundStyle(.readableSecondary)
                }
                ForEach(detail.conditions, id: \.self) { line in
                    ConditionRow(line: line)
                }
            }
        }
    }

    private func header(_ row: CatchRow) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline) {
                Text(row.species).font(.title2.bold())
                Spacer(minLength: 8)
                CatchFateTag(row: row)
            }
            if let size = row.size {
                Text(size).font(.title3)
            }
            Text(row.when).foregroundStyle(.readableSecondary)
            if let place = row.place {
                Label(place, systemImage: "mappin.and.ellipse")
                    .foregroundStyle(.readableSecondary)
            }
        }
        .padding(.vertical, 4)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(row.summary))
    }
}

/// A condition at the catch: its label and value, and where it came from.
private struct ConditionRow: View {
    let line: CatchDetail.Condition

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            LabeledContent {
                Text(line.value).multilineTextAlignment(.trailing)
            } label: {
                Text(line.label)
            }
            if !line.footnote.isEmpty {
                Text(line.footnote)
                    .font(.caption)
                    .foregroundStyle(.readableSecondary)
            }
        }
        .accessibilityElement(children: .combine)
    }
}

/// A photo of the catch, read from the app's container as a thumbnail.
struct CatchPhoto: View {
    let url: URL
    /// The side of the square, in points.
    var side: CGFloat = 120
    @State private var image: UIImage?

    var body: some View {
        Group {
            if let image {
                Image(uiImage: image)
                    .resizable()
                    .scaledToFill()
            } else {
                Rectangle().fill(.quaternary)
            }
        }
        .frame(width: side, height: side)
        .clipShape(RoundedRectangle(cornerRadius: 10))
        .accessibilityElement()
        .accessibilityLabel(Text("Photo of the catch"))
        .accessibilityAddTraits(.isImage)
        .task(id: url) {
            image = UIImage(contentsOfFile: url.path)?.preparingThumbnail(
                of: CGSize(width: side * 2, height: side * 2))
        }
    }
}
