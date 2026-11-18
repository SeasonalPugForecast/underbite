#if DEBUG
    import Foundation

    /// The synthetic file of screenshots and tests (`-importFile sample`): a GPX at the test
    /// places with a waypoint at The Battery (the seeded spot's name and position, so it is
    /// flagged), one near the Croton River's mouth and a walk along the Croton in two segments.
    nonisolated enum ImportSample {
        static let fileName = "harbor-and-croton.synthetic.gpx"

        static let gpx = Data(
            """
            <?xml version="1.0" encoding="UTF-8"?>
            <gpx version="1.1" creator="synthetic" xmlns="http://www.topografix.com/GPX/1/1">
              <wpt lat="40.7033" lon="-74.0170">
                <time>2025-06-01T12:00:00Z</time>
                <name>The Battery</name>
                <desc>Stripers on the push</desc>
              </wpt>
              <wpt lat="41.1920" lon="-73.8830">
                <time>2025-05-10T12:30:00Z</time>
                <name>Croton mouth</name>
              </wpt>
              <trk>
                <name>Croton River walk</name>
                <trkseg>
                  <trkpt lat="41.2108" lon="-73.8625"><time>2025-05-10T13:00:00Z</time></trkpt>
                  <trkpt lat="41.2112" lon="-73.8631"><time>2025-05-10T13:05:00Z</time></trkpt>
                  <trkpt lat="41.2119" lon="-73.8640"><time>2025-05-10T13:10:00Z</time></trkpt>
                </trkseg>
                <trkseg>
                  <trkpt lat="41.2131" lon="-73.8652"/>
                  <trkpt lat="41.2140" lon="-73.8660"/>
                </trkseg>
              </trk>
            </gpx>
            """.utf8)
    }
#endif
