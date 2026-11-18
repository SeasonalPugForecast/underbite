import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers
import XCTest

@testable import UnderbiteKit

/// Images the tile tests draw: no recorded tile is needed to check cutting and caching.
enum TilePictures {
    /// A `side`-pixel PNG with four coloured quarters: red top-left, green top-right, blue
    /// bottom-left, white bottom-right.
    static func quarters(side: Int = 256) -> Data {
        let space = CGColorSpace(name: CGColorSpace.sRGB)!
        let context = CGContext(
            data: nil, width: side, height: side, bitsPerComponent: 8, bytesPerRow: 0,
            space: space, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
        let half = CGFloat(side) / 2
        // Core Graphics counts y from the bottom.
        let quarters: [(CGRect, [CGFloat])] = [
            (CGRect(x: 0, y: half, width: half, height: half), [1, 0, 0, 1]),
            (CGRect(x: half, y: half, width: half, height: half), [0, 1, 0, 1]),
            (CGRect(x: 0, y: 0, width: half, height: half), [0, 0, 1, 1]),
            (CGRect(x: half, y: 0, width: half, height: half), [1, 1, 1, 1]),
        ]
        for (rect, rgba) in quarters {
            context.setFillColor(CGColor(colorSpace: space, components: rgba)!)
            context.fill(rect)
        }
        let out = NSMutableData()
        let destination = CGImageDestinationCreateWithData(
            out as CFMutableData, UTType.png.identifier as CFString, 1, nil)!
        CGImageDestinationAddImage(destination, context.makeImage()!, nil)
        CGImageDestinationFinalize(destination)
        return out as Data
    }

    /// The width of the image in `data` and the colour of its pixel at `x`, `y` (from the top),
    /// 0 to 255 per channel.
    static func pixel(_ data: Data, x: Int, y: Int) -> (width: Int, rgba: [UInt8])? {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil),
            let image = CGImageSourceCreateImageAtIndex(source, 0, nil)
        else { return nil }
        var pixel = [UInt8](repeating: 0, count: 4)
        let drawn = pixel.withUnsafeMutableBytes { buffer -> Bool in
            guard
                let context = CGContext(
                    data: buffer.baseAddress, width: 1, height: 1, bitsPerComponent: 8,
                    bytesPerRow: 4, space: CGColorSpace(name: CGColorSpace.sRGB)!,
                    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
            else { return false }
            context.draw(
                image,
                in: CGRect(
                    x: -x, y: -(image.height - 1 - y), width: image.width, height: image.height))
            return true
        }
        return drawn ? (image.width, pixel) : nil
    }
}

/// The tile cache's keys, freshness and fallback, the verdicts on responses and the over-zoom cut.
final class TileCacheTests: XCTestCase {
    private let t0 = Date(timeIntervalSince1970: 1_794_000_000)

    func testTheKeyNamesLayerDayAndTile() {
        let key = TileCacheKey(layer: "sst-imagery", date: "2026-11-16", z: 7, x: 37, y: 48)
        XCTAssertEqual(key.text, "sst-imagery|2026-11-16|7/37/48")
        let newest = TileCacheKey(layer: "sst-imagery", date: nil, z: 7, x: 37, y: 48)
        XCTAssertEqual(newest.text, "sst-imagery|-|7/37/48")
        XCTAssertNotEqual(key, newest)
        XCTAssertNotEqual(key, TileCacheKey(layer: "sst-imagery", date: "2026-11-15", z: 7, x: 37, y: 48))
        XCTAssertNotEqual(key, TileCacheKey(layer: "chlorophyll-imagery", date: "2026-11-16", z: 7, x: 37, y: 48))
    }

    func testATileIsFreshForItsLifetimeAndKeptAfterAsAFallback() {
        let cache = TileMemoryCache()
        let key = TileCacheKey(layer: "noaa-chart", date: nil, z: 12, x: 1206, y: 1539)
        let data = Data([1, 2, 3])
        cache.store(data, for: key, lifetime: 60, now: t0)
        XCTAssertEqual(cache.fresh(key, now: t0.addingTimeInterval(59)), data)
        XCTAssertNil(cache.fresh(key, now: t0.addingTimeInterval(60)))
        XCTAssertEqual(cache.any(key), data)
    }

    func testNothingIsKeptWithoutALifetimeOrABody() {
        let cache = TileMemoryCache()
        let key = TileCacheKey(layer: "radar", date: nil, z: 7, x: 36, y: 48)
        cache.store(Data([1]), for: key, lifetime: 0, now: t0)
        XCTAssertNil(cache.any(key))
        cache.store(Data(), for: key, lifetime: 300, now: t0)
        XCTAssertNil(cache.any(key))
    }

    func testTheLeastRecentlyUsedTileGoesFirstWhenTheCacheIsFull() {
        let cache = TileMemoryCache(costLimit: 10)
        func key(_ x: UInt32) -> TileCacheKey {
            TileCacheKey(layer: "radar", date: nil, z: 7, x: x, y: 0)
        }
        cache.store(Data(repeating: 1, count: 4), for: key(1), lifetime: 60, now: t0)
        cache.store(Data(repeating: 2, count: 4), for: key(2), lifetime: 60, now: t0)
        // Reading the first makes the second the oldest.
        XCTAssertNotNil(cache.any(key(1)))
        cache.store(Data(repeating: 3, count: 4), for: key(3), lifetime: 60, now: t0)
        XCTAssertNotNil(cache.any(key(1)))
        XCTAssertNil(cache.any(key(2)))
        XCTAssertNotNil(cache.any(key(3)))
        // A tile bigger than the whole cache is not kept.
        cache.store(Data(repeating: 4, count: 11), for: key(4), lifetime: 60, now: t0)
        XCTAssertNil(cache.any(key(4)))
    }

    func testAMemoryWarningEmptiesTheCache() {
        let center = NotificationCenter()
        let cache = TileMemoryCache(notifications: center)
        let key = TileCacheKey(layer: "radar", date: nil, z: 7, x: 36, y: 48)
        cache.store(Data([1]), for: key, lifetime: 300, now: t0)
        XCTAssertNotNil(cache.any(key))
        center.post(name: TileMemoryCache.memoryWarning, object: nil)
        XCTAssertNil(cache.any(key))
    }

    func testVerdictsOnResponses() {
        XCTAssertEqual(TileDecision(.image), .keep)
        XCTAssertEqual(TileDecision(.noData), .nothing)
        XCTAssertEqual(TileDecision(.tooDeep), .nothing)
        XCTAssertEqual(TileDecision(.retry(afterS: 3)), .retry(after: 3))
        XCTAssertEqual(TileDecision(.retry(afterS: 10)), .retry(after: 10))
        XCTAssertEqual(TileDecision(.retry(afterS: 30)), .fallback)
        XCTAssertEqual(TileDecision(.failed), .fallback)
    }

    func testTheCoreVerdictOnAChartErrorPageIsARetryThenAFallback() throws {
        // The chart service answers some failures with status 200 and an HTML page.
        let first = try classifyTile(
            layer: "noaa-chart", status: 200, contentType: "text/html", bodyLen: 900, attempt: 0)
        let second = try classifyTile(
            layer: "noaa-chart", status: 200, contentType: "text/html", bodyLen: 900, attempt: 1)
        XCTAssertEqual(TileDecision(first), .retry(after: 3))
        XCTAssertEqual(TileDecision(second), .fallback)
    }

    func testTheCutIsTheCropScaledToATile() throws {
        let picture = TilePictures.quarters()
        // The top-right quarter, as a map tile one level deeper shows it.
        let cut = try XCTUnwrap(
            TileImage.crop(picture, to: TileCrop(left: 0.5, top: 0, size: 0.5), side: 256))
        let corner = try XCTUnwrap(TilePictures.pixel(cut, x: 10, y: 10))
        XCTAssertEqual(corner.width, 256)
        XCTAssertEqual(corner.rgba, [0, 255, 0, 255])
        let far = try XCTUnwrap(TilePictures.pixel(cut, x: 250, y: 250))
        XCTAssertEqual(far.rgba, [0, 255, 0, 255])
        // The bottom-left quarter.
        let other = try XCTUnwrap(
            TileImage.crop(picture, to: TileCrop(left: 0, top: 0.5, size: 0.5), side: 256))
        XCTAssertEqual(try XCTUnwrap(TilePictures.pixel(other, x: 128, y: 128)).rgba, [0, 0, 255, 255])
    }

    func testAFarDeeperCutIsOneSourcePixel() throws {
        let picture = TilePictures.quarters()
        let tiny = try XCTUnwrap(
            TileImage.crop(
                picture, to: TileCrop(left: 0.75, top: 0.75, size: 1.0 / 32_768), side: 256))
        XCTAssertEqual(try XCTUnwrap(TilePictures.pixel(tiny, x: 0, y: 255)).rgba, [255, 255, 255, 255])
    }

    func testANonImageIsNoCut() {
        XCTAssertNil(
            TileImage.crop(Data("<html>".utf8), to: TileCrop(left: 0, top: 0, size: 0.5), side: 256))
    }
}
