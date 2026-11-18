#if DEBUG
    import Foundation

    /// `-seedTracks` (debug builds, screenshots): the walk along the Croton River from the
    /// import's synthetic sample file, saved as an imported track. Its id and import time are
    /// fixed and a track already saved is not saved twice, so every run shows the same one.
    nonisolated enum MineSeeds {
        /// The seeded track's id.
        static let trackID = "5EED0000-0000-4000-8000-00000000AC01"
        /// When it reads as imported, Unix seconds.
        static let importedAt: Int64 = 1_794_700_000

        /// The sample file's tracks, as the import would save them.
        static func tracks(settingsJSON: String) -> [SavedTrack] {
            let preview = ImportPreview.read(
                ImportSample.gpx, fileName: ImportSample.fileName, spots: [],
                settingsJSON: settingsJSON)
            var count = 0
            return preview.tracksToSave(now: importedAt) {
                defer { count += 1 }
                return count == 0 ? trackID : "\(trackID)-\(count)"
            }
        }

        /// Saves the sample's tracks in `store` when they are not there yet.
        @MainActor
        static func seed(into store: TrackStore, settingsJSON: String) {
            guard store.canSave else { return }
            _ = try? store.add(tracks(settingsJSON: settingsJSON))
        }
    }
#endif
