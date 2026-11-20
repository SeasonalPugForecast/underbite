import SwiftUI
import UnderbiteKit

/// The badges of the layers on the map drawn from the device's store: each layer once, its title
/// and the core's words, "cached <date>" (the oldest copy drawn) or "not available offline".
/// Nothing shows while every layer draws from the network.
struct LayerBadges: View {
    let controller: MapController
    var status: MapOfflineStatus = .shared

    /// One layer's badge as shown.
    private struct Row: Identifiable {
        let id: String
        let title: String
        let text: String
    }

    /// The badged layers, topmost first, with the core's words.
    private var rows: [Row] {
        let json = controller.settingsJSON
        let now = Int64(Date().timeIntervalSince1970)
        let zone = TimeZone.current.identifier
        var notAvailable: String?
        var rows: [Row] = []
        for layer in controller.plan.layers.reversed() {
            let date = layer.kind == .tiles ? layer.date : layer.time.map { String($0) }
            guard let badge = status.badge(layer: layer.id, date: date) else { continue }
            let text: String?
            switch badge {
            case .cached(let fetchedAt):
                text = try? mapCachedLabel(
                    fetchedAt: fetchedAt, now: now, settingsJson: json, timeZone: zone)
            case .notAvailable:
                if notAvailable == nil {
                    notAvailable = (try? mapOfflineWords(settingsJson: json))?.notAvailable
                }
                text = notAvailable
            }
            guard let text else { continue }
            rows.append(Row(id: layer.id, title: layer.info.title, text: text))
        }
        return rows
    }

    var body: some View {
        let rows = rows
        if !rows.isEmpty {
            VStack(alignment: .leading, spacing: 4) {
                ForEach(rows) { row in
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        Image(systemName: "icloud.slash")
                            .foregroundStyle(.secondary)
                            .accessibilityHidden(true)
                        Text(row.title)
                            .font(.footnote.weight(.semibold))
                        Text(row.text)
                            .font(.footnote)
                    }
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityElement(children: .combine)
                    .accessibilityIdentifier("map-offline-\(row.id)")
                }
            }
            .foregroundStyle(Color.primary)
            .padding(.horizontal, 10)
            .padding(.vertical, 4)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 10))
            .dynamicTypeSize(...MapChromeType.largest)
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("map-offline-badges")
        }
    }
}
