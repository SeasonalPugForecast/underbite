import SwiftUI
import UnderbiteKit

/// One pack: its area, what it holds, where its download stands and the controls for it
/// (download, pause, resume, cancel, refresh, delete), with what its plan downloads.
struct PackDetailView: View {
    let id: String
    let now: Int64

    @Environment(SpotStore.self) private var store
    @Environment(\.dismiss) private var dismiss
    @State private var plan: Result<PackPlanInfo, PackPlanProblem>?
    @State private var confirmsDelete = false

    private var library: PackLibrary { .shared }

    private var words: PackWords? { try? packWords(settingsJson: store.settingsJSON) }

    var body: some View {
        if let record = library.record(id), let info = record.info, let words {
            List {
                Section {
                    PackThumbnail(area: info.area)
                        .frame(height: 220)
                        .listRowInsets(EdgeInsets())
                }
                Section {
                    if let row = row(record) {
                        LabeledContent("On this device", value: row.sizeText)
                        Text(row.refreshedText)
                        Text(row.datesText)
                        if let stale = row.staleText {
                            Label(stale, systemImage: "clock.badge.exclamationmark")
                                .foregroundStyle(Palette.warning)
                        }
                    }
                    PackStatusLine(record: record, words: words)
                }
                Section {
                    controls(record, info: info)
                }
                Section {
                    switch plan {
                    case .success(let plan):
                        LabeledContent("Download", value: plan.sizeText)
                        Text(plan.timeText).readableText()
                        Text(plan.countsText).readableText()
                        if let over = plan.overBudgetText {
                            Label(over, systemImage: "exclamationmark.triangle.fill")
                                .foregroundStyle(Palette.warning)
                        }
                        ForEach(plan.skipped, id: \.layer) { skip in
                            LabeledContent(skip.title, value: skip.reason)
                        }
                    case .failure(let problem):
                        Text(problem.message).foregroundStyle(Palette.warning)
                    case nil:
                        ProgressView()
                    }
                } header: {
                    Text("Estimate")
                }
                Section {
                    if library.wifiOnly {
                        Text(words.wifiOnlyDetail).font(.footnote).readableText()
                    }
                    Text(words.backgroundRefreshNote).font(.footnote).readableText()
                    Text(words.baseMapNote).font(.footnote).readableText()
                    DrawerNotice(text: words.notice)
                }
            }
            .listStyle(.insetGrouped)
            .navigationTitle(info.name)
            .navigationBarTitleDisplayMode(.inline)
            .task(id: record.json) {
                plan = await library.estimate(record.json, settingsJSON: store.settingsJSON)
            }
            .confirmationDialog(info.name, isPresented: $confirmsDelete, titleVisibility: .visible) {
                Button("Delete", role: .destructive) {
                    Task { await library.delete(id) }
                }
            }
        }
    }

    @ViewBuilder
    private func controls(_ record: PackRecord, info: PackInfo) -> some View {
        let settings = store.settingsJSON
        let running = library.isRunning(id)
        if running {
            Button {
                library.pause(id)
            } label: {
                Label("Pause", systemImage: "pause.circle")
            }
            .accessibilityIdentifier("pack-pause")
        } else if record.progress != nil {
            Button {
                Task { await PackBackground.download(id, settingsJSON: settings) }
            } label: {
                Label("Resume", systemImage: "play.circle")
            }
            .accessibilityIdentifier("pack-resume")
        } else if record.status == .done {
            Button {
                Task { await PackBackground.download(id, settingsJSON: settings) }
            } label: {
                Label("Refresh now", systemImage: "arrow.clockwise.circle")
            }
            .accessibilityIdentifier("pack-refresh")
        } else {
            Button {
                Task { await PackBackground.download(id, settingsJSON: settings) }
            } label: {
                Label("Download", systemImage: "arrow.down.circle")
            }
            .disabled(overBudget)
            .accessibilityIdentifier("pack-download")
        }
        if running || record.progress != nil {
            Button(role: .destructive) {
                Task { await library.cancel(id) }
            } label: {
                Label("Cancel download", systemImage: "xmark.circle")
            }
            .accessibilityIdentifier("pack-cancel")
        }
        Button(role: .destructive) {
            confirmsDelete = true
        } label: {
            Label("Delete pack", systemImage: "trash")
        }
        .accessibilityIdentifier("pack-delete")
    }

    private var overBudget: Bool {
        if case .success(let plan) = plan { return plan.overBudget }
        return false
    }

    private func row(_ record: PackRecord) -> PackRow? {
        try? packRow(
            packJson: record.json,
            storedBytes: UInt64(max(0, library.storedBytes[record.id] ?? 0)), now: now,
            settingsJson: store.settingsJSON, timeZone: TimeZone.current.identifier)
    }
}
