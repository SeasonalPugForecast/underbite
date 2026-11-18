import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers
import XCTest

@testable import UnderbiteKit

/// Photos of catches: scaled down, turned upright and written without the picture's metadata
/// (position, time, camera), into the log's `photos/` folder.
@MainActor
final class CatchPhotosTests: XCTestCase {
    private let directory = FileManager.default.temporaryDirectory
        .appending(path: "underbite-photos-\(UUID().uuidString)", directoryHint: .isDirectory)

    override func tearDown() {
        try? FileManager.default.removeItem(at: directory)
    }

    /// A plain image of `width` by `height` pixels.
    private static func image(width: Int, height: Int) throws -> CGImage {
        let context = try XCTUnwrap(
            CGContext(
                data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
                space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        context.setFillColor(CGColor(red: 0.1, green: 0.4, blue: 0.6, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: width, height: height))
        context.setFillColor(CGColor(red: 0.9, green: 0.8, blue: 0.2, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: width / 4, height: height / 4))
        return try XCTUnwrap(context.makeImage())
    }

    /// A JPEG as a camera writes it: with a position, a time, the camera and an orientation.
    private static func cameraJPEG(width: Int, height: Int, orientation: Int = 1) throws -> Data {
        let out = NSMutableData()
        let destination = try XCTUnwrap(
            CGImageDestinationCreateWithData(
                out as CFMutableData, UTType.jpeg.identifier as CFString, 1, nil))
        let properties: [CFString: Any] = [
            kCGImagePropertyOrientation: orientation,
            kCGImagePropertyGPSDictionary: [
                kCGImagePropertyGPSLatitude: 40.7033, kCGImagePropertyGPSLatitudeRef: "N",
                kCGImagePropertyGPSLongitude: 74.0170, kCGImagePropertyGPSLongitudeRef: "W",
            ] as [CFString: Any],
            kCGImagePropertyExifDictionary: [
                kCGImagePropertyExifDateTimeOriginal: "2026:11:08 06:10:00"
            ] as [CFString: Any],
            kCGImagePropertyTIFFDictionary: [
                kCGImagePropertyTIFFMake: "Camera maker", kCGImagePropertyTIFFModel: "Model 1",
            ] as [CFString: Any],
        ]
        CGImageDestinationAddImage(
            destination, try image(width: width, height: height), properties as CFDictionary)
        XCTAssertTrue(CGImageDestinationFinalize(destination))
        return out as Data
    }

    private static func properties(_ data: Data) throws -> [CFString: Any] {
        let source = try XCTUnwrap(CGImageSourceCreateWithData(data as CFData, nil))
        return try XCTUnwrap(CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any])
    }

    private static func gpsTag(_ data: Data) throws -> CGImageMetadataTag? {
        let source = try XCTUnwrap(CGImageSourceCreateWithData(data as CFData, nil))
        guard let metadata = CGImageSourceCopyMetadataAtIndex(source, 0, nil) else { return nil }
        return CGImageMetadataCopyTagMatchingImageProperty(
            metadata, kCGImagePropertyGPSDictionary, kCGImagePropertyGPSLatitude)
    }

    func testTheCameraJPEGCarriesItsMetadata() throws {
        // The test's input is what it claims to be, so the next test means something.
        let original = try Self.cameraJPEG(width: 400, height: 200)
        let properties = try Self.properties(original)
        XCTAssertNotNil(properties[kCGImagePropertyGPSDictionary])
        XCTAssertNotNil(try Self.gpsTag(original))
        let tiff = properties[kCGImagePropertyTIFFDictionary] as? [CFString: Any]
        XCTAssertEqual(tiff?[kCGImagePropertyTIFFMake] as? String, "Camera maker")
    }

    func testThePositionTimeAndCameraAreDroppedAndThePhotoTurnedUpright() throws {
        // Orientation 6: stored 400 × 200, seen 200 × 400.
        let stripped = try CatchPhotos.stripped(
            Self.cameraJPEG(width: 400, height: 200, orientation: 6))
        let properties = try Self.properties(stripped)
        XCTAssertNil(properties[kCGImagePropertyGPSDictionary], "\(properties)")
        XCTAssertNil(try Self.gpsTag(stripped))
        let exif = properties[kCGImagePropertyExifDictionary] as? [CFString: Any]
        XCTAssertNil(exif?[kCGImagePropertyExifDateTimeOriginal], "\(properties)")
        let tiff = properties[kCGImagePropertyTIFFDictionary] as? [CFString: Any]
        XCTAssertNil(tiff?[kCGImagePropertyTIFFMake], "\(properties)")
        XCTAssertNil(tiff?[kCGImagePropertyTIFFModel], "\(properties)")
        XCTAssertEqual(properties[kCGImagePropertyPixelWidth] as? Int, 200)
        XCTAssertEqual(properties[kCGImagePropertyPixelHeight] as? Int, 400)
        let orientation = properties[kCGImagePropertyOrientation] as? Int ?? 1
        XCTAssertEqual(orientation, 1, "the pixels are upright, with no turn left to apply")
    }

    func testALargePhotoIsScaledToTheLongestSideKept() throws {
        let stripped = try CatchPhotos.stripped(Self.cameraJPEG(width: 3_000, height: 1_000))
        let properties = try Self.properties(stripped)
        let width = try XCTUnwrap(properties[kCGImagePropertyPixelWidth] as? Int)
        let height = try XCTUnwrap(properties[kCGImagePropertyPixelHeight] as? Int)
        XCTAssertEqual(width, CatchPhotos.maxPixels)
        XCTAssertEqual(Double(height), Double(CatchPhotos.maxPixels) / 3, accuracy: 1)
    }

    func testASmallPhotoKeepsItsSize() throws {
        let stripped = try CatchPhotos.stripped(Self.cameraJPEG(width: 300, height: 200))
        let properties = try Self.properties(stripped)
        XCTAssertEqual(properties[kCGImagePropertyPixelWidth] as? Int, 300)
        XCTAssertEqual(properties[kCGImagePropertyPixelHeight] as? Int, 200)
    }

    func testDataThatIsNoImageIsRefused() {
        XCTAssertThrowsError(try CatchPhotos.stripped(Data("not a photo".utf8))) { error in
            XCTAssertEqual(error as? CatchPhotos.Failure, .unreadable)
        }
        XCTAssertThrowsError(try CatchPhotos.stripped(Data()))
    }

    func testTheStoreKeepsAPhotoInItsFolderAndRemovesIt() async throws {
        let store = CatchStore(files: AppFiles(directory: directory), clock: { 1_800_000_000 })
        let name = try await store.addPhoto(Self.cameraJPEG(width: 400, height: 200))
        XCTAssertTrue(name.hasSuffix(".jpg"))
        XCTAssertFalse(name.contains("/"))
        let url = store.photoURL(name)
        XCTAssertEqual(url.deletingLastPathComponent().lastPathComponent, CatchStore.photosFolder)
        let saved = try Data(contentsOf: url)
        XCTAssertNil(try Self.properties(saved)[kCGImagePropertyGPSDictionary])

        let other = try await store.addPhoto(Self.cameraJPEG(width: 200, height: 200))
        XCTAssertNotEqual(other, name, "every photo has a name of its own")
        store.removePhotos([name, "../catches.json", ""])
        XCTAssertFalse(FileManager.default.fileExists(atPath: url.path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: store.photoURL(other).path))
    }

    func testAPhotoThatIsNoImageIsNotKept() async throws {
        let store = CatchStore(files: AppFiles(directory: directory), clock: { 1_800_000_000 })
        do {
            _ = try await store.addPhoto(Data("not a photo".utf8))
            XCTFail("a file that is no image is refused")
        } catch {
            XCTAssertEqual(error as? CatchPhotos.Failure, .unreadable)
        }
        let folder = directory.appending(path: CatchStore.photosFolder, directoryHint: .isDirectory)
        let kept = (try? FileManager.default.contentsOfDirectory(atPath: folder.path)) ?? []
        XCTAssertEqual(kept, [])
    }
}
