import Foundation

/// Says once when a tile overlay's first tiles have all been answered: every tile MapKit asked
/// for so far has had its reply, and no more were asked for during a short pause after the last
/// one (MapKit asks for the tiles on screen one after another, and the first may be answered
/// before the last is asked for).
///
/// A grid layer's overlay is replaced rather than reloaded when its blocks or its colour range
/// change (MapKit does not ask a tile overlay again for tiles it already had images for, and
/// drops those images on a reload), and the overlay it replaces stays on the map until then, so
/// the layer does not blink.
nonisolated final class FirstTiles: @unchecked Sendable {
    /// Runs the work a little later, on any thread.
    typealias Later = @Sendable (@escaping @Sendable () -> Void) -> Void

    /// How long no tile may be asked for after the last answer before the tiles count as answered.
    static let pause: TimeInterval = 0.15

    private let lock = NSLock()
    private let later: Later
    private var asked = 0
    private var answered = 0
    private var done = false
    private var action: (@Sendable () -> Void)?

    init(
        later: @escaping Later = { work in
            DispatchQueue.global(qos: .userInitiated).asyncAfter(
                deadline: .now() + FirstTiles.pause, execute: work)
        }
    ) {
        self.later = later
    }

    /// Whether the first tiles have been answered.
    var isDone: Bool { lock.withLock { done } }

    /// Runs `action` once the first tiles have been answered: at once if they already have.
    func onAnswered(_ action: @escaping @Sendable () -> Void) {
        let now = lock.withLock { () -> Bool in
            if done { return true }
            self.action = action
            return false
        }
        if now { action() }
    }

    /// MapKit asked for a tile.
    func ask() {
        lock.withLock { asked += 1 }
    }

    /// A tile asked for was answered, with an image or without one.
    func answer() {
        let mark = lock.withLock { () -> Int? in
            answered += 1
            return !done && answered == asked ? asked : nil
        }
        guard let mark else { return }
        later { [self] in settle(mark) }
    }

    /// The tiles are answered if none was asked for since `mark` and every one has its reply.
    private func settle(_ mark: Int) {
        let run = lock.withLock { () -> (@Sendable () -> Void)? in
            guard !done, asked == mark, answered == asked else { return nil }
            done = true
            defer { action = nil }
            return action
        }
        run?()
    }
}

/// The overlays of the map's layers that were replaced and wait to be taken off the map until
/// the overlay that replaced them has its first tiles, by the layer's overlay key, oldest first.
nonisolated struct RetiredOverlays<Item: AnyObject> {
    private var waiting: [String: [Item]] = [:]

    /// How many overlays wait under `key`.
    func count(for key: String) -> Int { waiting[key]?.count ?? 0 }

    /// `item` was replaced on the map for `key` and stays until ``settled(_:key:current:)``.
    mutating func retire(_ item: Item, key: String) {
        waiting[key, default: []].append(item)
    }

    /// The overlays to take off the map now that the overlay `id` has its first tiles (or has
    /// waited long enough): every one waiting under `key` when `id` is still the overlay drawn
    /// for it (`current`), none when it was itself replaced since (the overlay that replaced it
    /// takes them off in turn).
    mutating func settled(_ id: ObjectIdentifier, key: String, current: Item?) -> [Item] {
        guard let current, ObjectIdentifier(current) == id else { return [] }
        return waiting.removeValue(forKey: key) ?? []
    }

    /// The overlays waiting under `key`, which leaves the map: all of them go with it.
    mutating func drop(key: String) -> [Item] {
        waiting.removeValue(forKey: key) ?? []
    }
}

/// One replacement at a time per grid layer: blocks (an overview, then the first fine blocks, or
/// a screenful from the device's cache) arrive faster than the map draws a new overlay's tiles,
/// and replacing an overlay before its tiles are drawn only draws them again. A revision that
/// comes while a replacement is still drawing is owed, and all those owed are drawn by one
/// replacement once it settles.
nonisolated struct ReplacementGate {
    private var drawing: Set<String> = []
    private var owed: Set<String> = []

    /// Whether a replacement of `key` is drawing its first tiles.
    func isDrawing(_ key: String) -> Bool { drawing.contains(key) }

    /// Whether a replacement of `key` is owed.
    func isOwed(_ key: String) -> Bool { owed.contains(key) }

    /// A new revision of `key` is to be drawn: true to replace the overlay now, false when a
    /// replacement is still drawing (one more is then owed).
    mutating func request(_ key: String) -> Bool {
        guard !drawing.contains(key) else {
            owed.insert(key)
            return false
        }
        drawing.insert(key)
        return true
    }

    /// The replacement of `key` drew its first tiles (or waited long enough): true when one more
    /// is owed, to be started now (it counts as drawing).
    mutating func settled(_ key: String) -> Bool {
        if owed.remove(key) != nil { return true }
        drawing.remove(key)
        return false
    }

    /// The layer `key` left the map.
    mutating func drop(_ key: String) {
        drawing.remove(key)
        owed.remove(key)
    }
}
