import SwiftUI
import UnderbiteKit

/// The access group's own part of the layer drawer (a sheet on the iPhone, the side panel on
/// the iPad): "Find ramps and piers here", which alone looks the places up, then what the last
/// lookup said (busy, kept on this phone, zoom in), the notes on what access means and the
/// credits of the sources used. Every word is the core's.
struct AccessSection: View {
    let module: AccessModule
    let settingsJSON: String

    var body: some View {
        if let words = try? nearbyWords(settingsJson: settingsJSON) {
            Section {
                Button {
                    module.find()
                } label: {
                    HStack(spacing: 10) {
                        if module.busy {
                            ProgressView()
                        } else {
                            Image(systemName: "binoculars.fill")
                                .accessibilityHidden(true)
                        }
                        Text(module.busy ? words.findingAccess : words.findAccess)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                .disabled(module.busy)
                .accessibilityIdentifier("map-find-access")
                ForEach(module.notices, id: \.self) { notice in
                    Text(notice)
                        .font(.footnote)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityIdentifier("map-access-notice")
                }
                ForEach(module.notes, id: \.self) { note in
                    Text(note)
                        .font(.footnote)
                        .foregroundStyle(.readableSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                ForEach(module.credits, id: \.source) { credit in
                    NearbyCreditRow(credit: credit)
                }
            }
        }
    }
}

/// A source's credit as it asks for it, with the data's licence and a link to its terms.
struct NearbyCreditRow: View {
    let credit: NearbyCredit

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            if let url = credit.url.flatMap(URL.init(string:)) {
                Link(destination: url) {
                    Text(credit.text)
                        .multilineTextAlignment(.leading)
                }
            } else {
                Text(credit.text)
            }
            if let licence = credit.licence {
                Text(licence)
                    .foregroundStyle(.readableSecondary)
            }
        }
        .font(.caption)
        .fixedSize(horizontal: false, vertical: true)
        .accessibilityElement(children: .combine)
    }
}
