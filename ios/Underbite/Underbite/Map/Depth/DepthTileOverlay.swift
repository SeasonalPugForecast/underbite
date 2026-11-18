import Foundation
import MapKit
@preconcurrency import UnderbiteKit

/// The depth shading drawn from the elevation the ``DepthLoader`` put in the core's `GridStore`:
/// each map tile is the core's `render_depth_tile` with the settings it is made with (the depth
/// ramp and the depth band); water only, land and shore left clear. Where the store has nothing
/// the tile is empty. Rendering runs off the main thread (the store renders outside its lock).
///
/// MapKit asks a tile overlay for each tile once, so new elevation, a new band or a new ramp
/// come as a new overlay over this one, which comes off once ``firstTiles`` says the new one's
/// tiles are answered (as the map's other grid layers do).
nonisolated final class DepthTileOverlay: MKTileOverlay {
    /// Says when the tiles first asked for have all been answered.
    let firstTiles = FirstTiles()
    private let store: GridStore
    private let settingsJSON: String

    init(store: GridStore, settingsJSON: String) {
        self.store = store
        self.settingsJSON = settingsJSON
        super.init(urlTemplate: nil)
        tileSize = CGSize(width: Int(mapTileSize()), height: Int(mapTileSize()))
        // The shading renders at any zoom: MapKit asks for nothing past `maximumZ`.
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
        let (store, json) = (store, settingsJSON)
        DispatchQueue.global(qos: .userInitiated).async {
            let png = try? store.renderDepthTile(z: z, x: x, y: y, settingsJson: json)
            reply.send(png ?? nil)
            firstTiles.answer()
        }
    }
}
