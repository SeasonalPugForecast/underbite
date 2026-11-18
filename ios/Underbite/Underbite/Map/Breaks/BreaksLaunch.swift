import Foundation
import UnderbiteKit

/// The breaks module's launch argument, read in debug builds only (screenshots):
/// `-mapBreaksSst <°C per km>` uses that temperature breaks threshold for the launch; nothing is
/// saved. The layers themselves are shown with the map's `-mapLayers`.
enum BreaksLaunch {
    static var sstThreshold: Double? {
        #if DEBUG
            guard let text = LaunchArguments.value(after: "-mapBreaksSst"),
                let value = Double(text), value > 0
            else { return nil }
            return value
        #else
            return nil
        #endif
    }

    /// `json` with the threshold a launch names.
    static func applied(to json: String) -> String {
        guard let threshold = sstThreshold, var settings = try? Settings(json: json) else {
            return json
        }
        settings.map.breaks.sstCPerKm = threshold
        return (try? settings.json()) ?? json
    }
}
