import Foundation

/// The records module's launch arguments, read in debug builds only (screenshots); nothing is
/// saved, and a release build reads none:
/// - `-mapRecordsSpecies <id>` looks up that species (a catalogue id, `morone-saxatilis`),
/// - `-mapRecordsFind` presses the species records' lookup button once, when the map has
///   settled with the layer on,
/// - `-mapRecordsOpen` then picks out a record as if tapped, once.
///
/// The USGS Nonindigenous Aquatic Species state comes from the species pages' region, which a
/// launch can set for itself with `-speciesRegion US-NY` (the argument domain of
/// `UserDefaults`).
struct RecordsLaunch {
    #if DEBUG
        let species: String? = LaunchArguments.value(after: "-mapRecordsSpecies")
        var wantsFind = ProcessInfo.processInfo.arguments.contains("-mapRecordsFind")
        var wantsOpen = ProcessInfo.processInfo.arguments.contains("-mapRecordsOpen")
    #else
        let species: String? = nil
        var wantsFind = false
        var wantsOpen = false
    #endif
}
