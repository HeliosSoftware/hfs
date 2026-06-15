import HFSOperations
import SwiftUI

struct HFSSubscriptionsView: View {
    @Environment(HFSAppModel.self) private var model

    @State private var statusFilter: SubscriptionStatus?
    @State private var items: [SubscriptionSummary] = []
    @State private var total: Int?
    @State private var nextURL: URL?
    @State private var isLoading = false
    @State private var isLoadingMore = false
    @State private var errorMessage: String?
    @State private var selectedItemID: SubscriptionSummary.ID?
    @State private var hasSearched = false

    @State private var showCreateEditor = false
    @State private var deleteCandidate: SubscriptionSummary?

    /// Toggled by the toolbar "New Subscription" action in the root view.
    @Binding var newSubscriptionRequested: Bool

    private let pageSize = 20

    private let template = """
    {
      "resourceType" : "Subscription",
      "status" : "requested",
      "reason" : "Example subscription",
      "criteria" : "Patient?",
      "channel" : {
        "type" : "rest-hook",
        "endpoint" : "https://example.com/hook",
        "payload" : "application/fhir+json"
      }
    }
    """

    var body: some View {
        if model.isConnected {
            content
        } else {
            ContentUnavailableView {
                Label("Not Connected", systemImage: "bolt.horizontal.circle")
            } description: {
                Text("Connect to a server in Settings to manage subscriptions.")
            }
        }
    }

    private var content: some View {
        HStack(spacing: 0) {
            filterForm
                .frame(minWidth: 260, idealWidth: 300, maxWidth: 360)
                .frame(maxHeight: .infinity)

            Divider()

            subscriptionsColumn
                .frame(minWidth: 260, idealWidth: 340, maxWidth: 460)

            Divider()

            subscriptionDetail
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
        .task {
            if !hasSearched { await runSearch() }
        }
        .onChange(of: newSubscriptionRequested) { _, requested in
            guard requested else { return }
            showCreateEditor = true
            newSubscriptionRequested = false
        }
        .sheet(isPresented: $showCreateEditor) {
            HFSResourceEditorView(
                title: "New Subscription",
                saveButtonTitle: "Create",
                initialJSON: template
            ) { data in
                await performCreate(data)
            }
        }
        .confirmationDialog(
            deleteCandidate.map { "Delete Subscription/\($0.fhirID)?" } ?? "Delete subscription?",
            isPresented: Binding(
                get: { deleteCandidate != nil },
                set: { if !$0 { deleteCandidate = nil } }
            ),
            titleVisibility: .visible,
            presenting: deleteCandidate
        ) { item in
            Button("Delete", role: .destructive) {
                Task { await performDelete(item) }
            }
            Button("Cancel", role: .cancel) {}
        } message: { _ in
            Text("This permanently deletes the subscription on the server.")
        }
    }

    // MARK: - Filter form

    private var filterForm: some View {
        Form {
            Section("Filters") {
                Picker("Status", selection: $statusFilter) {
                    Text("Any").tag(SubscriptionStatus?.none)
                    ForEach(SubscriptionStatus.allCases) { value in
                        Text(value.label).tag(SubscriptionStatus?.some(value))
                    }
                }
            }

            Section {
                Button {
                    Task { await runSearch() }
                } label: {
                    HStack(spacing: 6) {
                        if isLoading {
                            ProgressView().controlSize(.small)
                        }
                        Text(isLoading ? "Loading…" : "Refresh")
                    }
                }
                .buttonStyle(.borderedProminent)
                .disabled(isLoading)

                Button {
                    showCreateEditor = true
                } label: {
                    Label("New Subscription", systemImage: "plus")
                }

                LabeledContent("Loaded", value: countSummary)
            } footer: {
                Text("Subscriptions are FHIR Subscription resources. Requires the server's subscriptions engine to deliver notifications.")
            }
        }
        .formStyle(.grouped)
    }

    private var countSummary: String {
        if isLoading { return "Loading…" }
        if let total { return "\(items.count) of \(total)" }
        return "\(items.count)"
    }

    // MARK: - Subscriptions list

    @ViewBuilder
    private var subscriptionsColumn: some View {
        if isLoading, items.isEmpty {
            ProgressView("Loading subscriptions…")
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if let errorMessage {
            ContentUnavailableView {
                Label("Couldn’t Load", systemImage: "exclamationmark.triangle")
            } description: {
                Text(errorMessage)
            } actions: {
                Button("Retry") { Task { await runSearch() } }
            }
        } else if items.isEmpty {
            ContentUnavailableView {
                Label("No Subscriptions", systemImage: "dot.radiowaves.left.and.right")
            } description: {
                Text("No Subscription resources match the current filter.")
            } actions: {
                Button("New Subscription") { showCreateEditor = true }
            }
        } else {
            VStack(spacing: 0) {
                List(selection: $selectedItemID) {
                    ForEach(items) { item in
                        subscriptionRow(item).tag(item.id)
                    }
                }

                if nextURL != nil {
                    Divider()
                    HStack(spacing: 8) {
                        Button {
                            Task { await loadMore() }
                        } label: {
                            HStack(spacing: 6) {
                                if isLoadingMore {
                                    ProgressView().controlSize(.small)
                                }
                                Text(isLoadingMore ? "Loading…" : "Load \(pageSize) more")
                            }
                        }
                        .disabled(isLoadingMore)

                        Spacer()

                        Text("\(items.count) loaded")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    .padding(8)
                }
            }
        }
    }

    private func subscriptionRow(_ item: SubscriptionSummary) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text(item.channelType ?? "Subscription")
                    .font(.callout.weight(.medium))
                Spacer()
                HFSSubscriptionStatusBadge(status: item.status)
            }
            if let endpoint = item.endpoint {
                Text(endpoint)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            if let topic = item.topic {
                Text(topic)
                    .font(.caption2)
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
        }
        .padding(.vertical, 2)
    }

    // MARK: - Subscription detail

    @ViewBuilder
    private var subscriptionDetail: some View {
        if let item = selectedItem {
            VStack(alignment: .leading, spacing: 0) {
                detailHeader(item)
                Divider()
                ScrollView {
                    VStack(alignment: .leading, spacing: 16) {
                        summaryCard(item)
                        jsonCard(item)
                    }
                    .padding()
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        } else {
            ContentUnavailableView {
                Label("No Subscription Selected", systemImage: "dot.radiowaves.right")
            } description: {
                Text("Select a subscription to inspect its channel, endpoint, and raw JSON.")
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    private func detailHeader(_ item: SubscriptionSummary) -> some View {
        HStack(alignment: .top, spacing: 10) {
            VStack(alignment: .leading, spacing: 4) {
                Text(item.channelType.map { "\($0) Subscription" } ?? "Subscription")
                    .font(.headline)
                Text("Subscription/\(item.fhirID)")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }

            Spacer()

            HFSSubscriptionStatusBadge(status: item.status)

            Button(role: .destructive) {
                deleteCandidate = item
            } label: {
                Label("Delete", systemImage: "trash")
            }
            .labelStyle(.iconOnly)
            .buttonStyle(.borderless)
            .foregroundStyle(.red)
            .help("Delete this subscription")

            Button {
                selectedItemID = nil
            } label: {
                Label("Close", systemImage: "xmark.circle.fill")
            }
            .labelStyle(.iconOnly)
            .buttonStyle(.borderless)
            .foregroundStyle(.secondary)
        }
        .padding(.horizontal)
        .padding(.vertical, 10)
    }

    private func summaryCard(_ item: SubscriptionSummary) -> some View {
        GroupBox("Summary") {
            VStack(alignment: .leading, spacing: 6) {
                if let status = item.status {
                    LabeledContent("Status", value: status.label)
                }
                if let channel = item.channelType {
                    LabeledContent("Channel", value: channel)
                }
                if let endpoint = item.endpoint {
                    LabeledContent("Endpoint") {
                        Text(endpoint)
                            .textSelection(.enabled)
                            .lineLimit(2)
                            .truncationMode(.middle)
                    }
                }
                if let topic = item.topic {
                    LabeledContent("Topic / Criteria") {
                        Text(topic)
                            .textSelection(.enabled)
                            .lineLimit(2)
                            .truncationMode(.middle)
                    }
                }
                if let reason = item.reason {
                    LabeledContent("Reason", value: reason)
                }
            }
            .font(.callout)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func jsonCard(_ item: SubscriptionSummary) -> some View {
        GroupBox("Raw JSON") {
            Text(item.prettyJSON)
                .font(.system(.caption, design: .monospaced))
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    // MARK: - Actions

    private var selectedItem: SubscriptionSummary? {
        items.first { $0.id == selectedItemID }
    }

    /// Returns an error message to show in the editor, or `nil` on success.
    private func performCreate(_ data: Data) async -> String? {
        guard let operations = model.resourceOperations() else { return "Not connected to a server." }
        do {
            let created = try await operations.create(resourceType: "Subscription", json: data)
            await runSearch()
            selectedItemID = items.first { $0.fhirID == created.fhirID }?.id
            return nil
        } catch {
            return HFSAppModel.describe(error)
        }
    }

    private func performDelete(_ item: SubscriptionSummary) async {
        guard let operations = model.resourceOperations() else { return }
        do {
            try await operations.delete(resourceType: "Subscription", id: item.fhirID)
            selectedItemID = nil
            await runSearch()
        } catch {
            errorMessage = HFSAppModel.describe(error)
        }
    }

    private func runSearch() async {
        guard let operations = model.subscriptionOperations() else { return }
        hasSearched = true
        errorMessage = nil
        selectedItemID = nil
        isLoading = true
        defer { isLoading = false }

        do {
            let page = try await operations.search(
                filters: SubscriptionFilters(status: statusFilter?.rawValue ?? ""),
                count: pageSize
            )
            items = page.items
            total = page.total
            nextURL = page.nextURL
        } catch {
            items = []
            total = nil
            nextURL = nil
            errorMessage = HFSAppModel.describe(error)
        }
    }

    private func loadMore() async {
        guard
            let url = nextURL,
            let operations = model.subscriptionOperations(),
            !isLoadingMore
        else { return }

        isLoadingMore = true
        defer { isLoadingMore = false }

        do {
            let page = try await operations.searchPage(url: url)
            items.append(contentsOf: page.items)
            nextURL = page.nextURL
            if let pageTotal = page.total { total = pageTotal }
        } catch {
            errorMessage = HFSAppModel.describe(error)
        }
    }
}
