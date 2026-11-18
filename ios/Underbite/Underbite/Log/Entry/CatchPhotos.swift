import CoreGraphics
import Foundation
import ImageIO
import Observation
import UniformTypeIdentifiers

/// A picked photo made fit to keep with a catch: scaled down while it is decoded (a panorama or
/// a large HEIC is never decoded whole), turned upright, and written as a new JPEG that carries
/// none of the original's metadata (no position, time or camera). The image is added to the new
/// file as pixels, never copied from the picked file, which would carry its properties over.
nonisolated enum CatchPhotos {
    /// The longest side kept, in pixels.
    static let maxPixels = 2_048

    enum Failure: Error, Equatable {
        /// The data is not an image ImageIO reads.
        case unreadable
        /// The JPEG could not be written.
        case unwritable
    }

    /// `data` (JPEG, HEIC, PNG, ...) as a JPEG of at most `maxPixels` on its long side, upright,
    /// without metadata.
    static func stripped(_ data: Data, maxPixels: Int = maxPixels) throws -> Data {
        // Not cached: the source is read only to make the scaled image.
        let sourceOptions = [kCGImageSourceShouldCache: false] as CFDictionary
        guard let source = CGImageSourceCreateWithData(data as CFData, sourceOptions),
            CGImageSourceGetCount(source) > 0
        else { throw Failure.unreadable }
        let thumbnailOptions =
            [
                kCGImageSourceCreateThumbnailFromImageAlways: true,
                kCGImageSourceCreateThumbnailWithTransform: true,
                kCGImageSourceShouldCacheImmediately: true,
                kCGImageSourceThumbnailMaxPixelSize: maxPixels,
            ] as [CFString: Any] as CFDictionary
        guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, thumbnailOptions) else {
            throw Failure.unreadable
        }
        let out = NSMutableData()
        guard
            let destination = CGImageDestinationCreateWithData(
                out as CFMutableData, UTType.jpeg.identifier as CFString, 1, nil)
        else { throw Failure.unwritable }
        // Only the compression: no EXIF, GPS, TIFF or orientation property is written.
        let properties =
            [kCGImageDestinationLossyCompressionQuality: 0.85] as [CFString: Any] as CFDictionary
        CGImageDestinationAddImage(destination, image, properties)
        guard CGImageDestinationFinalize(destination) else { throw Failure.unwritable }
        return out as Data
    }

    /// A new file name for a photo of a catch.
    static func newName() -> String {
        UUID().uuidString.lowercased() + ".jpg"
    }
}

/// The photos a catch editor adds, each kept by the store (`addPhoto`) as it comes, one after the
/// other. Closing the editor stops them: its Cancel (`cancel`), or the editor going away, cancels
/// the work under way and removes the photos added since it opened, and a photo that finishes
/// after that is removed too. Saving keeps them (`keep`).
@MainActor
@Observable
final class CatchPhotoAdds {
    /// Reads one picked photo; `nil` when it has no data.
    typealias Load = @MainActor () async throws -> Data?

    /// Photos under way.
    private(set) var adding = 0
    /// The photo files written since the editor opened, in the order they came.
    private(set) var added: [String] = []
    /// A photo could not be added since the last pick.
    private(set) var failed = false

    @ObservationIgnored private let catches: CatchStore
    @ObservationIgnored private var tasks: [Task<Void, Never>] = []

    init(catches: CatchStore) {
        self.catches = catches
    }

    isolated deinit {
        cancel()
    }

    /// Adds the photos `loads` read, after any still under way.
    func add(_ loads: [Load]) {
        guard !loads.isEmpty else { return }
        failed = false
        adding += loads.count
        let catches = self.catches
        let before = tasks.last
        tasks.append(
            Task { [weak self] in
                await before?.value
                for load in loads {
                    let name = await Self.written(load, in: catches)
                    guard let self, !Task.isCancelled else {
                        // The editor closed meanwhile: no catch keeps the photo.
                        if let name { catches.removePhotos([name]) }
                        return
                    }
                    adding -= 1
                    if let name { added.append(name) } else { failed = true }
                }
            })
    }

    /// The photo `load` reads, kept by `catches`; `nil` when it cannot be read or kept.
    private static func written(_ load: Load, in catches: CatchStore) async -> String? {
        guard !Task.isCancelled, let data = try? await load() else { return nil }
        return try? await catches.addPhoto(data)
    }

    /// Stops the photos under way and removes those added: the editor was cancelled.
    func cancel() {
        for task in tasks { task.cancel() }
        adding = 0
        catches.removePhotos(added)
        added = []
    }

    /// Keeps the photos added: the catch was saved (the editor removes those it dropped).
    func keep() {
        added = []
    }

    /// Waits until the work under way is done, cancelled or not.
    func settle() async {
        for task in tasks { await task.value }
    }
}
