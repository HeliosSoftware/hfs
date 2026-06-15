import HFSOperations
import SwiftUI

/// A tracked Bulk Data `$export` job, owned by the Bulk Jobs screen.
struct BulkExportJob: Identifiable {
    enum State {
        case running(progress: String?)
        case completed(BulkExportManifest)
        case failed(String)
        case cancelled
    }

    let id = UUID()
    let level: BulkExportLevel
    let groupID: String?
    let types: [String]
    let since: String?
    let statusURL: URL
    let startedAt: Date
    var state: State

    var isRunning: Bool {
        if case .running = state { return true }
        return false
    }

    /// A one-line description of what was requested.
    var requestSummary: String {
        var parts = [level.label]
        if level == .group, let groupID { parts.append("Group/\(groupID)") }
        if !types.isEmpty { parts.append("_type=\(types.joined(separator: ","))") }
        if let since, !since.isEmpty { parts.append("_since=\(since)") }
        return parts.joined(separator: " · ")
    }

    var badgeKind: HFSJobStatusBadge.Kind {
        switch state {
        case .running: .running
        case .completed: .completed
        case .failed: .failed
        case .cancelled: .cancelled
        }
    }

    var badgeDetail: String? {
        switch state {
        case .running(let progress): progress
        default: nil
        }
    }
}

struct HFSBulkJobsView: View {
    @Environment(HFSAppModel.self) private var model

    @State private var jobs: [BulkExportJob] = []
    @State private var selectedJobID: BulkExportJob.ID?

    // Kick-off form fields.
    @State private var level: BulkExportLevel = .patient
    @State private var groupID = ""
    @State private var typesText = ""
    @State private var sinceText = ""
    @State private var isStarting = false
    @State private var kickoffError: String?

    /// Toggled by the toolbar "Refresh Jobs" action in the root view.
    @Binding var refreshJobsRequested: Bool

    var body: some View {
        if model.isConnected {
            content
        } else {
            ContentUnavailableView {
                Label("Not Connected", systemImage: "bolt.horizontal.circle")
            } description: {
                Text("Connect to a server in Settings to run export jobs.")
            }
        }
    }

    private var content: some View {
        HStack(spacing: 0) {
            kickoffForm
                .frame(minWidth: 280, idealWidth: 320, maxWidth: 380)
                .frame(maxHeight: .infinity)

            Divider()

            jobsList
                .frame(minWidth: 220, idealWidth: 300, maxWidth: 420)

            Divider()

            jobDetail
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
        .onChange(of: refreshJobsRequested) { _, requested in
            guard requested else { return }
            Task { await pollAll() }
            refreshJobsRequested = false
        }
        .task {
            // Lightweight auto-poll: refresh running jobs every couple seconds
            // for the lifetime of this screen.
            await autoPollLoop()
        }
    }

    // MARK: - Kick-off form

    private var kickoffForm: some View {
        Form {
            Section("New Export") {
                Picker("Level", selection: $level) {
                    ForEach(BulkExportLevel.allCases) { level in
                        Text(level.label).tag(level)
                    }
                }

                if level == .group {
                    TextField("Group ID", text: $groupID, prompt: Text("e.g. 1234"))
                        #if !os(macOS)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        #endif
                }

                TextField("_type", text: $typesText, prompt: Text("Patient,Observation"))
                    #if !os(macOS)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    #endif

                TextField("_since", text: $sinceText, prompt: Text("2024-01-01T00:00:00Z"))
                    #if !os(macOS)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    #endif
            }

            Section {
                Button {
                    Task { await startExport() }
                } label: {
                    HStack(spacing: 6) {
                        if isStarting {
                            ProgressView().controlSize(.small)
                        }
                        Text(isStarting ? "Starting…" : "Start Export")
                    }
                }
                .buttonStyle(.borderedProminent)
                .disabled(isStarting || (level == .group && groupID.trimmingCharacters(in: .whitespaces).isEmpty))

                if let kickoffError {
                    Label(kickoffError, systemImage: "exclamationmark.triangle.fill")
                        .foregroundStyle(.red)
                        .font(.callout)
                        .labelStyle(.titleAndIcon)
                }
            } footer: {
                Text("Kicks off an async export (Prefer: respond-async) and polls the returned status URL.")
            }
        }
        .formStyle(.grouped)
    }

    // MARK: - Jobs list

    @ViewBuilder
    private var jobsList: some View {
        if jobs.isEmpty {
            ContentUnavailableView {
                Label("No Jobs", systemImage: "arrow.up.doc")
            } description: {
                Text("Start an export to track its progress here.")
            }
        } else {
            List(selection: $selectedJobID) {
                ForEach(jobs) { job in
                    jobRow(job).tag(job.id)
                }
            }
        }
    }

    private func jobRow(_ job: BulkExportJob) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text(job.level.label)
                    .font(.callout.weight(.medium))
                Spacer()
                HFSJobStatusBadge(kind: job.badgeKind, detail: job.badgeDetail)
            }
            Text(job.requestSummary)
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
            Text(job.startedAt, style: .relative)
                .font(.caption2)
                .foregroundStyle(.tertiary)
        }
        .padding(.vertical, 2)
    }

    // MARK: - Job detail / manifest inspector

    @ViewBuilder
    private var jobDetail: some View {
        if let job = selectedJob {
            VStack(alignment: .leading, spacing: 0) {
                detailHeader(job)
                Divider()
                ScrollView {
                    detailBody(job)
                        .padding()
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        } else {
            ContentUnavailableView {
                Label("No Job Selected", systemImage: "doc.plaintext")
            } description: {
                Text("Select a job to view its status and manifest.")
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    private func detailHeader(_ job: BulkExportJob) -> some View {
        HStack(alignment: .top, spacing: 10) {
            VStack(alignment: .leading, spacing: 4) {
                Text("\(job.level.label) Export")
                    .font(.headline)
                Text(job.statusURL.absoluteString)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }

            Spacer()

            Button {
                Task { await poll(job.id) }
            } label: {
                Label("Refresh", systemImage: "arrow.clockwise")
            }
            .labelStyle(.iconOnly)
            .buttonStyle(.borderless)
            .help("Poll this job's status")
            .disabled(!job.isRunning)

            Button(role: .destructive) {
                Task { await cancel(job.id) }
            } label: {
                Label("Cancel", systemImage: "xmark.octagon")
            }
            .labelStyle(.iconOnly)
            .buttonStyle(.borderless)
            .foregroundStyle(.red)
            .help("Cancel and delete this job")
            .disabled(!job.isRunning)
        }
        .padding(.horizontal)
        .padding(.vertical, 10)
    }

    @ViewBuilder
    private func detailBody(_ job: BulkExportJob) -> some View {
        switch job.state {
        case .running(let progress):
            GroupBox {
                HStack(spacing: 10) {
                    ProgressView().controlSize(.small)
                    Text(progress.map { "Processing — \($0)" } ?? "Processing…")
                        .foregroundStyle(.secondary)
                    Spacer(minLength: 0)
                }
            }
        case .failed(let message):
            ContentUnavailableView {
                Label("Export Failed", systemImage: "exclamationmark.triangle")
            } description: {
                Text(message)
            }
        case .cancelled:
            ContentUnavailableView {
                Label("Export Cancelled", systemImage: "slash.circle")
            } description: {
                Text("This job was cancelled and deleted on the server.")
            }
        case .completed(let manifest):
            manifestView(manifest)
        }
    }

    @ViewBuilder
    private func manifestView(_ manifest: BulkExportManifest) -> some View {
        VStack(alignment: .leading, spacing: 16) {
            GroupBox("Manifest") {
                VStack(alignment: .leading, spacing: 6) {
                    if let time = manifest.transactionTime {
                        LabeledContent("Transaction Time", value: time)
                    }
                    if let requiresToken = manifest.requiresAccessToken {
                        LabeledContent("Requires Access Token", value: requiresToken ? "Yes" : "No")
                    }
                    LabeledContent("Output Files", value: "\(manifest.output.count)")
                    if !manifest.error.isEmpty {
                        LabeledContent("Error Files", value: "\(manifest.error.count)")
                    }
                    if let request = manifest.request {
                        LabeledContent("Request") {
                            Text(request)
                                .textSelection(.enabled)
                                .lineLimit(1)
                                .truncationMode(.middle)
                        }
                    }
                }
                .font(.callout)
                .frame(maxWidth: .infinity, alignment: .leading)
            }

            if manifest.output.isEmpty {
                ContentUnavailableView {
                    Label("No Output", systemImage: "tray")
                } description: {
                    Text("The export completed with no output files.")
                }
            } else {
                fileTable("Output", files: manifest.output)
            }

            if !manifest.error.isEmpty {
                fileTable("Errors", files: manifest.error)
            }
            if !manifest.deleted.isEmpty {
                fileTable("Deleted", files: manifest.deleted)
            }
        }
    }

    private func fileTable(_ title: String, files: [BulkExportFile]) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title)
                .font(.subheadline.weight(.semibold))

            Table(files) {
                TableColumn("Type", value: \.type)
                TableColumn("Count") { file in
                    Text(file.count.map(String.init) ?? "—")
                }
                TableColumn("File") { file in
                    Text(file.url.lastPathComponent)
                        .textSelection(.enabled)
                        .help(file.url.absoluteString)
                }
            }
            .frame(minHeight: 120, idealHeight: 220)
        }
    }

    // MARK: - Actions

    private var selectedJob: BulkExportJob? {
        jobs.first { $0.id == selectedJobID }
    }

    private func startExport() async {
        guard let operations = model.bulkDataOperations() else {
            kickoffError = "Not connected to a server."
            return
        }
        kickoffError = nil
        isStarting = true
        defer { isStarting = false }

        let types = typesText
            .split(separator: ",")
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .filter { !$0.isEmpty }
        let since = sinceText.trimmingCharacters(in: .whitespaces)
        let trimmedGroup = groupID.trimmingCharacters(in: .whitespaces)

        do {
            let kickoff = try await operations.kickOff(
                level: level,
                groupID: level == .group ? trimmedGroup : nil,
                types: types,
                since: since.isEmpty ? nil : since
            )
            let job = BulkExportJob(
                level: level,
                groupID: level == .group ? trimmedGroup : nil,
                types: types,
                since: since.isEmpty ? nil : since,
                statusURL: kickoff.statusURL,
                startedAt: Date(),
                state: .running(progress: nil)
            )
            jobs.insert(job, at: 0)
            selectedJobID = job.id
            await poll(job.id)
        } catch {
            kickoffError = HFSAppModel.describe(error)
        }
    }

    private func poll(_ jobID: BulkExportJob.ID) async {
        guard
            let operations = model.bulkDataOperations(),
            let job = jobs.first(where: { $0.id == jobID }),
            job.isRunning
        else { return }

        do {
            let status = try await operations.status(url: job.statusURL)
            switch status {
            case .inProgress(let progress):
                updateState(jobID, .running(progress: progress))
            case .complete(let manifest):
                updateState(jobID, .completed(manifest))
            }
        } catch {
            updateState(jobID, .failed(HFSAppModel.describe(error)))
        }
    }

    private func pollAll() async {
        for job in jobs where job.isRunning {
            await poll(job.id)
        }
    }

    private func cancel(_ jobID: BulkExportJob.ID) async {
        guard
            let operations = model.bulkDataOperations(),
            let job = jobs.first(where: { $0.id == jobID })
        else { return }

        do {
            try await operations.cancel(url: job.statusURL)
            updateState(jobID, .cancelled)
        } catch {
            updateState(jobID, .failed(HFSAppModel.describe(error)))
        }
    }

    private func autoPollLoop() async {
        while !Task.isCancelled {
            try? await Task.sleep(for: .seconds(2))
            if jobs.contains(where: { $0.isRunning }) {
                await pollAll()
            }
        }
    }

    private func updateState(_ jobID: BulkExportJob.ID, _ state: BulkExportJob.State) {
        guard let index = jobs.firstIndex(where: { $0.id == jobID }) else { return }
        jobs[index].state = state
    }
}
