/// The modules the map draws, bottom first: one line per module. Later sections add theirs here.
enum MapModules {
    @MainActor
    static func make(controller: MapController) -> [any MapModule] {
        [
            SourcesModule(controller: controller),
            StationsModule(controller: controller),
        ]
    }
}
