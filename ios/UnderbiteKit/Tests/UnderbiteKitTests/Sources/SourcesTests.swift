import Foundation
import XCTest

@testable import UnderbiteKit

/// The chain edits against the core, the keys' state and the export.
final class SourcesTests: XCTestCase {
    private func defaults() throws -> Settings { try Settings.defaults() }

    private func assertValid(_ settings: Settings, file: StaticString = #filePath, line: UInt = #line) throws {
        let json = try settings.json()
        XCTAssertEqual(
            validateSettings(json: json).filter { $0.severity == .error }, [], file: file, line: line)
    }

    // MARK: Chains

    func testEveryChainEditIsValidSettings() throws {
        let base = try defaults()
        for kind in Preferences.SourceKind.allCases {
            let chain = kind.group(in: base.sources).chain
            guard chain.count > 1 else { continue }
            var edited = base
            SourceChains.move(kind, fromOffsets: IndexSet(integer: 0), toOffset: chain.count, in: &edited.sources)
            XCTAssertEqual(kind.group(in: edited.sources).chain, Array(chain.dropFirst()) + [chain[0]])
            try assertValid(edited)

            SourceChains.setUsed(false, id: chain[0], kind: kind, in: &edited.sources)
            XCTAssertFalse(kind.group(in: edited.sources).chain.contains(chain[0]))
            try assertValid(edited)

            SourceChains.setUsed(true, id: chain[0], kind: kind, in: &edited.sources)
            XCTAssertEqual(kind.group(in: edited.sources).chain.last, chain[0])
            try assertValid(edited)

            SourceChains.reset(kind, defaults: base.sources, in: &edited.sources)
            XCTAssertTrue(SourceChains.isDefault(kind, sources: edited.sources, defaults: base.sources))
        }
    }

    func testEditsKeepTheLimitsAndOtherChains() throws {
        let base = try defaults()
        var edited = base
        SourceChains.setUsed(false, id: "usgs", kind: .waterTemperature, in: &edited.sources)
        XCTAssertEqual(edited.sources.waterTemperature.maxDistanceKm, base.sources.waterTemperature.maxDistanceKm)
        XCTAssertEqual(edited.sources.waterTemperature.maxAgeMinutes, base.sources.waterTemperature.maxAgeMinutes)
        XCTAssertEqual(edited.sources.flow, base.sources.flow)
        SourceChains.reset(.waterTemperature, defaults: base.sources, in: &edited.sources)
        XCTAssertEqual(edited, base)
    }

    func testMovingFollowsListSemantics() {
        let items = ["a", "b", "c", "d"]
        XCTAssertEqual(SourceChains.moved(items, fromOffsets: IndexSet(integer: 0), toOffset: 3), ["b", "c", "a", "d"])
        XCTAssertEqual(SourceChains.moved(items, fromOffsets: IndexSet(integer: 3), toOffset: 1), ["a", "d", "b", "c"])
        XCTAssertEqual(SourceChains.moved(items, fromOffsets: IndexSet(integer: 1), toOffset: 2), items)
        XCTAssertEqual(SourceChains.moved(items, fromOffsets: IndexSet(integer: 9), toOffset: 0), items)
    }

    func testRowsListTheChainThenTheSourcesLeftOut() throws {
        let base = try defaults()
        let catalogue = sourceCatalogue()
        var edited = base
        SourceChains.setUsed(false, id: "coops", kind: .waterTemperature, in: &edited.sources)
        let rows = SourceChains.rows(.waterTemperature, sources: edited.sources, defaults: base.sources, catalogue: catalogue)
        let chain = edited.sources.waterTemperature.chain
        XCTAssertEqual(rows.prefix(chain.count).map(\.id), chain)
        XCTAssertTrue(rows.prefix(chain.count).allSatisfy(\.used))
        XCTAssertEqual(rows.suffix(from: chain.count).map(\.id), ["coops"])
        XCTAssertEqual(rows.last?.used, false)
        XCTAssertEqual(rows.last?.name, catalogue.first { $0.id == "coops" }?.name)
    }

    func testSourcesNotImplementedStayInTheChainAndAreMarked() throws {
        let base = try defaults()
        let catalogue = sourceCatalogue()
        let rows = SourceChains.rows(.waterTemperature, sources: base.sources, defaults: base.sources, catalogue: catalogue)
        let mur = try XCTUnwrap(rows.first { $0.id == "mur-sst" })
        XCTAssertFalse(mur.implemented)
        XCTAssertTrue(mur.used)
        let coops = try XCTUnwrap(rows.first { $0.id == "coops" })
        XCTAssertTrue(coops.implemented)

        var odd = base
        odd.sources.waterTemperature.chain.append("somewhere-else")
        let withOdd = SourceChains.rows(.waterTemperature, sources: odd.sources, defaults: base.sources, catalogue: catalogue)
        let unknown = try XCTUnwrap(withOdd.last)
        XCTAssertEqual(unknown.id, "somewhere-else")
        XCTAssertEqual(unknown.name, "somewhere-else")
        XCTAssertFalse(unknown.known)
        XCTAssertFalse(unknown.implemented)
        // It stays valid (a warning at most) and is not dropped.
        try assertValid(odd)
    }

    func testEveryDefaultChainSourceIsInTheCatalogue() throws {
        let base = try defaults()
        let ids = Set(sourceCatalogue().map(\.id))
        for kind in Preferences.SourceKind.allCases {
            for id in kind.group(in: base.sources).chain {
                XCTAssertTrue(ids.contains(id), "\(kind) names \(id)")
            }
        }
    }

    func testCatalogueTellsWhichSourcesTakeKeys() {
        let catalogue = sourceCatalogue()
        let usgs = catalogue.first { $0.id == "usgs" }
        XCTAssertEqual(usgs?.keyOptional, true)
        let copernicus = catalogue.first { $0.id == "copernicus" }
        XCTAssertEqual(copernicus?.optIn, true)
        XCTAssertEqual(copernicus?.needsKey, true)
        XCTAssertEqual(copernicus?.implemented, false)
    }

    // MARK: Keys

    private final class MemoryStore: KeyStoring {
        var items: [Keychain.Account: String] = [:]
        var reads = 0
        func value(for account: Keychain.Account) -> String? {
            reads += 1
            return items[account]
        }
        func set(_ value: String, for account: Keychain.Account) {
            let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
            items[account] = trimmed.isEmpty ? nil : trimmed
        }
    }

    func testKeyStateReportsSavedWithoutRevealingTheKey() {
        let store = MemoryStore()
        XCTAssertEqual(KeyState(store: store), KeyState())
        var state = KeyState.save("  abc123  ", as: .usgs, in: store)
        XCTAssertTrue(state.usgs)
        XCTAssertFalse(state.copernicusAccount)
        XCTAssertEqual(store.items[.usgs], "abc123")
        state = KeyState.save("   ", as: .copernicusUser, in: store)
        XCTAssertFalse(state.copernicusUser)
        state = KeyState.save("someone", as: .copernicusUser, in: store)
        state = KeyState.save("secret", as: .copernicusPassword, in: store)
        XCTAssertTrue(state.copernicusAccount)
        state = KeyState.remove(.usgs, from: store)
        XCTAssertFalse(state.usgs)
        XCTAssertNil(store.items[.usgs])
        XCTAssertTrue(state.copernicusAccount)
    }

    func testExportNeverReadsTheKeysAndHoldsNone() throws {
        let store = MemoryStore()
        _ = KeyState.save("sentinel-usgs-key", as: .usgs, in: store)
        _ = KeyState.save("sentinel-user", as: .copernicusUser, in: store)
        _ = KeyState.save("sentinel-password", as: .copernicusPassword, in: store)
        let reads = store.reads
        let json = try defaults().json()
        let toml = try SettingsExport.toml(settingsJSON: json)
        XCTAssertEqual(store.reads, reads)
        for text in [json, toml] {
            XCTAssertFalse(text.contains("sentinel"))
        }
        // The export reads back as the same settings.
        let back = try importSettingsToml(text: toml)
        XCTAssertEqual(try Settings(json: back.json), try defaults())
    }

    func testTheSettingsDocumentHasNoFieldForAKey() throws {
        let json = try defaults().json().lowercased()
        for word in ["api_key", "apikey", "password", "token", "secret"] {
            XCTAssertFalse(json.contains(word), word)
        }
    }
}
