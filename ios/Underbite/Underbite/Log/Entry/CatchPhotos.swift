import CoreGraphics
import Foundation
import ImageIO
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
