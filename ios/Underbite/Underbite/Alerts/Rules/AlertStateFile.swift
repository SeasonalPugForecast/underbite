import Foundation
@preconcurrency import UnderbiteKit

/// What the last check of the alert rules found (the bytes `evaluate_alerts` returns), kept
/// as they are in `alerts/state.json` in the app's directory (Application Support) and passed
/// back to the next check, to `alert_rules` and to `view_model`. Written atomically; a file
/// that cannot be read, or that the core cannot read, is set aside as `alerts/state.json.bad`
/// and the next check starts over.
nonisolated struct AlertStateFile: Sendable {
    /// The file, in the app's directory.
    static let name = "alerts/state.json"

    let files: AppFiles

    init(files: AppFiles = .standard) {
        self.files = files
    }

    /// The kept bytes; `nil` before the first check, or when the file could not be read (it
    /// is set aside).
    func read() -> Data? {
        do {
            return try files.data(Self.name)
        } catch {
            _ = files.setAside(Self.name)
            return nil
        }
    }

    /// Keeps `state` for the next check.
    @discardableResult
    func write(_ state: Data) -> Bool {
        files.write(state, Self.name)
    }

    /// Runs `body` with the kept bytes. When the core says it cannot read them, the file is
    /// set aside and `body` runs again with none.
    func using<T>(_ body: (Data?) throws -> T) throws -> T {
        let state = read()
        do {
            return try body(state)
        } catch AlertError.Invalid(let what, _) where what == Self.coreName && state != nil {
            _ = files.setAside(Self.name)
            return try body(nil)
        }
    }

    /// What the core calls the state in its errors.
    static let coreName = "alert state"
}
