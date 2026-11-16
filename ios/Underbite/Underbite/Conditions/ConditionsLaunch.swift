import Foundation

/// Launch arguments of the conditions screens, read in debug builds only (screenshots, UI
/// tests): `-customize` opens the Customize sheet on the conditions page.
enum ConditionsLaunch {
    static var opensCustomize: Bool {
        #if DEBUG
            ProcessInfo.processInfo.arguments.contains("-customize")
        #else
            false
        #endif
    }
}
