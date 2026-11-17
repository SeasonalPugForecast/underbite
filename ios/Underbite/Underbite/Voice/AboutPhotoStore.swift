import Foundation
import UIKit

/// The optional photo on the About screen. It is personal: it lives in the app container only,
/// is re-encoded without the picture's metadata, is kept out of device backups, and is never
/// shared, uploaded or logged.
enum AboutPhotoStore {
    /// Where the photo is kept.
    static var url: URL {
        URL.applicationSupportDirectory
            .appending(path: "Underbite", directoryHint: .isDirectory)
            .appending(path: "about-photo.jpg", directoryHint: .notDirectory)
    }

    /// The saved photo, if any.
    static func load() -> UIImage? {
        guard let data = try? Data(contentsOf: url) else { return nil }
        // A photo saved before it was kept out of backups is brought in line.
        try? excludeFromBackup()
        return UIImage(data: data)
    }

    /// Saves the picked image data, scaled to at most 1200 points on its long side.
    static func save(_ data: Data) throws {
        guard let image = UIImage(data: data) else { throw CocoaError(.fileReadCorruptFile) }
        let longSide = max(image.size.width, image.size.height)
        let scale = min(1, 1200 / max(longSide, 1))
        let size = CGSize(width: image.size.width * scale, height: image.size.height * scale)
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        let scaled = UIGraphicsImageRenderer(size: size, format: format).image { _ in
            image.draw(in: CGRect(origin: .zero, size: size))
        }
        guard let jpeg = scaled.jpegData(compressionQuality: 0.85) else {
            throw CocoaError(.fileWriteUnknown)
        }
        try FileManager.default.createDirectory(
            at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try jpeg.write(to: url, options: [.atomic, .completeFileProtection])
        try excludeFromBackup()
    }

    /// Keeps the photo out of iCloud and computer backups: it is personal and stays on the
    /// device. Writing replaces the file, so the flag is set again after each save.
    static func excludeFromBackup() throws {
        var file = url
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try file.setResourceValues(values)
    }

    /// Removes the saved photo.
    static func remove() {
        try? FileManager.default.removeItem(at: url)
    }
}
