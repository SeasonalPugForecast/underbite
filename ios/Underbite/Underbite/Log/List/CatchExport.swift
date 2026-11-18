import SwiftUI
import UIKit
@preconcurrency import UnderbiteKit

/// The log's export as the list's toolbar offers it: a menu of the two formats in the core's
/// words, the core's warning that the file holds exact places before anything is made, then the
/// share sheet over a temporary file, which is removed once the sheet is done or dismissed.
struct CatchExportMenu: View {
    let catches: CatchStore
    let words: ExportWords
    /// The file being shared; set once the warning is accepted.
    @Binding var sharing: URL?
    @State private var asked: CatchStore.ExportFormat?
    @State private var problem: String?
    #if DEBUG
        private static var launched = false
    #endif

    var body: some View {
        Menu {
            Section(words.title) {
                Button(words.csv, systemImage: "tablecells") { asked = .csv }
                Button(words.gpx, systemImage: "mappin.and.ellipse") { asked = .gpx }
            }
        } label: {
            Label(words.title, systemImage: "square.and.arrow.up")
        }
        #if DEBUG
            .onAppear {
                // `-exportLog csv|gpx` (screenshot runs): asks for that export at launch.
                if let format = LaunchArguments.value(after: "-exportLog").flatMap(
                    CatchStore.ExportFormat.init(rawValue:)), !Self.launched
                {
                    Self.launched = true
                    asked = format
                }
            }
        #endif
        .alert(
            words.title,
            isPresented: Binding(get: { asked != nil }, set: { if !$0 { asked = nil } }),
            presenting: asked
        ) { format in
            Button(label(format)) { share(format) }
            Button("Cancel", role: .cancel) {}
        } message: { _ in
            // Safety text: plain in every voice and never shortened.
            Text(words.warning)
        }
        .alert(
            "The log could not be exported",
            isPresented: Binding(get: { problem != nil }, set: { if !$0 { problem = nil } }),
            presenting: problem
        ) { _ in
            Button("OK", role: .cancel) {}
        } message: { problem in
            Text(problem)
        }
    }

    private func label(_ format: CatchStore.ExportFormat) -> String {
        switch format {
        case .csv: words.csv
        case .gpx: words.gpx
        }
    }

    private func share(_ format: CatchStore.ExportFormat) {
        do {
            sharing = try catches.exportFile(format)
        } catch {
            problem = CatchStore.message(error)
        }
    }
}

/// Presents the share sheet over `file` from the view it is the background of (a popover from
/// its top trailing corner on the iPad), and removes the file with `CatchStore.discardExport`
/// once the sheet completes or is dismissed, or when the view goes away first.
struct ExportSharePresenter: UIViewControllerRepresentable {
    @Binding var file: URL?

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeUIViewController(context: Context) -> UIViewController {
        let host = UIViewController()
        host.view.backgroundColor = .clear
        host.view.isUserInteractionEnabled = false
        return host
    }

    func updateUIViewController(_ host: UIViewController, context: Context) {
        let coordinator = context.coordinator
        coordinator.done = { file = nil }
        guard let file, coordinator.shown == nil, host.presentedViewController == nil else { return }
        coordinator.shown = file
        let sheet = UIActivityViewController(activityItems: [file], applicationActivities: nil)
        sheet.completionWithItemsHandler = { _, _, _, _ in
            coordinator.finish()
        }
        if let popover = sheet.popoverPresentationController {
            let bounds = host.view.bounds
            popover.sourceView = host.view
            popover.sourceRect = CGRect(x: bounds.maxX - 44, y: bounds.minY, width: 1, height: 1)
            popover.permittedArrowDirections = [.up]
        }
        // Presented after this update, not during it.
        Task { @MainActor in
            guard host.presentedViewController == nil, host.view.window != nil else {
                coordinator.finish()
                return
            }
            host.present(sheet, animated: true)
        }
    }

    static func dismantleUIViewController(_ host: UIViewController, coordinator: Coordinator) {
        coordinator.done = nil
        coordinator.finish()
    }

    final class Coordinator {
        /// The file the sheet is showing.
        var shown: URL?
        /// Clears the binding.
        var done: (() -> Void)?

        /// Removes the shared file and clears the binding.
        func finish() {
            if let shown { CatchStore.discardExport(shown) }
            shown = nil
            done?()
        }
    }
}
