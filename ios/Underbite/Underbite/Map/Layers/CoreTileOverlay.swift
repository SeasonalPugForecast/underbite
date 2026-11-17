import Foundation
import MapKit
import UnderbiteKit

/// A tiled layer of the core's catalogue (the NOAA chart, GIBS imagery, GEBCO relief, radar) as a
/// MapKit tile overlay. Each tile comes from the ``TileLoader``: the core's request, the app's
/// fetch host, the core's verdict on the response, the in-memory cache.
///
/// MapKit asks for nothing deeper than `maximumZ`, so it is ``MapPlan/deepestZoom`` whatever the
/// source has: deeper map tiles are cut from the source's deepest tile here (the loader applies
/// the crop `tile_request` returns). One overlay per layer and day; another day is another
/// overlay.
nonisolated final class CoreTileOverlay: MKTileOverlay {
    let layer: String
    /// `YYYY-MM-DD`, or none for the source's newest.
    let date: String?
    private let loader: TileLoader

    init(layer: LayerInfo, date: String?, loader: TileLoader) {
        self.layer = layer.id
        self.date = date
        self.loader = loader
        super.init(urlTemplate: nil)
        tileSize = CGSize(width: Int(mapTileSize()), height: Int(mapTileSize()))
        minimumZ = Int(layer.minZoom ?? 0)
        maximumZ = MapPlan.deepestZoom
        // The NOAA chart is drawn over Apple's map rather than in place of it: beyond the chart's
        // coverage Apple's map still shows.
        canReplaceMapContent = false
    }

    override func loadTile(
        at path: MKTileOverlayPath, result: @escaping (Data?, (any Error)?) -> Void
    ) {
        let reply = TileReply(result: result)
        let (layer, date, loader) = (layer, date, loader)
        let (z, x, y) = (path.z, path.x, path.y)
        Task.detached(priority: .userInitiated) {
            reply.send(await loader.tile(layer: layer, date: date, z: z, x: x, y: y))
        }
    }
}

/// MapKit's tile callback, carried into the loader's task: MapKit accepts the call from any
/// thread, and every tile gets exactly one answer: the image, or an error where there is none to
/// draw (nothing is drawn there, and MapKit asks again on a reload).
nonisolated struct TileReply: @unchecked Sendable {
    let result: (Data?, (any Error)?) -> Void

    func send(_ data: Data?) {
        result(data, data == nil ? URLError(.zeroByteResource) : nil)
    }
}
