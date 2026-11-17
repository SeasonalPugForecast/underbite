import Foundation
import MapKit
import UnderbiteKit

/// A gridded layer (MUR SST, chlorophyll, clarity, Great Lakes SST) drawn from the blocks the
/// ``GridLoader`` put in the core's `GridStore`: each map tile is the core's `render_tile` in the
/// layer's ramp and range (the settings' own, else the one fitted to the view). Where the store
/// has nothing yet the tile is empty. Rendering runs off the main thread (the store is
/// thread-safe and renders outside its lock).
///
/// An overlay draws with what the store holds when MapKit asks for each tile, and MapKit asks
/// for a tile once: when blocks arrive or the range changes, the map puts a new overlay in its
/// place (a reload of the renderer would drop the tiles drawn and not ask for them again), and
/// takes the old one off once ``firstTiles`` says the new one's tiles are answered.
nonisolated final class GridTileOverlay: MKTileOverlay {
    let layer: String
    /// Says when the tiles first asked for have all been answered.
    let firstTiles = FirstTiles()
    private let ramp: String?
    private let range: ValueRange?
    private let store: GridStore

    /// - Parameter range: the range to draw with, canonical units; none: the settings' own, else
    ///   the layer's.
    init(_ planned: PlannedLayer, store: GridStore, range: ValueRange? = nil) {
        layer = planned.id
        ramp = planned.ramp
        let own = planned.range.flatMap {
            $0.count == 2 ? ValueRange(low: $0[0], high: $0[1]) : nil
        }
        self.range = range ?? own
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
        let firstTiles = firstTiles
        firstTiles.ask()
        guard let z = UInt8(exactly: path.z), let x = UInt32(exactly: path.x),
            let y = UInt32(exactly: path.y)
        else {
            reply.send(nil)
            firstTiles.answer()
            return
        }
        let (layer, ramp, low, high, store) = (layer, ramp, range?.low, range?.high, store)
        DispatchQueue.global(qos: .userInitiated).async {
            let png = try? store.renderTile(
                layer: layer, ramp: ramp, low: low, high: high, z: z, x: x, y: y)
            reply.send(png ?? nil)
            firstTiles.answer()
        }
    }
}
