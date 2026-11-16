import Foundation

/// Launch arguments of the conditions screens, read in debug builds only (screenshots, UI
/// tests): `-customize` opens the Customize sheet on the conditions page, and
/// `-open-spot <id>` asks the spots list to open that spot.
enum ConditionsLaunch {
    private static var arguments: [String] {
        #if DEBUG
            ProcessInfo.processInfo.arguments
        #else
            []
        #endif
    }

    static var opensCustomize: Bool { arguments.contains("-customize") }

    static var openSpotID: String? {
        guard let index = arguments.firstIndex(of: "-open-spot"), index + 1 < arguments.count
        else { return nil }
        return arguments[index + 1]
    }
}
