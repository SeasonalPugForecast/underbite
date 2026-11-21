import UnderbiteKit

/// Which reasons a go/no-go day lists under its verdict.
enum GoNoGoReasons {
    /// The day's reasons without the one the verdict line already says, so no sentence is shown
    /// twice. The comparison is on the view model's own text, ignoring surrounding whitespace.
    static func listed(
        _ reasons: [ViewModel.GoNoGoReasonVm], verdictLine: String
    ) -> [ViewModel.GoNoGoReasonVm] {
        let said = verdictLine.trimmingCharacters(in: .whitespacesAndNewlines)
        return reasons.filter { $0.text.trimmingCharacters(in: .whitespacesAndNewlines) != said }
    }
}
