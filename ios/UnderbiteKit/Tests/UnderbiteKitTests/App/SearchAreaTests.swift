import XCTest

/// What a place search sends of where the phone is.
final class SearchAreaTests: XCTestCase {
    func testAPlaceSearchIsCentredOnAnApproximateArea() {
        let battery = SearchArea.centre(latitude: 40.7033, longitude: -74.0170)
        XCTAssertEqual(battery.latitude, 40.7, accuracy: 1e-9)
        XCTAssertEqual(battery.longitude, -74.0, accuracy: 1e-9)
        let croton = SearchArea.centre(latitude: 41.2250277777778, longitude: -73.8588888888889)
        XCTAssertEqual(croton.latitude, 41.2, accuracy: 1e-9)
        XCTAssertEqual(croton.longitude, -73.9, accuracy: 1e-9)
        for (lat, lon) in [(40.7033, -74.0170), (41.2250277777778, -73.8588888888889)] {
            let centre = SearchArea.centre(latitude: lat, longitude: lon)
            XCTAssertEqual((centre.latitude * 10).rounded(), centre.latitude * 10, accuracy: 1e-9)
            XCTAssertEqual((centre.longitude * 10).rounded(), centre.longitude * 10, accuracy: 1e-9)
        }
    }
}
