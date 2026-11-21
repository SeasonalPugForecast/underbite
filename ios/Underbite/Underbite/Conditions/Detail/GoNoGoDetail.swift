import SwiftUI
import UnderbiteKit

/// The go/no-go card's full form: the profile, then each day's verdict, its reasons whole (a
/// safety reason is never shortened; a hurricane's links to the official page) and its hours.
/// The verdict is its word and a symbol as well as a colour. All text is the view model's.
struct GoNoGoDetail: View {
    let full: ViewModel.GoNoGoFull

    var body: some View {
        Section { Text(full.profile) }
        ForEach(full.days, id: \.date) { day in
            Section(day.date) {
                Label {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(day.verdictLabel).font(.headline)
                        Text(day.summary)
                    }
                } icon: {
                    Image(systemName: Self.symbol(day.verdict)).foregroundStyle(Self.color(day.verdict))
                }
                .accessibilityElement(children: .combine)
                ForEach(Array(day.reasons.enumerated()), id: \.offset) { _, reason in
                    if let link = reason.link.flatMap(URL.init(string:)) {
                        Link(reason.text, destination: link)
                    } else {
                        Text(reason.text)
                    }
                }
                if !day.hours.isEmpty {
                    HoursStrip(hours: day.hours)
                }
            }
        }
    }

    static func symbol(_ verdict: String) -> String {
        switch verdict {
        case "go": "checkmark.circle.fill"
        case "marginal": "exclamationmark.circle.fill"
        case "no_go": "xmark.octagon.fill"
        default: "questionmark.circle"
        }
    }

    static func color(_ verdict: String) -> Color {
        switch verdict {
        case "go": .green
        case "marginal": .orange
        case "no_go": .red
        default: .secondary
        }
    }
}

/// The day's hours, side by side in a strip: the hour's label over its symbol, VoiceOver reading
/// the label, the verdict's word through the symbol and the leading reason.
private struct HoursStrip: View {
    let hours: [ViewModel.GoNoGoHourVm]
    /// Which sides have more hours out of view; the strip fades on those sides and shows its
    /// scroll indicator, so it reads as something to swipe.
    @State private var more = HoursOutOfView(before: false, after: true)

    var body: some View {
        ScrollView(.horizontal) {
            HStack(alignment: .top, spacing: 12) {
                ForEach(hours, id: \.startT) { hour in
                    VStack(spacing: 2) {
                        Text(hour.label).font(.caption)
                        Image(systemName: GoNoGoDetail.symbol(hour.verdict))
                            .foregroundStyle(GoNoGoDetail.color(hour.verdict))
                        if let reason = hour.reason {
                            Text(reason).font(.caption2).foregroundStyle(.readableSecondary)
                                .frame(maxWidth: 90)
                        }
                    }
                    .accessibilityElement(children: .combine)
                    .accessibilityLabel(Text([hour.label, hour.reason].compactMap { $0 }.joined(separator: ", ")))
                }
            }
            .padding(.vertical, 4)
        }
        .scrollIndicators(.visible)
        .onScrollGeometryChange(for: HoursOutOfView.self) { geometry in
            HoursOutOfView(
                before: geometry.contentOffset.x > 1,
                after: geometry.contentOffset.x + geometry.containerSize.width
                    < geometry.contentSize.width - 1)
        } action: { _, new in
            more = new
        }
        .mask {
            HStack(spacing: 0) {
                LinearGradient(
                    colors: [more.before ? .clear : .black, .black],
                    startPoint: .leading, endPoint: .trailing
                )
                .frame(width: 28)
                Rectangle()
                LinearGradient(
                    colors: [.black, more.after ? .clear : .black],
                    startPoint: .leading, endPoint: .trailing
                )
                .frame(width: 28)
            }
        }
    }
}

/// Whether hours of the strip lie out of view before and after the part shown.
private struct HoursOutOfView: Equatable {
    var before: Bool
    var after: Bool
}
