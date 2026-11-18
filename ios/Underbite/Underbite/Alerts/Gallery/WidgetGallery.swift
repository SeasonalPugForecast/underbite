#if DEBUG
    import SwiftUI
    import UIKit
    import UnderbiteKit
    import WidgetKit

    extension View {
        /// `-widgetGallery` (debug builds, screenshots): every widget in every family over the
        /// app, from the sample payloads.
        func widgetGalleryOnLaunch() -> some View {
            modifier(WidgetGalleryLaunch())
        }
    }

    private struct WidgetGalleryLaunch: ViewModifier {
        @State private var shown = ProcessInfo.processInfo.arguments.contains("-widgetGallery")

        func body(content: Content) -> some View {
            content.fullScreenCover(isPresented: $shown) { WidgetGallery() }
        }
    }

    /// One widget of the gallery: a kind for a spot (or the next trip) at the payload's first
    /// entry.
    struct WidgetGalleryItem: Identifiable {
        let kind: WidgetKind
        let moment: WidgetMoment
        let title: String

        var id: String { "\(kind.rawValue)-\(title)" }
    }

    /// The sample payloads of The Battery, the Croton River and a trip to both, kept in the
    /// shared container as the app keeps its own, then read back the way the widgets read them.
    enum WidgetGallerySamples {
        static let spotFiles = ["sample-widget-battery", "sample-widget-croton"]
        static let tripFile = "sample-widget-trip"

        static func items(store: WidgetPayloadStore = .shared) throws -> [WidgetGalleryItem] {
            let spots = try spotFiles.map { try store.saveSpot(text($0)) }
            try store.saveNextTrip(text(tripFile))
            var list = store.spots()
            for payload in spots where !list.spots.contains(where: { $0.id == payload.id }) {
                list.spots.append(WidgetSpot(id: payload.id, name: payload.name))
            }
            if list.appSpotID == nil { list.appSpotID = spots.first?.id }
            try store.saveSpots(list)

            var items: [WidgetGalleryItem] = []
            for kind in [WidgetKind.tide, .flow, .snack] {
                for spot in spots {
                    items.append(item(kind, choice: .spot(spot.id), store: store))
                }
            }
            items.append(item(.nextTrip, choice: .app, store: store))
            return items
        }

        private static func item(
            _ kind: WidgetKind, choice: WidgetSpotChoice, store: WidgetPayloadStore
        ) -> WidgetGalleryItem {
            let (payload, name, gap) = WidgetData.load(kind, choice: choice, store: store)
            // At the payload's first entry, so every run draws the same.
            let now = payload.map { Date(timeIntervalSince1970: TimeInterval($0.generatedAt)) } ?? .now
            let moment = WidgetData.moment(kind, payload: payload, name: name, missing: gap, now: now)
            let title = [WidgetGapView.title(kind), payload?.name ?? name].compactMap { $0 }
            return WidgetGalleryItem(kind: kind, moment: moment, title: title.joined(separator: " · "))
        }

        private static func text(_ name: String) throws -> String {
            guard let url = Bundle.main.url(forResource: name, withExtension: "json") else {
                throw CocoaError(.fileNoSuchFile)
            }
            return try String(contentsOf: url, encoding: .utf8)
        }
    }

    /// The widget sizes of the device, in points (Apple's tables for an iPhone 6.1" to 6.3"
    /// and an 11" iPad).
    struct WidgetGallerySizes {
        let small: CGSize
        let medium: CGSize
        let large: CGSize
        let extraLarge: CGSize?
        let rectangular: CGSize

        static let iPhone = WidgetGallerySizes(
            small: CGSize(width: 158, height: 158), medium: CGSize(width: 338, height: 158),
            large: CGSize(width: 338, height: 354), extraLarge: nil,
            rectangular: CGSize(width: 160, height: 72))
        static let iPad = WidgetGallerySizes(
            small: CGSize(width: 155, height: 155), medium: CGSize(width: 342, height: 155),
            large: CGSize(width: 342, height: 342), extraLarge: CGSize(width: 715.5, height: 342),
            rectangular: CGSize(width: 160, height: 72))

        static let gap: CGFloat = 8
        static let titleHeight: CGFloat = 22

        /// One widget's block: its title, then the extra large (iPad), the large beside the
        /// medium over the small and the Lock Screen's (iPad), or each under the other (iPhone).
        var block: CGSize {
            let g = Self.gap
            if let extraLarge {
                return CGSize(width: extraLarge.width, height: Self.titleHeight + extraLarge.height + large.height + 2 * g)
            }
            return CGSize(width: large.width, height: Self.titleHeight + large.height + medium.height + small.height + 3 * g)
        }

        /// The columns that fit `count` blocks into `space` the largest, and their scale.
        func grid(count: Int, in space: CGSize, spacing: CGFloat) -> (columns: Int, scale: CGFloat) {
            var best = (columns: 1, scale: CGFloat(0))
            for columns in 1...max(count, 1) {
                let rows = (count + columns - 1) / columns
                let width = CGFloat(columns) * block.width + CGFloat(columns - 1) * spacing
                let height = CGFloat(rows) * block.height + CGFloat(rows - 1) * spacing
                let scale = min(space.width / width, space.height / height)
                if scale > best.scale { best = (columns, scale) }
            }
            return best
        }
    }

    /// Every widget in every family for the sample spots and trip, scaled to fit one screen, for
    /// screenshots (`-widgetGallery`); debug builds only.
    struct WidgetGallery: View {
        @Environment(\.colorScheme) private var scheme
        @State private var items: [WidgetGalleryItem] = []
        @State private var failure: String?

        private let sizes =
            UIDevice.current.userInterfaceIdiom == .pad ? WidgetGallerySizes.iPad : WidgetGallerySizes.iPhone
        private let spacing: CGFloat = 16

        var body: some View {
            GeometryReader { geometry in
                let space = CGSize(
                    width: geometry.size.width - 2 * spacing, height: geometry.size.height - 2 * spacing)
                let grid = sizes.grid(count: max(items.count, 1), in: space, spacing: spacing)
                let columns = Array(
                    repeating: GridItem(.fixed(sizes.block.width), spacing: spacing, alignment: .topLeading),
                    count: grid.columns)
                LazyVGrid(columns: columns, alignment: .leading, spacing: spacing) {
                    ForEach(items) { item in block(item) }
                }
                .frame(
                    width: CGFloat(grid.columns) * sizes.block.width + CGFloat(grid.columns - 1) * spacing,
                    alignment: .topLeading
                )
                .scaleEffect(grid.scale, anchor: .topLeading)
                .padding(spacing)
                .frame(width: geometry.size.width, height: geometry.size.height, alignment: .topLeading)
            }
            .background(wallpaper.ignoresSafeArea())
            .overlay {
                if let failure { Text(failure).padding().background(.background) }
            }
            .environment(\.colorTheme, ThemeStore.shared.resolved())
            .task {
                do {
                    items = try WidgetGallerySamples.items()
                } catch {
                    failure = String(describing: error)
                }
            }
        }

        private var wallpaper: Color {
            scheme == .dark ? Color(white: 0.22) : Color(white: 0.78)
        }

        @ViewBuilder private func block(_ item: WidgetGalleryItem) -> some View {
            let g = WidgetGallerySizes.gap
            VStack(alignment: .leading, spacing: g) {
                Text(item.title)
                    .font(.headline)
                    .lineLimit(1)
                    .frame(height: WidgetGallerySizes.titleHeight - g, alignment: .leading)
                if let extraLarge = sizes.extraLarge {
                    home(item, .systemExtraLarge, extraLarge)
                    HStack(alignment: .top, spacing: g) {
                        home(item, .systemLarge, sizes.large)
                        VStack(alignment: .leading, spacing: g) {
                            home(item, .systemMedium, sizes.medium)
                            HStack(alignment: .top, spacing: g) {
                                home(item, .systemSmall, sizes.small)
                                lock(item)
                            }
                        }
                    }
                } else {
                    home(item, .systemLarge, sizes.large)
                    home(item, .systemMedium, sizes.medium)
                    HStack(alignment: .top, spacing: g) {
                        home(item, .systemSmall, sizes.small)
                        lock(item)
                    }
                }
            }
            .frame(width: sizes.block.width, height: sizes.block.height, alignment: .topLeading)
        }

        /// A Home Screen widget: the system's margins and background, its corners.
        private func home(_ item: WidgetGalleryItem, _ family: WidgetFamily, _ size: CGSize) -> some View {
            WidgetMomentView(kind: item.kind, moment: item.moment, family: family, paint: .color)
                .padding(16)
                .frame(width: size.width, height: size.height)
                .background(Color(uiColor: .systemBackground), in: .rect(cornerRadius: 22))
        }

        /// The Lock Screen's widget, for the kinds that have one: one colour on the wallpaper.
        @ViewBuilder private func lock(_ item: WidgetGalleryItem) -> some View {
            if item.kind == .tide || item.kind == .flow {
                WidgetMomentView(kind: item.kind, moment: item.moment, family: .accessoryRectangular, paint: .mono)
                    .frame(width: sizes.rectangular.width, height: sizes.rectangular.height)
                    .padding(6)
                    .foregroundStyle(.white)
                    .background(Color.black.opacity(0.6), in: .rect(cornerRadius: 12))
                    .environment(\.colorScheme, .dark)
            }
        }
    }
#endif
