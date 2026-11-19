import MapKit
import SwiftUI
import UnderbiteKit

/// A pack's area on a small map: the box outlined, the map not movable.
struct PackThumbnail: View {
    let area: PackBox

    var body: some View {
        Map(initialPosition: .region(area.region), interactionModes: []) {
            MapPolygon(coordinates: area.corners)
                .foregroundStyle(Palette.accent.opacity(0.15))
                .stroke(Palette.accent, lineWidth: 2)
        }
        .mapStyle(.standard(pointsOfInterest: .excludingAll))
        .mapControlVisibility(.hidden)
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }
}

/// The new pack's area: the part of the map inside the frame. Moving and zooming the map sets
/// it.
struct PackAreaMap: View {
    @Binding var area: PackBox?
    @State private var position: MapCameraPosition

    init(area: Binding<PackBox?>) {
        _area = area
        _position = State(
            initialValue: area.wrappedValue.map { .region($0.region) } ?? .automatic)
    }

    var body: some View {
        Map(position: $position, interactionModes: [.pan, .zoom]) {}
            .mapStyle(.standard(pointsOfInterest: .excludingAll))
            .onMapCameraChange(frequency: .onEnd) { context in
                area = PackBox(region: context.region)
            }
            .overlay {
                Rectangle()
                    .strokeBorder(Palette.accent, lineWidth: 3)
                    .allowsHitTesting(false)
            }
            .clipShape(RoundedRectangle(cornerRadius: 12))
    }
}

extension PackBox {
    /// The map region of the box (across the antimeridian when `west` > `east`).
    var region: MKCoordinateRegion {
        let span = west <= east ? east - west : east + 360 - west
        var center = west + span / 2
        if center > 180 { center -= 360 }
        return MKCoordinateRegion(
            center: CLLocationCoordinate2D(latitude: (south + north) / 2, longitude: center),
            span: MKCoordinateSpan(latitudeDelta: north - south, longitudeDelta: span))
    }

    /// The box's corners, for its outline.
    var corners: [CLLocationCoordinate2D] {
        [
            CLLocationCoordinate2D(latitude: north, longitude: west),
            CLLocationCoordinate2D(latitude: north, longitude: east),
            CLLocationCoordinate2D(latitude: south, longitude: east),
            CLLocationCoordinate2D(latitude: south, longitude: west),
        ]
    }

    /// The box of a map region, within the latitudes a map shows.
    init(region: MKCoordinateRegion) {
        let halfLat = region.span.latitudeDelta / 2
        let halfLon = min(region.span.longitudeDelta / 2, 180)
        func wrap(_ lon: Double) -> Double {
            var l = lon
            while l > 180 { l -= 360 }
            while l < -180 { l += 360 }
            return l
        }
        self.init(
            west: wrap(region.center.longitude - halfLon),
            south: max(-85, region.center.latitude - halfLat),
            east: wrap(region.center.longitude + halfLon),
            north: min(85, region.center.latitude + halfLat))
    }
}
