import Foundation
import XCTest

@testable import UnderbiteKit

/// The roles' raw values (which saved themes use) and the lookups from the core's names.
final class ColorRoleTests: XCTestCase {
    /// Every raw value shipped so far. Saved themes name roles by these, so one may be added
    /// to the enum but never renamed or removed.
    private let shipped = [
        "accent", "status.warning", "status.error", "status.success",
        "card.fresh", "card.stale", "card.far", "card.unavailable",
        "verdict.go", "verdict.marginal", "verdict.no_go", "verdict.unknown",
        "alert.severe", "alert.moderate",
        "fate.released", "fate.kept", "fate.kept_for_sashimi",
        "flow.record_low", "flow.much_below_normal", "flow.below_normal", "flow.normal",
        "flow.above_normal", "flow.much_above_normal", "flow.record_high",
        "flood.action", "flood.minor", "flood.moderate", "flood.major",
        "station.none", "station.unknown", "station.no_data", "station.reporting", "station.low",
        "station.normal", "station.elevated", "station.flood_minor", "station.flood_moderate",
        "station.flood_major",
        "tide.curve", "tide.fill", "tide.high", "tide.low", "tide.slack",
        "timeline.night", "timeline.sun", "timeline.solunar_major", "timeline.solunar_minor",
        "timeline.window",
        "chart.predicted", "chart.observed", "chart.forecast", "chart.modelled",
        "chart.flood_category", "chart.user_range", "chart.threshold", "chart.value_band", "chart.now",
        "spot.favourite", "spot.pinned",
    ]

    func testShippedRawValuesStillExist() {
        for raw in shipped {
            XCTAssertNotNil(ColorRole(rawValue: raw), "\(raw) was renamed or removed")
        }
        XCTAssertEqual(Set(ColorRole.allCases.map(\.rawValue)).count, ColorRole.allCases.count)
    }

    func testEveryRoleHasAGroupATitleAndAKind() {
        for group in ColorRoleGroup.allCases {
            let roles = ColorRole.roles(in: group)
            XCTAssertFalse(roles.isEmpty, "\(group) has no roles")
            XCTAssertFalse(group.title.isEmpty)
            XCTAssertEqual(Set(roles.map(\.title)).count, roles.count, "titles repeat in \(group)")
        }
        XCTAssertEqual(ColorRoleGroup.allCases.flatMap { ColorRole.roles(in: $0) }.count, ColorRole.allCases.count)
        XCTAssertEqual(ColorRole.roles(in: .stations).map(\.kind), Array(repeating: .pin, count: 10))
        XCTAssertEqual(ColorRole.tideFill.kind, .fill)
        XCTAssertEqual(ColorRole.tideCurve.kind, .mark)
        XCTAssertEqual(ColorRole.timelineSun.kind, .mark)
        XCTAssertEqual(ColorRole.timelineWindow.kind, .fill)
        XCTAssertEqual(ColorRole.chartValueBand.kind, .fill)
        XCTAssertEqual(ColorRole.verdictNoGo.kind, .text)
    }

    func testStationRolesAreTheCoresStateRoles() {
        // `station_pin_roles()` in the FFI.
        let core = [
            "none", "unknown", "no_data", "reporting", "low", "normal", "elevated", "flood_minor",
            "flood_moderate", "flood_major",
        ]
        XCTAssertEqual(core.map(ColorRole.station), ColorRole.roles(in: .stations))
        XCTAssertEqual(ColorRole.station("tsunami"), .stationUnknown)
        XCTAssertEqual(ColorRole.station(""), .stationUnknown)
    }

    func testLookupsTakeTheViewModelsNames() {
        XCTAssertEqual(ColorRole.verdict("go"), .verdictGo)
        XCTAssertEqual(ColorRole.verdict("marginal"), .verdictMarginal)
        XCTAssertEqual(ColorRole.verdict("no_go"), .verdictNoGo)
        XCTAssertEqual(ColorRole.verdict("unknown"), .verdictUnknown)
        XCTAssertEqual(ColorRole.verdict("later"), .verdictUnknown)

        XCTAssertEqual(ColorRole.cardState("stale"), .cardStale)
        XCTAssertNil(ColorRole.cardState("other"))

        XCTAssertEqual(ColorRole.alert(severity: "extreme"), .alertSevere)
        XCTAssertEqual(ColorRole.alert(severity: "severe"), .alertSevere)
        XCTAssertEqual(ColorRole.alert(severity: "minor"), .alertModerate)

        XCTAssertEqual(ColorRole.fate("kept_for_sashimi"), .fateKeptForSashimi)
        XCTAssertEqual(ColorRole.flow("much_below_normal"), .flowMuchBelowNormal)
        XCTAssertEqual(ColorRole.flow("record_high"), .flowRecordHigh)
        XCTAssertNil(ColorRole.flow("high"))
        XCTAssertEqual(ColorRole.flood("Moderate"), .floodModerate)
        XCTAssertNil(ColorRole.flood("record"))

        XCTAssertEqual(ColorRole.series("predicted"), .chartPredicted)
        XCTAssertEqual(ColorRole.series("observed"), .chartObserved)
        XCTAssertEqual(ColorRole.series("forecast"), .chartForecast)
        XCTAssertEqual(ColorRole.series("modelled"), .chartModelled)
        XCTAssertEqual(ColorRole.rule("flood_category"), .chartFloodCategory)
        XCTAssertEqual(ColorRole.rule("user_range"), .chartUserRange)
        XCTAssertEqual(ColorRole.rule("threshold"), .chartThreshold)

        XCTAssertEqual(ColorRole.band("night"), .timelineNight)
        XCTAssertEqual(ColorRole.band("slack"), .tideSlack)
        XCTAssertEqual(ColorRole.band("solunar_major"), .timelineSolunarMajor)
        XCTAssertEqual(ColorRole.band("solunar_minor"), .timelineSolunarMinor)
        XCTAssertEqual(ColorRole.band("highlight"), .timelineWindow)
    }
}
