import SwiftUI
import UnderbiteKit
import WidgetKit

/// The extension's widgets. They read what the app wrote to the App Group and never fetch:
/// each reads only its own spot's payload (or the next trip's) and builds its timeline from the
/// payload's hourly entries; the app reloads them after it writes new payloads.
@main
struct UnderbiteWidgets: WidgetBundle {
    var body: some Widget {
        TideWidget()
        FlowWidget()
        SnackWidget()
        NextTripWidget()
    }
}

/// One moment of a widget's timeline, with the colours to paint it in.
struct UnderbiteEntry: TimelineEntry {
    let date: Date
    let kind: WidgetKind
    let moment: WidgetMoment
    let theme: ColorTheme

    /// The system's placeholder: the widget's shape, without values.
    static func placeholder(_ kind: WidgetKind) -> UnderbiteEntry {
        UnderbiteEntry(
            date: .now, kind: kind, moment: WidgetMoment(date: .now, name: nil, content: .gap(.loading)),
            theme: ColorPresets.underbite)
    }
}

extension WidgetKind {
    /// The widget's kind for WidgetKit (and `WidgetCenter.reloadTimelines(ofKind:)`).
    var widgetKind: String { "dev.savery.underbite.\(rawValue)" }
}

enum WidgetTimelines {
    /// The timeline for `kind` set to `choice`, from the shared container.
    static func timeline(_ kind: WidgetKind, choice: WidgetSpotChoice, now: Date = .now) -> Timeline<UnderbiteEntry> {
        let (payload, name, gap) = WidgetData.load(kind, choice: choice, store: .shared)
        let schedule = WidgetData.schedule(kind, payload: payload, name: name, missing: gap, now: now)
        let theme = ThemeStore.shared.resolved()
        let entries = schedule.moments.map {
            UnderbiteEntry(date: $0.date, kind: kind, moment: $0, theme: theme)
        }
        return Timeline(entries: entries, policy: .after(schedule.reload))
    }

    /// What `kind` shows now (the widget gallery's preview and a snapshot).
    static func entry(_ kind: WidgetKind, choice: WidgetSpotChoice, now: Date = .now) -> UnderbiteEntry {
        let (payload, name, gap) = WidgetData.load(kind, choice: choice, store: .shared)
        let moment = WidgetData.moment(kind, payload: payload, name: name, missing: gap, now: now)
        return UnderbiteEntry(date: now, kind: kind, moment: moment, theme: ThemeStore.shared.resolved())
    }
}

/// The tide, flow and snack time widgets: set to a spot in Edit Widget.
struct SpotProvider: AppIntentTimelineProvider {
    let kind: WidgetKind

    func placeholder(in context: Context) -> UnderbiteEntry {
        .placeholder(kind)
    }

    func snapshot(for configuration: SpotWidgetIntent, in context: Context) async -> UnderbiteEntry {
        WidgetTimelines.entry(kind, choice: configuration.choice)
    }

    func timeline(for configuration: SpotWidgetIntent, in context: Context) async -> Timeline<UnderbiteEntry> {
        WidgetTimelines.timeline(kind, choice: configuration.choice)
    }
}

/// The next trip's widget: there is one next trip, so nothing to set.
struct TripProvider: TimelineProvider {
    func placeholder(in context: Context) -> UnderbiteEntry {
        .placeholder(.nextTrip)
    }

    func getSnapshot(in context: Context, completion: @escaping (UnderbiteEntry) -> Void) {
        completion(WidgetTimelines.entry(.nextTrip, choice: .app))
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<UnderbiteEntry>) -> Void) {
        completion(WidgetTimelines.timeline(.nextTrip, choice: .app))
    }
}

/// A widget's view in WidgetKit: the family and the rendering mode from the system, the
/// theme from the entry, the system background behind (left out where the system tints).
struct UnderbiteWidgetBody: View {
    let entry: UnderbiteEntry
    @Environment(\.widgetFamily) private var family
    @Environment(\.widgetRenderingMode) private var renderingMode

    var body: some View {
        WidgetMomentView(kind: entry.kind, moment: entry.moment, family: family, paint: WidgetPaint(renderingMode))
            .environment(\.colorTheme, entry.theme)
            .containerBackground(for: .widget) {
                if family.isAccessory {
                    Color.clear
                } else {
                    Color(uiColor: .systemBackground)
                }
            }
    }
}

extension WidgetFamily {
    /// The Home Screen families every widget offers: small, medium and large on the iPhone, and
    /// extra large too on the iPad (the iPhone does not offer it).
    static var home: [WidgetFamily] { [.systemSmall, .systemMedium, .systemLarge, .systemExtraLarge] }
}

struct TideWidget: Widget {
    var body: some WidgetConfiguration {
        AppIntentConfiguration(
            kind: WidgetKind.tide.widgetKind, intent: SpotWidgetIntent.self, provider: SpotProvider(kind: .tide)
        ) { entry in
            UnderbiteWidgetBody(entry: entry)
        }
        .configurationDisplayName("Tide")
        .description("A spot's tide curve with its highs and lows. Touch and hold to choose the spot.")
        .supportedFamilies(WidgetFamily.home + [.accessoryRectangular])
    }
}

struct FlowWidget: Widget {
    var body: some WidgetConfiguration {
        AppIntentConfiguration(
            kind: WidgetKind.flow.widgetKind, intent: SpotWidgetIntent.self, provider: SpotProvider(kind: .flow)
        ) { entry in
            UnderbiteWidgetBody(entry: entry)
        }
        .configurationDisplayName("River flow")
        .description("A spot's river flow, its trend and the past week. Touch and hold to choose the spot.")
        .supportedFamilies(WidgetFamily.home + [.accessoryRectangular])
    }
}

struct SnackWidget: Widget {
    var body: some WidgetConfiguration {
        AppIntentConfiguration(
            kind: WidgetKind.snack.widgetKind, intent: SpotWidgetIntent.self, provider: SpotProvider(kind: .snack)
        ) { entry in
            UnderbiteWidgetBody(entry: entry)
        }
        .configurationDisplayName("Snack times")
        .description("A spot's next snack time, when fish feed. Touch and hold to choose the spot.")
        .supportedFamilies(WidgetFamily.home)
    }
}

struct NextTripWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: WidgetKind.nextTrip.widgetKind, provider: TripProvider()) { entry in
            UnderbiteWidgetBody(entry: entry)
        }
        .configurationDisplayName("Next trip")
        .description("Go or no-go for your next trip, with the reason.")
        .supportedFamilies(WidgetFamily.home)
    }
}
