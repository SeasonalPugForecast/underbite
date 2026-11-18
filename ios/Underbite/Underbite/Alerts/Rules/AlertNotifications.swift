import UIKit
@preconcurrency import UnderbiteKit
@preconcurrency import UserNotifications

/// Whether the app may post notifications, as the system says.
nonisolated enum NotificationPermission: Equatable, Sendable {
    /// Not asked yet: the app asks once a rule is saved or checked.
    case notAsked
    /// Allowed (also provisionally, or for a while).
    case allowed
    /// Turned off in Settings: the system shows nothing and ignores what the app posts.
    case off
}

/// The app's notification permission and its local notifications. Every word posted is the
/// core's (`evaluate_alerts`).
nonisolated enum AlertNotifications {
    /// The system's answer now.
    static func permission() async -> NotificationPermission {
        let settings = await UNUserNotificationCenter.current().notificationSettings()
        return permission(settings.authorizationStatus)
    }

    static func permission(_ status: UNAuthorizationStatus) -> NotificationPermission {
        switch status {
        case .notDetermined: .notAsked
        case .denied: .off
        default: .allowed
        }
    }

    /// Asks once (the system shows its prompt only the first time), then says what was chosen.
    static func ask() async -> NotificationPermission {
        _ = try? await UNUserNotificationCenter.current().requestAuthorization(
            options: [.alert, .sound, .badge])
        return await permission()
    }

    /// Posts `notices` at once, one per rule: a newer one for a rule replaces the older.
    static func post(_ notices: [AlertNotice]) async {
        guard await permission() == .allowed else { return }
        let center = UNUserNotificationCenter.current()
        for notice in notices {
            let content = UNMutableNotificationContent()
            content.title = notice.title
            content.body = notice.body
            content.sound = .default
            content.threadIdentifier = "alerts"
            content.interruptionLevel = notice.timeSensitive ? .timeSensitive : .active
            let request = UNNotificationRequest(
                identifier: "alert-rule-" + notice.ruleId, content: content, trigger: nil)
            try? await center.add(request)
        }
    }

    /// The app's own page in the Settings app, where notifications are turned on.
    static var settingsURL: URL? {
        URL(string: UIApplication.openNotificationSettingsURLString)
    }
}
