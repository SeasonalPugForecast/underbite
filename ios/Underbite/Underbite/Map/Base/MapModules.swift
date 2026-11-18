/// The modules the map draws, bottom first: one line per module. Later sections add theirs here.
enum MapModules {
    @MainActor
    static func make(controller: MapController) -> [any MapModule] {
        [
            InspectorModule(controller: controller),
            DepthModule(controller: controller),
            SourcesModule(controller: controller),
            BreaksModule(controller: controller),
            StructureModule(controller: controller),
            RecordsModule(controller: controller),
            StationsModule(controller: controller),
            AccessModule(controller: controller),
            MineModule(controller: controller),
        ]
    }
}
