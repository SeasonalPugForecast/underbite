import Foundation
import MapKit
import UnderbiteKit

/// A gridded layer (MUR SST, chlorophyll, clarity, Great Lakes SST) drawn from the blocks the
/// ``GridLoader`` put in the core's `GridStore`: each map tile is the core's `render_tile` in the
/// layer's ramp and range. Where the store has nothing yet the tile is empty; the map reloads the
/// renderer as blocks arrive. Rendering runs off the main thread (the store is thread-safe and
/// renders outside its lock).
nonisolated final class GridTileOverlay: MKTileOverlay {
    let layer: String
    private let ramp: String?
    private let low: Double?
    private let high: Double?
    private let store: GridStore

    init(_ planned: PlannedLayer, store: GridStore) {
        layer = planned.id
        ramp = planned.ramp
        low = planned.range?.first
        high = planned.range?.last
        self.store = store
        super.init(urlTemplate: nil)
        tileSize = CGSize(width: Int(mapTileSize()), height: Int(mapTileSize()))
        minimumZ = Int(planned.info.minZoom ?? 0)
        // A grid renders at any zoom: MapKit asks for nothing past `maximumZ`.
        maximumZ = MapPlan.deepestZoom
        canReplaceMapContent = false
    }

    override func loadTile(
        at path: MKTileOverlayPath, result: @escaping (Data?, (any Error)?) -> Void
    ) {
        let reply = TileReply(result: result)
        guard let z = UInt8(exactly: path.z), let x = UInt32(exactly: path.x),
            let y = UInt32(exactly: path.y)
        else {
            reply.send(nil)
            return
        }
        let (layer, ramp, low, high, store) = (layer, ramp, low, high, store)
        DispatchQueue.global(qos: .userInitiated).async {
            let png = try? store.renderTile(
                layer: layer, ramp: ramp, low: low, high: high, z: z, x: x, y: y)
            reply.send(png ?? nil)
        }
    }
}
