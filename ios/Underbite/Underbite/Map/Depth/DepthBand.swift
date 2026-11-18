import Foundation
@preconcurrency import UnderbiteKit

/// The depth band ("show 15–25 ft"): the depths the shading highlights, kept in the settings'
/// `map.depth.band_m` as metres below the surface, shallow then deep. Its ends move over the
/// core's round stops in the user's depth unit (`depth_band_stops`), each with the value to
/// store and its label; nothing is converted here.
nonisolated enum DepthBand {
    /// The stops the band's ends can take, shallow first.
    static func stops(settingsJSON: String) -> [DisplayStop] {
        (try? depthBandStops(settingsJson: settingsJSON)) ?? []
    }

    /// The stops nearest the band's ends (`band`, metres, shallow then deep), as positions in
    /// `stops`; none without a band or with fewer than two stops. Two ends nearest the same stop
    /// are kept a stop apart.
    static func positions(of band: [Double]?, in stops: [DisplayStop]) -> ClosedRange<Int>? {
        guard let band, band.count == 2, stops.count > 1 else { return nil }
        let nearest = { (value: Double) -> Int in
            stops.indices.min { abs(stops[$0].value - value) < abs(stops[$1].value - value) } ?? 0
        }
        var low = nearest(min(band[0], band[1]))
        var high = nearest(max(band[0], band[1]))
        if low == high {
            if high < stops.count - 1 { high += 1 } else { low -= 1 }
        }
        return low...high
    }

    /// Writes the band between the stops at `positions` into `map`; none, or ends on the same
    /// stop, clears it.
    static func set(
        _ positions: ClosedRange<Int>?, stops: [DisplayStop], in map: inout Settings.MapSettings
    ) {
        guard let positions, positions.lowerBound < positions.upperBound,
            stops.indices.contains(positions.lowerBound), stops.indices.contains(positions.upperBound)
        else {
            map.depth.bandM = nil
            return
        }
        map.depth.bandM = [stops[positions.lowerBound].value, stops[positions.upperBound].value]
    }

    /// The position a drag at `fraction` (0 at the shallow end of the track, 1 at the deep end)
    /// lands on, among `count` stops.
    static func position(at fraction: Double, count: Int) -> Int {
        guard count > 1 else { return 0 }
        let clamped = min(max(fraction, 0), 1)
        return Int((clamped * Double(count - 1)).rounded())
    }

    /// The band after one end moves to `position`: the shallow end stays above the deep one,
    /// a stop apart at least.
    static func moved(
        _ band: ClosedRange<Int>, shallow: Bool, to position: Int, count: Int
    ) -> ClosedRange<Int> {
        guard count > 1 else { return band }
        if shallow {
            let low = min(max(position, 0), band.upperBound - 1)
            return low...band.upperBound
        }
        let high = max(min(position, count - 1), band.lowerBound + 1)
        return band.lowerBound...high
    }
}
