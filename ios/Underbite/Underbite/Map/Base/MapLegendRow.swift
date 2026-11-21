import SwiftUI

/// The map's legends above the sources strip: one alone as it is, two or more side by side in one
/// row, each about half the width, and stacked when the text size is too large for that.
struct MapLegendRow<Content: View>: View {
    /// How many legends the content holds.
    let count: Int
    @ViewBuilder var content: () -> Content
    @Environment(\.dynamicTypeSize) private var typeSize

    /// Whether the legends sit side by side: more than one, at a text size that leaves room.
    static func sideBySide(count: Int, typeSize: DynamicTypeSize) -> Bool {
        count > 1 && typeSize <= .xxLarge
    }

    var body: some View {
        if Self.sideBySide(count: count, typeSize: typeSize) {
            HStack(alignment: .top, spacing: 6) { content() }
        } else {
            VStack(alignment: .leading, spacing: 6) { content() }
        }
    }
}
