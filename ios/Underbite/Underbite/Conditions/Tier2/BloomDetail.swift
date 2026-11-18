import SwiftUI
import UnderbiteKit

/// The algal-bloom card's full form: the health advice first, then the satellite estimate of a
/// lake and the coastal forecast of the nearest beach as the view model lays them out, then the
/// regional pages. Every word here is safety text: plain, whole, never shortened. A stale
/// composite reads "No recent satellite data", which the view model says, never "no bloom".
struct BloomDetail: View {
    let full: ViewModel.BloomFull

    var body: some View {
        Section {
            Label {
                Text(full.advisory)
                    .fixedSize(horizontal: false, vertical: true)
            } icon: {
                Image(systemName: "exclamationmark.triangle.fill")
                    .foregroundStyle(Palette.warning)
            }
            .accessibilityElement(children: .combine)
        }
        LabeledSections(sections: full.sections)
        PageLinks(links: full.links)
    }
}
