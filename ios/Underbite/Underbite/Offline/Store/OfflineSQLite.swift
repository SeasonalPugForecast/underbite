import Foundation
import SQLite3

/// Why the offline store failed.
nonisolated enum OfflineStoreError: Error, Equatable, Sendable {
    /// The file could not be opened, or is not a database.
    case open(String)
    /// The file was written by another version of the store (`found`; 0 for a database that is
    /// not the store's) and this one reads `expected`. It is left as it is.
    case schema(found: Int64, expected: Int64)
    /// SQLite refused a statement.
    case sqlite(code: Int32, message: String)
}

/// A value bound to, or read from, a statement.
nonisolated enum OfflineSQLValue: Equatable, Sendable {
    case int(Int64)
    case text(String)
    case blob(Data)
    case null
}

/// One SQLite connection and its prepared statements. It is not thread-safe: the store's actor
/// owns the one instance and nothing else touches it (the connection is opened in SQLite's
/// multi-thread mode, which allows that use from whichever thread runs the actor).
nonisolated final class OfflineSQLite {
    private let db: OpaquePointer
    private var statements: [String: OpaquePointer] = [:]
    private var inTransaction = false

    /// Opens, or creates, the database at `path`.
    init(path: String) throws {
        var handle: OpaquePointer?
        let flags = SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE | SQLITE_OPEN_NOMUTEX
        let code = sqlite3_open_v2(path, &handle, flags, nil)
        guard code == SQLITE_OK, let handle else {
            let message = handle.map { String(cString: sqlite3_errmsg($0)) } ?? "out of memory"
            // A handle comes back even on failure and is released either way.
            sqlite3_close_v2(handle)
            throw OfflineStoreError.open(message)
        }
        db = handle
    }

    deinit {
        for statement in statements.values { sqlite3_finalize(statement) }
        sqlite3_close_v2(db)
    }

    /// Runs `sql`, binding `values` to its parameters in order, and returns its rows read by
    /// `read`.
    @discardableResult
    func rows<T>(
        _ sql: String, _ values: [OfflineSQLValue] = [], _ read: (Row) throws -> T
    ) throws -> [T] {
        let statement = try prepared(sql)
        defer {
            sqlite3_reset(statement)
            sqlite3_clear_bindings(statement)
        }
        for (i, value) in values.enumerated() {
            try bind(value, at: Int32(i + 1), in: statement)
        }
        var out: [T] = []
        while true {
            let code = sqlite3_step(statement)
            if code == SQLITE_DONE { return out }
            guard code == SQLITE_ROW else { throw failure(code) }
            out.append(try read(Row(statement: statement)))
        }
    }

    /// Runs `sql`, which returns no rows.
    func run(_ sql: String, _ values: [OfflineSQLValue] = []) throws {
        try rows(sql, values) { _ in () }
    }

    /// The first column of the first row of `sql`, as an integer; none without rows or for NULL.
    func integer(_ sql: String, _ values: [OfflineSQLValue] = []) throws -> Int64? {
        try rows(sql, values) { $0.isNull(0) ? nil : $0.int(0) }.first ?? nil
    }

    /// The rows the last statement changed.
    var changes: Int { Int(sqlite3_changes(db)) }

    /// Runs `body` in one write transaction, rolled back if it throws. A transaction already
    /// open takes `body` into it.
    func transaction<T>(_ body: () throws -> T) throws -> T {
        if inTransaction { return try body() }
        try run("BEGIN IMMEDIATE")
        inTransaction = true
        defer { inTransaction = false }
        do {
            let result = try body()
            try run("COMMIT")
            return result
        } catch {
            try? run("ROLLBACK")
            throw error
        }
    }

    /// A row being read; valid only inside the `read` closure of ``rows(_:_:_:)``.
    nonisolated struct Row {
        fileprivate let statement: OpaquePointer

        func isNull(_ column: Int32) -> Bool {
            sqlite3_column_type(statement, column) == SQLITE_NULL
        }

        func int(_ column: Int32) -> Int64 {
            sqlite3_column_int64(statement, column)
        }

        func text(_ column: Int32) -> String? {
            guard let text = sqlite3_column_text(statement, column) else { return nil }
            return String(cString: text)
        }

        func blob(_ column: Int32) -> Data {
            let count = Int(sqlite3_column_bytes(statement, column))
            guard count > 0, let bytes = sqlite3_column_blob(statement, column) else {
                return Data()
            }
            return Data(bytes: bytes, count: count)
        }
    }

    private func prepared(_ sql: String) throws -> OpaquePointer {
        if let statement = statements[sql] { return statement }
        var statement: OpaquePointer?
        let code = sqlite3_prepare_v2(db, sql, -1, &statement, nil)
        guard code == SQLITE_OK, let statement else {
            sqlite3_finalize(statement)
            throw failure(code)
        }
        statements[sql] = statement
        return statement
    }

    private func bind(_ value: OfflineSQLValue, at index: Int32, in statement: OpaquePointer)
        throws
    {
        let code: Int32
        switch value {
        case .int(let n):
            code = sqlite3_bind_int64(statement, index, n)
        case .text(let s):
            code = sqlite3_bind_text(statement, index, s, -1, Self.transient)
        case .blob(let data) where data.isEmpty:
            // An empty buffer has no address, and a null pointer would bind NULL.
            code = sqlite3_bind_zeroblob(statement, index, 0)
        case .blob(let data):
            code = data.withUnsafeBytes {
                sqlite3_bind_blob64(
                    statement, index, $0.baseAddress, sqlite3_uint64($0.count), Self.transient)
            }
        case .null:
            code = sqlite3_bind_null(statement, index)
        }
        guard code == SQLITE_OK else { throw failure(code) }
    }

    private func failure(_ code: Int32) -> OfflineStoreError {
        let message = String(cString: sqlite3_errmsg(db))
        if code == SQLITE_NOTADB { return .open(message) }
        return .sqlite(code: code, message: message)
    }

    /// `SQLITE_TRANSIENT`: SQLite copies the bound bytes before the bind returns.
    private static var transient: sqlite3_destructor_type {
        unsafeBitCast(-1, to: sqlite3_destructor_type.self)
    }
}
