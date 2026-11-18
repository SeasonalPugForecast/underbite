import Foundation

/// A raster tile's place in the offline store: the layer, the day asked for and the tile fetched
/// (for an over-zoomed map tile, its ancestor at the layer's deepest zoom), as the core's
/// `tile_request` and pack plans name it.
nonisolated struct OfflineTileKey: Hashable, Sendable {
    /// The catalogue layer's id.
    let layer: String
    /// `YYYY-MM-DD` (a UTC day) or another token the layer uses (a radar frame's time), or none
    /// for the source's newest.
    let date: String?
    let z: UInt8
    let x: UInt32
    let y: UInt32

    /// A key, or none when the layer or date could not be written as one (empty, or holding a
    /// `/`, or a date of `-` or `latest`, which stand for none).
    init?(layer: String, date: String?, z: UInt8, x: UInt32, y: UInt32) {
        guard Self.isPart(layer) else { return nil }
        if let date, !Self.isPart(date) || Self.newest.contains(date) { return nil }
        self.layer = layer
        self.date = date
        self.z = z
        self.x = x
        self.y = y
    }

    /// The tokens that stand for "no date" in a key: `-` as the store writes it, `latest` as a
    /// pack plan does.
    static let newest: Set<String> = ["-", "latest"]

    /// What the store's `date` column holds: the date, or `-` for none.
    var dateColumn: String { date ?? "-" }

    /// `sst-imagery/2026-11-16/7/37/48`; `-` for no date. Pins name tiles by this text.
    var text: String { "\(layer)/\(dateColumn)/\(z)/\(x)/\(y)" }

    /// The key written as ``text``, or as a pack plan's tile key (`latest` for no date).
    init?(text: String) {
        let parts = text.split(separator: "/", omittingEmptySubsequences: false).map(String.init)
        guard parts.count == 5, let z = UInt8(parts[2]), let x = UInt32(parts[3]),
            let y = UInt32(parts[4]),
            // Only the canonical decimal form, so that one tile has one text.
            String(z) == parts[2], String(x) == parts[3], String(y) == parts[4]
        else { return nil }
        let date = Self.newest.contains(parts[1]) ? nil : parts[1]
        self.init(layer: parts[0], date: date, z: z, x: x, y: y)
    }

    private static func isPart(_ s: String) -> Bool {
        !s.isEmpty && !s.contains("/") && !s.contains(where: \.isNewline)
    }
}

/// What a row of the store is: a raster tile, or a blob (a grid block, an elevation area, a
/// station metadata response).
nonisolated enum OfflineKind: String, Hashable, Sendable, CaseIterable {
    case tile
    case blob
}

/// What a pack download is, as the core's pack plan says (`PackItemKind`).
nonisolated enum OfflinePackItemKind: Hashable, Sendable {
    case tile
    case grid
    case dem
}

/// A row of the store: a tile by its place, or a blob by the core's key (a grid block's
/// `BlockId` key, an elevation area's `dem/area/...` key, a metadata response's cache key).
nonisolated enum OfflineKey: Hashable, Sendable {
    case tile(OfflineTileKey)
    case blob(String)

    /// The row a pack download of `kind` under `key` is stored as: a tile's key parsed, a grid
    /// block or elevation area as a blob under the key itself.
    init?(pack kind: OfflinePackItemKind, key: String) {
        switch kind {
        case .tile:
            guard let tile = OfflineTileKey(text: key) else { return nil }
            self = .tile(tile)
        case .grid, .dem:
            guard !key.isEmpty else { return nil }
            self = .blob(key)
        }
    }

    var kind: OfflineKind {
        switch self {
        case .tile: .tile
        case .blob: .blob
        }
    }

    /// How a pin names the row: a tile's ``OfflineTileKey/text``, a blob's key.
    var text: String {
        switch self {
        case .tile(let tile): tile.text
        case .blob(let key): key
        }
    }

    /// The key a pin of `kind` names by `text`.
    init?(kind: OfflineKind, text: String) {
        switch kind {
        case .tile:
            guard let tile = OfflineTileKey(text: text) else { return nil }
            self = .tile(tile)
        case .blob:
            guard !text.isEmpty else { return nil }
            self = .blob(text)
        }
    }
}
