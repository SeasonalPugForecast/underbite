import Foundation
import XCTest

/// The recorded samples under `Resources`, and JSON helpers the model tests share.
enum Fixtures {
    static func text(_ name: String) throws -> String {
        let url = try XCTUnwrap(
            Bundle.module.url(forResource: name, withExtension: "json", subdirectory: "Resources"),
            "missing resource \(name).json")
        return try String(contentsOf: url, encoding: .utf8)
    }

    static func object(_ json: String) throws -> Any {
        try JSONSerialization.jsonObject(with: Data(json.utf8), options: [.fragmentsAllowed])
    }

    static func string(_ object: Any) throws -> String {
        String(
            decoding: try JSONSerialization.data(
                withJSONObject: object, options: [.fragmentsAllowed]),
            as: UTF8.self)
    }

    /// `object` with an unknown field added to every dictionary in it, as a later version of
    /// the core might add.
    static func addingUnknownFields(_ object: Any) -> Any {
        if let dictionary = object as? [String: Any] {
            var out = dictionary.mapValues { addingUnknownFields($0) }
            out["x_later_field"] = ["nested": 1]
            return out
        }
        if let array = object as? [Any] { return array.map { addingUnknownFields($0) } }
        return object
    }

    /// Two JSON documents hold the same values (key order and number spelling do not matter).
    static func assertSameJSON(
        _ a: String, _ b: String, file: StaticString = #filePath, line: UInt = #line
    ) throws {
        let x = try object(a)
        let y = try object(b)
        XCTAssertTrue((x as AnyObject).isEqual(y), "the JSON differs", file: file, line: line)
    }
}
