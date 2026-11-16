import Foundation
import XCTest

@testable import UnderbiteKit

/// The page JSON of the recorded sample, decoded: what the cards carry, and that decoding
/// tolerates what later versions add.
final class ViewModelTests: XCTestCase {
    private func samplePage() throws -> ViewModel.Page {
        try ViewModel.decode(Fixtures.text("sample-view"))
    }

    func testTheSamplePageDecodes() throws {
        let page = try samplePage()
        XCTAssertEqual(page.active, 0)
        XCTAssertEqual(page.spots.count, 1)
        XCTAssertEqual(page.spots[0].id, "battery")
        XCTAssertEqual(page.spots[0].waterTypeLabel, "Tidal river or estuary")
        XCTAssertEqual(page.spots[0].status.updated, "updated 6 min ago")
        XCTAssertEqual(page.spots[0].status.lastSuccess, 1_794_000_000)
        XCTAssertNil(page.spots[0].status.error)

        let view = try XCTUnwrap(page.view)
        XCTAssertEqual(view.header.name, "The Battery")
        XCTAssertEqual(view.header.updatedAt, 1_794_000_000)
        XCTAssertTrue(view.notices.isEmpty)
        XCTAssertEqual(
            view.cards.compactMap { $0.chrome?.id },
            [
                "now", "alerts", "tide", "current", "wind", "water_temp", "waves",
                "marine_forecast", "flow", "sun_moon", "sources",
            ])
        XCTAssertTrue(view.cards.allSatisfy(\.isKnown))
        XCTAssertTrue(view.cards.allSatisfy { $0.compact != nil })
    }

    func testCardsCarryTheirFramesAndForms() throws {
        let view = try XCTUnwrap(samplePage().view)
        let cards = view.cards

        guard case .now(let now) = cards[0] else { return XCTFail("the first card is Now") }
        XCTAssertEqual(now.chrome.title, "Now")
        XCTAssertEqual(now.chrome.state, .fresh)
        XCTAssertEqual(now.compact.headline.text, "5.2")
        XCTAssertEqual(now.compact.headline.unit, "ft MLLW")
        XCTAssertEqual(try XCTUnwrap(now.compact.headline.value), 5.249344, accuracy: 1e-6)
        XCTAssertEqual(now.full.items.first?.card, "tide")
        XCTAssertTrue(now.chrome.notices.contains { $0.kind == "not_for_navigation" })
        XCTAssertFalse(now.chrome.credits.isEmpty)

        guard case .tide(let tide) = cards[2] else { return XCTFail("the third card is Tide") }
        XCTAssertFalse(tide.full.next.isEmpty)
        XCTAssertEqual(tide.full.next[0].kind, "high")
        XCTAssertFalse(tide.full.next[0].time.isEmpty)
        let chart = try XCTUnwrap(tide.full.chart)
        XCTAssertFalse(chart.series.isEmpty)
        XCTAssertFalse(chart.series[0].points.isEmpty)
        XCTAssertFalse(chart.accessibility.isEmpty)

        guard case .sources(let sources) = try XCTUnwrap(cards.last) else {
            return XCTFail("the last card is Sources")
        }
        XCTAssertFalse(sources.full.credits.isEmpty)
        XCTAssertFalse(sources.full.stations.isEmpty)
        XCTAssertFalse(sources.full.errors.isEmpty)
    }

    func testTheTimelineDecodes() throws {
        let timeline = try XCTUnwrap(samplePage().view?.timeline)
        XCTAssertGreaterThan(timeline.lanes.count, 0)
        XCTAssertLessThan(timeline.startT, timeline.endT)
        for lane in timeline.lanes {
            XCTAssertFalse(lane.label.isEmpty)
            XCTAssertFalse(lane.chart.accessibility.isEmpty)
        }
        // Lanes with no data or only bands have no series.
        XCTAssertTrue(timeline.lanes.contains { !$0.chart.series.isEmpty })
        let tides = try XCTUnwrap(timeline.lanes.first { $0.kind == "tide" })
        XCTAssertTrue(tides.notices.contains { $0.kind == "not_for_navigation" })
        XCTAssertFalse(tides.credits.isEmpty)
    }

    func testUnknownFieldsAreIgnored() throws {
        let original = try Fixtures.text("sample-view")
        let later = try Fixtures.string(Fixtures.addingUnknownFields(Fixtures.object(original)))
        XCTAssertNotEqual(original, later)
        XCTAssertEqual(try ViewModel.decode(later), try ViewModel.decode(original))
    }

    func testAnUnknownCardKindIsSkippedNotFatal() throws {
        var page = try XCTUnwrap(Fixtures.object(Fixtures.text("sample-view")) as? [String: Any])
        var view = try XCTUnwrap(page["view"] as? [String: Any])
        var cards = try XCTUnwrap(view["cards"] as? [Any])
        let known = cards.count
        let first = try XCTUnwrap(cards.first as? [String: Any])
        // One with a frame, one with nothing a decoder would know, one with the wrong shape.
        cards.append([
            "card": "stocking", "chrome": first["chrome"] as Any, "full": ["stocked": []],
        ])
        cards.append(["card": "future_card", "anything": [1, 2, 3]])
        cards.append(["card": "odd", "chrome": "not a frame"])
        view["cards"] = cards
        page["view"] = view

        let decoded = try ViewModel.decode(Fixtures.string(page))
        let all = try XCTUnwrap(decoded.view?.cards)
        XCTAssertEqual(all.count, known + 3)
        XCTAssertEqual(all.filter(\.isKnown).count, known)
        guard case .unknown(let kind, let chrome) = all[known] else {
            return XCTFail("a card of an unknown kind decodes as .unknown")
        }
        XCTAssertEqual(kind, "stocking")
        XCTAssertEqual(chrome?.id, "now")
        guard case .unknown("future_card", nil) = all[known + 1] else {
            return XCTFail("an unknown card without a frame has none")
        }
        guard case .unknown("odd", nil) = all[known + 2] else {
            return XCTFail("a frame that does not decode is dropped, the card kept")
        }
    }

    func testCardStates() throws {
        func state(_ json: String) throws -> ViewModel.CardState {
            try ViewModel.decoder().decode(ViewModel.CardState.self, from: Data(json.utf8))
        }
        XCTAssertEqual(try state(#"{"state":"fresh"}"#), .fresh)
        XCTAssertEqual(
            try state(#"{"state":"stale","reason":"6 h old"}"#), .stale(reason: "6 h old"))
        XCTAssertEqual(try state(#"{"state":"far","reason":"40 mi away"}"#).reason, "40 mi away")
        XCTAssertEqual(
            try state(#"{"state":"unavailable","reason":"no data"}"#),
            .unavailable(reason: "no data"))
        XCTAssertEqual(try state(#"{"state":"later"}"#), .unknown("later"))
    }

    // The sample has no pressure, solunar, clarity or storm card: decode these from a
    // hand-made page.
    func testTheCardsTheSampleLacksDecode() throws {
        func chrome(_ id: String) -> String {
            """
            "card": "\(id)", "chrome": {"id": "\(id)", "title": "T", "footnote": null,
              "credits": [], "state": {"state": "fresh"}, "notices": [], "accessibility": "a"},
              "compact": {"headline": {"value": null, "text": "—", "unit": ""}, "lines": [],
              "spark": [1.5, 2.5]}
            """
        }
        let quantity = #"{"value": 1.5, "text": "1.5", "unit": "in"}"#
        let json = """
            {"active": 0, "spots": [], "view": {"header": {"name": "n", "water_type": "pond",
              "water_type_label": "Pond", "updated_at": 1, "updated": "u"}, "timeline": null,
              "notices": [{"text": "no data"}], "cards": [
              {\(chrome("pressure")), "full": {"value": \(quantity),
                "trend": {"direction": "falling", "text": "Falling"}, "change": null,
                "chart": null, "sections": [{"title": "S", "rows":
                  [{"label": "l", "value": "v", "detail": null}]}]}},
              {\(chrome("solunar")), "full": {"days": [{"date": "Sun", "periods": [
                {"kind": "major", "cause": "upper_transit", "label": "Major", "start_t": 1,
                 "end_t": 2, "range": "1 to 2", "cause_label": "Moon overhead"}]}],
                "sections": []}},
              {\(chrome("clarity")), "full": {"turbidity": \(quantity), "chlorophyll": null,
                "kd490": {"available": false, "value": null, "note": "n"}, "chart": null,
                "sections": []}},
              {\(chrome("storm_ahead")), "full": {"event": {"title": "Nor'easter",
                "window": "Fri to Sat", "start": 1, "end": 2, "severity": "strong",
                "headline": {"kind": "snow", "text": "Snow", "amount": \(quantity)},
                "agreement": {"agreement": "high", "label": "High", "sentence": "s"},
                "models": [{"model": "m", "snow": \(quantity), "precipitation": \(quantity),
                  "gust": \(quantity), "low": \(quantity), "low_at": "Sat", "onset": "Fri"}]},
                "hurricanes": [{"id": "al01", "name": "N", "title": "T", "classification": "c",
                  "category": 2, "severity": "severe", "wind": \(quantity),
                  "pressure": \(quantity), "movement": "m", "strength": "s", "advisory": "a",
                  "advisory_url": null, "reaches": "r", "reaches_at": 5, "leaves_at": 6}],
                "sections": []}}
            ]}}
            """
        let view = try XCTUnwrap(ViewModel.decode(json).view)
        XCTAssertEqual(view.notices.map(\.text), ["no data"])
        XCTAssertEqual(view.cards.count, 4)
        guard case .pressure(let pressure) = view.cards[0],
            case .solunar(let solunar) = view.cards[1],
            case .clarity(let clarity) = view.cards[2],
            case .stormAhead(let storm) = view.cards[3]
        else { return XCTFail("the four cards decode as their kinds") }
        XCTAssertEqual(pressure.compact.spark, [1.5, 2.5])
        XCTAssertEqual(pressure.full.trend?.direction, "falling")
        XCTAssertEqual(pressure.full.sections[0].rows[0].detail, nil)
        XCTAssertEqual(solunar.full.days[0].periods[0].causeLabel, "Moon overhead")
        XCTAssertEqual(solunar.full.days[0].periods[0].startT, 1)
        XCTAssertFalse(clarity.full.kd490.available)
        XCTAssertEqual(clarity.full.turbidity?.unit, "in")
        XCTAssertEqual(storm.full.event?.severity, "strong")
        XCTAssertEqual(storm.full.event?.models.count, 1)
        XCTAssertEqual(storm.full.hurricanes[0].category, 2)
        XCTAssertEqual(storm.full.hurricanes[0].reachesAt, 5)
    }

    func testAPageWithoutASnapshotHasNoView() throws {
        let json = """
            {"active": 0, "spots": [{"id": "a", "name": "A", "water_type": "pond",
              "water_type_label": "Pond", "status": {"last_success": null, "updated": null,
              "error": "offline", "refreshing": true}}], "view": null}
            """
        let page = try ViewModel.decode(json)
        XCTAssertNil(page.view)
        XCTAssertEqual(page.spots[0].status.error, "offline")
        XCTAssertTrue(page.spots[0].status.refreshing)
    }
}
