import SwiftUI
import UnderbiteKit

/// The depth module's part of the layer drawer (the side panel on the iPad, the sheet on the
/// iPhone): what is said in place of depth (zoom in, loading, failed), the contours' interval,
/// the legend with the datums of the view, the NCEI credit and "Not for navigation.", and the
/// depth band over the core's stops, written to the settings' `map.depth.band_m`. Every word is
/// the core's.
struct DepthSection: View {
    let module: DepthModule
    @Environment(SpotStore.self) private var store

    var body: some View {
        if let info = module.info, let words = module.words {
            Section {
                if let status = module.status {
                    DepthStatusRow(status: status, text: status.text(words))
                }
                if module.visible.contours {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(words.contoursTitle)
                        Text(words.contoursDetail)
                            .font(.footnote)
                            .foregroundStyle(.readableSecondary)
                    }
                    .accessibilityElement(children: .combine)
                    .accessibilityIdentifier("map-depth-contours")
                }
                // The legend first: on the iPhone it stays in view at the sheet's middle height.
                DepthLegend(
                    info: info, bar: module.visible.shading,
                    datums: module.datums?.text ?? info.depthReference)
                if module.visible.shading {
                    let json = module.settingsJSON
                    let stops = DepthBand.stops(settingsJSON: json)
                    let band = (try? Settings(json: json))?.map.depth.bandM
                    DepthBandControl(
                        stops: stops, band: DepthBand.positions(of: band, in: stops), words: words
                    ) { positions in
                        store.editMap { DepthBand.set(positions, stops: stops, in: &$0) }
                    }
                }
            } header: {
                Text(info.title)
            }
        }
    }
}

/// What the depth layers say in place of depth, with a sign of which it is.
private struct DepthStatusRow: View {
    let status: DepthStatus
    let text: String

    var body: some View {
        Label {
            Text(text)
                .font(.subheadline)
                .fixedSize(horizontal: false, vertical: true)
        } icon: {
            switch status {
            case .zoomIn:
                Image(systemName: "plus.magnifyingglass")
                    .foregroundStyle(.readableSecondary)
            case .loading:
                ProgressView()
                    .controlSize(.small)
            case .failed:
                Image(systemName: "exclamationmark.circle")
                    .foregroundStyle(Palette.warning)
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("map-depth-status")
    }
}
