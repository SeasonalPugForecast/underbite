import Foundation

/// The folder the app shares with its widgets and Live Activities: `widgets` in the App Group's
/// container, which both targets are entitled to. Without the App Group (an unsigned build, the
/// package tests) it is a folder in the temporary directory instead, which the widgets never
/// see, and `backing` says so.
public struct SharedContainer: Sendable, Hashable, CustomStringConvertible {
    /// Where the folder is.
    public enum Backing: String, Sendable, Hashable {
        /// In the App Group's container, read by the widgets too.
        case appGroup = "app_group"
        /// In the temporary directory: the widgets do not see it, and the system may empty it.
        case temporary
    }

    /// The App Group the app and its widgets share (as the theme store's).
    public static let appGroupID = ThemeStore.appGroupID
    /// The folder's name, in the container or the temporary directory.
    static let folderName = "widgets"
    /// The fallback's name in the temporary directory.
    static let temporaryName = "underbite-shared"

    /// The folder: it exists once `directory()` has made it.
    public let url: URL
    public let backing: Backing

    public init(url: URL, backing: Backing) {
        self.url = url
        self.backing = backing
    }

    /// Whether the widgets read what is written here.
    public var isShared: Bool { backing == .appGroup }

    /// Where the payloads are, in words, for the debug screens and logs.
    public var description: String {
        switch backing {
        case .appGroup:
            "App Group container (\(Self.appGroupID)), shared with the widgets"
        case .temporary:
            "temporary directory (no App Group): the widgets do not see these payloads"
        }
    }

    /// The app's and the widgets' folder: in the App Group's container when the group is there,
    /// else in the temporary directory.
    public static let shared: SharedContainer = {
        let fileManager = FileManager.default
        return resolve(
            groupContainer: fileManager.containerURL(
                forSecurityApplicationGroupIdentifier: appGroupID),
            temporaryDirectory: fileManager.temporaryDirectory
        ) { url in
            var isDirectory: ObjCBool = false
            return fileManager.fileExists(
                atPath: url.path(percentEncoded: false), isDirectory: &isDirectory)
                && isDirectory.boolValue
        }
    }()

    /// The folder for a group container as `FileManager` answers it (`nil` when the app is not
    /// entitled to the group), with the theme store's test of whether the group is there.
    static func resolve(
        groupContainer: URL?, temporaryDirectory: URL, containerExists: (URL) -> Bool
    ) -> SharedContainer {
        if let groupContainer,
            ThemeStore.backing(groupContainer: groupContainer, containerExists: containerExists)
                == .appGroup
        {
            return SharedContainer(
                url: groupContainer.appending(path: folderName, directoryHint: .isDirectory),
                backing: .appGroup)
        }
        return SharedContainer(
            url: temporaryDirectory
                .appending(path: temporaryName, directoryHint: .isDirectory)
                .appending(path: folderName, directoryHint: .isDirectory),
            backing: .temporary)
    }

    /// The folder, made if it is missing.
    public func directory() throws -> URL {
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }
}
