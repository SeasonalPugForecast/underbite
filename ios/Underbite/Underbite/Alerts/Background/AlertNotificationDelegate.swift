import Foundation
@preconcurrency import UserNotifications

/// The notification center's delegate, made by the app scene and kept by it (the center holds
/// its delegate weakly): an alert that arrives while the app is open shows as a banner too, and
/// stays in Notification Center, as it would with the app in the background.
nonisolated final class AlertNotificationDelegate: NSObject, UNUserNotificationCenterDelegate,
    Sendable
{
    /// A delegate set on the current notification center, for the scene to keep. Made as the
    /// app starts, so a notification tapped to open the app finds it.
    static func install() -> AlertNotificationDelegate {
        let delegate = AlertNotificationDelegate()
        UNUserNotificationCenter.current().delegate = delegate
        return delegate
    }

    /// How a notification posted while the app is in the foreground is shown.
    static let foregroundOptions: UNNotificationPresentationOptions = [.banner, .list, .sound]

    func userNotificationCenter(
        _ center: UNUserNotificationCenter, willPresent notification: UNNotification
    ) async -> UNNotificationPresentationOptions {
        Self.foregroundOptions
    }
}
