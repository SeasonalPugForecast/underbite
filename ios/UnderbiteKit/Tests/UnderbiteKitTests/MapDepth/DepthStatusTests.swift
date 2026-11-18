import Foundation
import XCTest

@testable import UnderbiteKit

/// What the depth layers show from the catalogue, what they say in place of depth and the words
/// of their controls.
@MainActor
final class DepthStatusTests: XCTestCase {
    /// The harbour off The Battery at zoom 13.
    private let battery = MapRegion(
        west: -74.04, south: 40.69, east: -73.99, north: 40.72, zoom: 13)

    private func json(_ change: (inout Settings) -> Void = { _ in }) throws -> String {
        var settings = try Settings.defaults()
        change(&settings)
        return try settings.json()
    }

    // MARK: - What is shown

    func testTheCataloguesDepthLayersAreFollowedWithTheirOpacityAndZoom() throws {
        let settings = try json { settings in
            for i in settings.map.layers.indices
            where DepthShown.ids.contains(settings.map.layers[i].id) {
                settings.map.layers[i].shown = true
            }
        }
        let catalogue = try mapCatalogue(settingsJson: settings)
        let depth = catalogue.filter { DepthShown.ids.contains($0.id) }
        XCTAssertEqual(Set(depth.map(\.id)), DepthShown.ids, "the catalogue lists both")
        XCTAssertTrue(depth.allSatisfy { $0.group == "depth" && $0.draw == .depth })
        XCTAssertTrue(depth.allSatisfy { $0.notice != nil && !$0.credit.isEmpty })

        let both = DepthShown(catalogue)
        XCTAssertTrue(both.shading)
        XCTAssertTrue(both.contours)
        XCTAssertEqual(both.minZoom, 8)
        let shading = try XCTUnwrap(depth.first { $0.id == DepthShown.shadingID })
        XCTAssertEqual(both.shadingOpacity, Double(shading.opacity))

        let contoursOnly = DepthShown(depth.filter { $0.id == DepthShown.contoursID })
        XCTAssertFalse(contoursOnly.shading)
        XCTAssertTrue(contoursOnly.contours)
        XCTAssertTrue(contoursOnly.any)

        let others = DepthShown(catalogue.filter { !DepthShown.ids.contains($0.id) })
        XCTAssertFalse(others.any)
        XCTAssertNil(others.minZoom)
    }

    // MARK: - In place of depth

    func testShallowerThanTheLayersZoomTheViewAsksToZoomIn() {
        XCTAssertTrue(DepthStatus.deepEnough(zoom: 8, minZoom: 8))
        XCTAssertTrue(DepthStatus.deepEnough(zoom: 7.8, minZoom: 8), "a little short still draws")
        XCTAssertFalse(DepthStatus.deepEnough(zoom: 7.5, minZoom: 8))
        XCTAssertTrue(DepthStatus.deepEnough(zoom: 2, minZoom: nil))

        let area = DepthArea.around(battery)
        XCTAssertEqual(DepthStatus.of(zoom: 6, minZoom: 8, loader: .ready(area)), .zoomIn)
        XCTAssertEqual(DepthStatus.of(zoom: 9, minZoom: 8, loader: .tooWide), .zoomIn)
        XCTAssertEqual(DepthStatus.of(zoom: 13, minZoom: 8, loader: .loading(area)), .loading)
        XCTAssertEqual(DepthStatus.of(zoom: 13, minZoom: 8, loader: .failed(area)), .failed)
        XCTAssertNil(DepthStatus.of(zoom: 13, minZoom: 8, loader: .ready(area)))
        XCTAssertNil(DepthStatus.of(zoom: nil, minZoom: 8, loader: .idle), "before the map settles")
    }

    func testTheWordsAreTheCoresInTheSettingsUnits() throws {
        let plain = try depthWords(settingsJson: try json())
        for status in [DepthStatus.zoomIn, .loading, .failed] {
            XCTAssertFalse(status.text(plain).isEmpty)
        }
        XCTAssertEqual(DepthStatus.zoomIn.text(plain), plain.zoomIn)
        XCTAssertEqual(DepthStatus.failed.text(plain), plain.failed)
        XCTAssertEqual(plain.bandText, plain.bandOff, "no band")
        XCTAssertTrue(plain.contoursDetail.hasSuffix(" ft"), plain.contoursDetail)
        XCTAssertFalse(plain.bandFromLabel.isEmpty)
        XCTAssertFalse(plain.bandClearLabel.isEmpty)

        // A band set at the stops for 15 and 25 ft reads back in feet.
        let stops = DepthBand.stops(settingsJSON: try json())
        let from = try XCTUnwrap(stops.firstIndex { $0.label == "15 ft" })
        let to = try XCTUnwrap(stops.firstIndex { $0.label == "25 ft" })
        let banded = try json { DepthBand.set(from...to, stops: stops, in: &$0.map) }
        XCTAssertEqual(try depthWords(settingsJson: banded).bandText, "15–25 ft")

        let metres = try depthWords(settingsJson: try json { $0.units.height = "m" })
        XCTAssertTrue(metres.contoursDetail.hasSuffix(" m"), metres.contoursDetail)
    }
}
