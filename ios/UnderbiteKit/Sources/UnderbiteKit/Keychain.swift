import Foundation
import Security

/// The keys of the sources that need one, in the keychain: this device only, never in files,
/// backups or logs. One generic-password item per account, under the app's service.
public enum Keychain {
    /// The service of every item: the app's bundle identifier, fixed so that a build under
    /// another identifier and the app's extensions read the same items.
    public static let service = "dev.savery.underbite"

    /// What is kept: one item each.
    public enum Account: String, CaseIterable, Sendable {
        /// The USGS Water Data API key (api.data.gov).
        case usgs
        /// The Copernicus Marine user name.
        case copernicusUser = "copernicus-user"
        /// The Copernicus Marine password.
        case copernicusPassword = "copernicus-password"
    }

    private static func query(_ account: Account) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account.rawValue,
        ]
    }

    /// The stored value for `account`, if any.
    public static func key(for account: Account) -> String? {
        var item: CFTypeRef?
        var q = query(account)
        q[kSecReturnData as String] = true
        q[kSecMatchLimit as String] = kSecMatchLimitOne
        guard SecItemCopyMatching(q as CFDictionary, &item) == errSecSuccess,
            let data = item as? Data
        else { return nil }
        return String(data: data, encoding: .utf8)
    }

    /// Stores `value` for `account`, or removes the item when it is empty. A key loses
    /// surrounding spaces and line breaks, a password only line breaks. Readable after the
    /// first unlock, so background refreshes find it; never moved to another device.
    public static func setKey(_ value: String, for account: Account) {
        SecItemDelete(query(account) as CFDictionary)
        let trimmed = value.trimmingCharacters(in: trimmed(for: account))
        guard !trimmed.isEmpty else { return }
        var q = query(account)
        q[kSecValueData as String] = Data(trimmed.utf8)
        q[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        SecItemAdd(q as CFDictionary, nil)
    }

    /// Every stored value, for one fetch (`Keys` is never stored or logged by the core).
    public static func keys() -> Keys {
        Keys(
            usgs: key(for: .usgs),
            copernicusUser: key(for: .copernicusUser),
            copernicusPassword: key(for: .copernicusPassword)
        )
    }

    private static func trimmed(for account: Account) -> CharacterSet {
        account == .copernicusPassword ? .newlines : .whitespacesAndNewlines
    }
}
