import Foundation
import XCTest

@testable import UnderbiteKit

/// The preference edits against the core: what they write is valid settings, typed values
/// and stops go through the core, and a units change leaves stored values alone.
final class PreferencesTests: XCTestCase {
    private func defaults() throws -> String { defaultSettingsJson() }

    private func settings(_ json: String) throws -> Settings { try Settings(json: json) }

    private func assertValid(_ json: String, file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertEqual(
            validateSettings(json: json).filter { $0.severity == .error }, [], file: file, line: line)
    }

    // MARK: Units

    func testEveryUnitChoiceOfEveryGroupIsValid() throws {
        let base = try defaults()
        for group in try unitOptions(settingsJson: base) {
            for option in group.options {
                let edited = try Preferences.applying(base) {
                    Preferences.setUnit(option.id, for: group.id, in: &$0.units)
                }
                assertValid(edited)
                let reread = try unitOptions(settingsJson: edited).first { $0.id == group.id }
                XCTAssertEqual(reread?.selected, option.id, group.id)
            }
        }
    }

    func testAGroupThatFollowsAnotherReturnsToFollowingIt() throws {
        let base = try defaults()
        let own = try Preferences.applying(base) {
            Preferences.setUnit("m", for: "tide", in: &$0.units)
        }
        XCTAssertEqual(try settings(own).units.tide, "m")
        let tide = try unitOptions(settingsJson: own).first { $0.id == "tide" }
        XCTAssertEqual(tide?.overridden, true)
        let back = try Preferences.applying(own) {
            Preferences.setUnit(nil, for: "tide", in: &$0.units)
        }
        XCTAssertNil(try settings(back).units.tide)
        // A base group has nothing to follow: nil leaves it.
        let kept = try Preferences.applying(base) {
            Preferences.setUnit(nil, for: "height", in: &$0.units)
        }
        XCTAssertEqual(try settings(kept).units.height, try settings(base).units.height)
    }

    func testChangingUnitsDoesNotRewriteStoredRangesOrLimits() throws {
        let base = try Preferences.applying(try defaults()) {
            var river = Preferences.newRiver(site: "usgs:01420500")
            river.wadeableBelowCfs = 1200
            river.bestCfs = [400, 900]
            river.blownOutAboveCfs = 3000
            $0.rivers = [river]
            $0.thresholds.troutStressC = 21
        }
        let metric = try Preferences.applying(base) {
            Preferences.setUnit("m3s", for: "discharge", in: &$0.units)
            Preferences.setUnit("C", for: "water_temp", in: &$0.units)
            Preferences.setUnit("kmh", for: "speed", in: &$0.units)
            Preferences.setUnit("m", for: "height", in: &$0.units)
        }
        let before = try settings(base)
        let after = try settings(metric)
        XCTAssertEqual(after.rivers, before.rivers)
        XCTAssertEqual(after.limits, before.limits)
        XCTAssertEqual(after.thresholds, before.thresholds)
    }

    // MARK: Rejection

    func testAnEditThatLeavesInvalidSettingsIsRefused() throws {
        let base = try defaults()
        XCTAssertThrowsError(
            try Preferences.applying(base) {
                var river = Preferences.newRiver(site: "usgs:01420500")
                river.wadeableBelowCfs = 900
                river.blownOutAboveCfs = 500
                $0.rivers = [river]
            }
        ) { error in
            XCTAssertTrue(error is Preferences.Rejected)
            XCTAssertTrue("\(error)".contains("rivers[0]"), "\(error)")
        }
        XCTAssertThrowsError(
            try Preferences.applying(base) { $0.windows.solunarMajorMinutes = 0 })
    }

    // MARK: Limits

    func testNewProfilesGetUniqueNamesAndAreValid() throws {
        let base = try defaults()
        let edited = try Preferences.applying(base) { s in
            s.limits.append(Preferences.newLimit(existing: s.limits))
            s.limits.append(Preferences.newLimit(existing: s.limits))
        }
        let names = try settings(edited).limits.map(\.name)
        XCTAssertEqual(Set(names).count, names.count)
        XCTAssertTrue(names.contains("New profile"))
        XCTAssertTrue(names.contains("New profile 2"))
        assertValid(edited)
    }

    func testDeletingAndEditingProfiles() throws {
        let base = try defaults()
        let edited = try Preferences.applying(base) { s in
            Preferences.LimitField.wind.set(15, on: &s.limits[0])
            Preferences.LimitField.visibility.set(nil, on: &s.limits[0])
            s.limits.removeLast()
        }
        let after = try settings(edited)
        XCTAssertEqual(after.limits.count, try settings(base).limits.count - 1)
        XCTAssertEqual(Preferences.LimitField.wind.value(of: after.limits[0]), 15)
        assertValid(edited)
    }

    // MARK: Rivers

    func testRiverSitesKeyByTheGaugeId() {
        XCTAssertEqual(Preferences.riverSite(from: "01420500"), "usgs:01420500")
        XCTAssertEqual(Preferences.riverSite(from: " USGS:01420500 "), "usgs:01420500")
        XCTAssertNil(Preferences.riverSite(from: "abc"))
        XCTAssertNil(Preferences.riverSite(from: "usgs:"))
        XCTAssertNil(Preferences.riverSite(from: "123"))
    }

    // MARK: Privacy and freshness

    func testPrecisionNeverOffersFinerThanTwoDecimals() {
        XCTAssertEqual(Preferences.precisionChoices, [2, 1, 0])
        XCTAssertFalse(Preferences.allowsPrecision(3))
        XCTAssertFalse(Preferences.allowsPrecision(-1))
    }

    func testPrecisionVoiceAndSolunarEditsAreValid() throws {
        let edited = try Preferences.applying(try defaults()) {
            $0.sources.coordinatePrecision = 1
            $0.presentation.voice = "plain"
            $0.windows.showSolunar = false
            $0.sources.flow.maxAgeMinutes = 60
            $0.sources.flow.maxDistanceKm = 40
        }
        let s = try settings(edited)
        XCTAssertEqual(s.sources.coordinatePrecision, 1)
        XCTAssertEqual(s.presentation.voice, "plain")
        XCTAssertFalse(s.windows.showSolunar)
        for kind in Preferences.SourceKind.allCases {
            var sources = s.sources
            var group = kind.group(in: sources)
            group.maxAgeMinutes = 30
            kind.set(group, in: &sources)
            XCTAssertEqual(kind.group(in: sources).maxAgeMinutes, 30)
        }
    }

    // MARK: Values through the core

    func testTypedValuesGoThroughTheCoreInTheUsersUnit() throws {
        let knots = try Preferences.applying(try defaults()) {
            Preferences.setUnit("kn", for: "speed", in: &$0.units)
        }
        let kmh = try Preferences.applying(knots) {
            Preferences.setUnit("kmh", for: "speed", in: &$0.units)
        }
        let wind = Preferences.LimitField.wind.measurement
        // Typed in knots, the field (knots) holds what was typed.
        let inKnots = try Preferences.Entry(wind, settingsJSON: knots)
        XCTAssertEqual(try inKnots.field(fromTyped: "12"), 12, accuracy: 1e-3)
        // Typed in km/h, the same wind is a different number of knots.
        let inKmh = try Preferences.Entry(wind, settingsJSON: kmh)
        XCTAssertEqual(try inKmh.field(fromTyped: "22.224"), 12, accuracy: 1e-3)
        // An explicit unit overrides the setting.
        XCTAssertEqual(try inKmh.field(fromTyped: "12 kn"), 12, accuracy: 1e-3)
        // A decimal comma and a thousands comma are left to the core.
        let flow = try Preferences.Entry(Preferences.flow, settingsJSON: try defaults())
        XCTAssertEqual(try flow.field(fromTyped: "1,200 cfs"), 1200, accuracy: 1e-3)
        XCTAssertThrowsError(try flow.field(fromTyped: "plenty"))
    }

    func testTemperatureStaysInCelsiusWhateverTheUnit() throws {
        let fahrenheit = try Preferences.applying(try defaults()) {
            Preferences.setUnit("F", for: "water_temp", in: &$0.units)
        }
        let entry = try Preferences.Entry(Preferences.troutStress, settingsJSON: fahrenheit)
        XCTAssertEqual(try entry.field(fromTyped: "68"), 20, accuracy: 1e-3)
        XCTAssertEqual(try entry.field(fromTyped: "20 °C"), 20, accuracy: 1e-3)
        let stops = try entry.stops()
        XCTAssertTrue(stops.contains { $0.label.contains("°F") })
        // 68 °F (the default 20 °C) falls between the round stops, so its label says "about".
        XCTAssertFalse(Preferences.isOnStop(stops, Preferences.defaultTroutStressC))
        let index = try XCTUnwrap(Preferences.nearest(stops, to: Preferences.defaultTroutStressC))
        XCTAssertEqual(stops[index].value, 20, accuracy: 6)
    }

    func testStopsAreLabelledByTheCoreAndHoldTheStoredValue() throws {
        let base = try defaults()
        let entry = try Preferences.Entry(Preferences.flow, settingsJSON: base)
        let stops = try entry.stops()
        XCTAssertGreaterThan(stops.count, 2)
        XCTAssertTrue(stops.allSatisfy { $0.label.contains("cfs") })
        XCTAssertEqual(stops.map(\.value), stops.map(\.value).sorted())
        // A stored value outside the usual span still gets stops that reach it.
        let wide = try Preferences.stopsHolding(30000, entry: entry)
        XCTAssertGreaterThanOrEqual(wide.last?.value ?? 0, 29000)
        XCTAssertTrue(Preferences.isOnStop(stops, stops[1].value))
        XCTAssertFalse(Preferences.isOnStop(stops, (stops[1].value + stops[2].value) / 2))
    }

    func testTheCoresMessageForAnUnreadableValueIsKept() throws {
        let entry = try Preferences.Entry(Preferences.flow, settingsJSON: try defaults())
        do {
            _ = try entry.field(fromTyped: "plenty")
            XCTFail("expected an error")
        } catch {
            XCTAssertTrue(Preferences.message(for: error).contains("plenty"), "\(error)")
        }
    }

    func testEveryLimitFieldHasWorkingStopsAndEntry() throws {
        let base = try defaults()
        for field in Preferences.LimitField.allCases {
            let entry = try Preferences.Entry(field.measurement, settingsJSON: base)
            XCTAssertFalse(try entry.stops().isEmpty, field.rawValue)
            let typed = try entry.field(
                fromTyped: "5 \(field.measurement.unit)")
            XCTAssertEqual(typed, 5, accuracy: 1e-3, field.rawValue)
        }
    }

    func testEditedLimitsFromStopsAreValid() throws {
        let base = try defaults()
        let edited = try Preferences.applying(base) { s in
            for field in Preferences.LimitField.allCases {
                let entry = try? Preferences.Entry(field.measurement, settingsJSON: base)
                if let stop = try? entry?.stops().last {
                    field.set(stop.value, on: &s.limits[0])
                }
            }
        }
        assertValid(edited)
    }
}
