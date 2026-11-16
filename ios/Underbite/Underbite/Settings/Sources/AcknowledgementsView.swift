import SwiftUI

/// The open-source notices: the app's own licence and every crate in the Rust core with its
/// licence text. The list is `Acknowledgements.json`, written by `cargo xtask ios licenses`.
struct AcknowledgementsView: View {
    /// Read once, not per view update.
    private static let loaded = AcknowledgementList.load()
    private var notices: AcknowledgementList { Self.loaded }

    var body: some View {
        List {
            Section {
                Text("Underbite is free software under the MIT License. Copyright (c) 2026 Sean Avery.")
                    .font(.callout)
            }
            Section {
                ForEach(notices.crates) { crate in
                    NavigationLink {
                        ScrollView {
                            Text(notices.texts[crate.text])
                                .font(.system(.caption, design: .monospaced))
                                .padding()
                        }
                        .navigationTitle(crate.name)
                        .navigationBarTitleDisplayMode(.inline)
                    } label: {
                        VStack(alignment: .leading, spacing: 2) {
                            Text("\(crate.name) \(crate.version)")
                            Text(crate.license).font(.caption).foregroundStyle(.readableSecondary)
                        }
                    }
                }
            } header: {
                Text("Libraries")
            } footer: {
                Text("The Rust crates in the app's core.")
            }
        }
        .readableText()
        .navigationTitle("Acknowledgements")
        .navigationBarTitleDisplayMode(.inline)
    }
}

/// `Acknowledgements.json`: crates pointing into a list of distinct texts.
struct AcknowledgementList: Decodable {
    struct Crate: Decodable, Identifiable {
        let name: String
        let version: String
        let license: String
        /// Index into `texts`.
        let text: Int
        var id: String { "\(name) \(version)" }
    }

    let crates: [Crate]
    let texts: [String]

    /// The bundled list; empty when it is missing or unreadable.
    static func load() -> AcknowledgementList {
        guard let url = Bundle.main.url(forResource: "Acknowledgements", withExtension: "json"),
            let data = try? Data(contentsOf: url),
            let list = try? JSONDecoder().decode(AcknowledgementList.self, from: data),
            list.crates.allSatisfy({ list.texts.indices.contains($0.text) })
        else { return AcknowledgementList(crates: [], texts: []) }
        return list
    }
}
