import HFSOperations
import SwiftUI

struct HFSResourcesView: View {
    @Environment(HFSAppModel.self) private var model

    @State private var selectedType: String?
    @State private var typeFilter = ""
    @State private var parameters: [ResourceSearchParameter] = []
    @State private var items: [ResourceListItem] = []
    @State private var total: Int?
    @State private var nextURL: URL?
    @State private var isLoading = false
    @State private var showSpinner = false
    @State private var isLoadingMore = false
    @State private var errorMessage: String?
    @State private var selectedItem: ResourceListItem?
    @State private var loadGeneration = 0

    private let pageSize = 20
    private let spinnerDelay = Duration.milliseconds(300)

    var body: some View {
        Group {
            if model.isConnected {
                browser
            } else {
                ContentUnavailableView {
                    Label("Not Connected", systemImage: "bolt.horizontal.circle")
                } description: {
                    Text("Connect to a server in Settings to browse resources.")
                }
            }
        }
        .inspector(isPresented: inspectorPresented) {
            jsonInspector
        }
    }

    private var browser: some View {
        HStack(spacing: 0) {
            typeColumn
                .frame(minWidth: 160, idealWidth: 220, maxWidth: 280)

            Divider()

            resultsColumn
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
        .searchable(text: $typeFilter, prompt: "Filter types")
        .task(id: selectedType) {
            parameters = []
            await runSearch()
        }
    }

    // MARK: - Resource type column

    private var typeColumn: some View {
        List(filteredTypes, id: \.self, selection: $selectedType) { type in
            Text(type)
        }
        .listStyle(.sidebar)
    }

    private var filteredTypes: [String] {
        guard !typeFilter.isEmpty else { return model.resourceTypes }
        return model.resourceTypes.filter { $0.localizedCaseInsensitiveContains(typeFilter) }
    }

    // MARK: - Results column

    @ViewBuilder
    private var resultsColumn: some View {
        if let type = selectedType {
            VStack(spacing: 0) {
                resultsHeader(type: type)
                Divider()
                parametersEditor
                Divider()
                resultsContent(type: type)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        } else {
            ContentUnavailableView {
                Label("Select a Resource Type", systemImage: "folder")
            } description: {
                Text("Choose a resource type to load a page of results.")
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }

    private func resultsHeader(type: String) -> some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text(type)
                    .font(.headline)
                Text(countSummary)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Spacer()

            Button {
                Task { await runSearch() }
            } label: {
                Label("Reload", systemImage: "arrow.clockwise")
            }
            .disabled(isLoading)
        }
        .padding(12)
    }

    private var countSummary: String {
        if isLoading {
            return "Loading…"
        }
        if let total {
            return "\(items.count) loaded of \(total)"
        }
        return "\(items.count) loaded"
    }

    private var parametersEditor: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach($parameters) { $parameter in
                HStack(spacing: 8) {
                    TextField("parameter", text: $parameter.name)
                        .textFieldStyle(.roundedBorder)
                        .frame(maxWidth: 170)
                    TextField("value", text: $parameter.value)
                        .textFieldStyle(.roundedBorder)
                    Button {
                        parameters.removeAll { $0.id == parameter.id }
                    } label: {
                        Image(systemName: "minus.circle.fill")
                    }
                    .buttonStyle(.borderless)
                    .foregroundStyle(.secondary)
                }
            }

            HStack {
                Button {
                    parameters.append(ResourceSearchParameter())
                } label: {
                    Label("Add Parameter", systemImage: "plus")
                }
                .buttonStyle(.borderless)

                Spacer()

                Button {
                    Task { await runSearch() }
                } label: {
                    Label("Search", systemImage: "magnifyingglass")
                }
                .buttonStyle(.borderedProminent)
                .disabled(isLoading)
            }
        }
        .padding(12)
    }

    @ViewBuilder
    private func resultsContent(type: String) -> some View {
        if showSpinner {
            ProgressView("Loading \(type)…")
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if let errorMessage {
            ContentUnavailableView {
                Label("Couldn’t Load", systemImage: "exclamationmark.triangle")
            } description: {
                Text(errorMessage)
            } actions: {
                Button("Retry") { Task { await runSearch() } }
            }
        } else if isLoading {
            // Loading, but not long enough to warrant a spinner yet — stay blank
            // so fast responses don't flash the spinner or the empty state.
            Color.clear
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if items.isEmpty {
            ContentUnavailableView {
                Label("No \(type) Resources", systemImage: "tray")
            } description: {
                Text("No resources match the current search.")
            }
        } else {
            VStack(spacing: 0) {
                List(selection: $selectedItem) {
                    ForEach(items) { item in
                        resourceRow(item).tag(item)
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

    private func resourceRow(_ item: ResourceListItem) -> some View {
        Label {
            VStack(alignment: .leading, spacing: 2) {
                Text(item.fhirID)
                    .font(.callout.weight(.medium))
                Text(item.resourceType)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        } icon: {
            Image(systemName: "doc.text")
        }
    }

    // MARK: - JSON inspector

    private var inspectorPresented: Binding<Bool> {
        Binding(
            get: { selectedItem != nil },
            set: { if !$0 { selectedItem = nil } }
        )
    }

    @ViewBuilder
    private var jsonInspector: some View {
        if let item = selectedItem {
            ScrollView {
                Text(item.prettyJSON)
                    .font(.system(.caption, design: .monospaced))
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding()
            }
            .navigationTitle("\(item.resourceType)/\(item.fhirID)")
            .inspectorColumnWidth(min: 280, ideal: 360, max: 460)
        } else {
            ContentUnavailableView(
                "No Selection",
                systemImage: "doc.text",
                description: Text("Select a resource to inspect its JSON.")
            )
        }
    }

    // MARK: - Loading

    private func runSearch() async {
        guard let type = selectedType, let operations = model.resourceOperations() else {
            isLoading = false
            showSpinner = false
            items = []
            total = nil
            nextURL = nil
            errorMessage = nil
            return
        }

        loadGeneration += 1
        let generation = loadGeneration

        errorMessage = nil
        selectedItem = nil
        items = []
        total = nil
        nextURL = nil
        isLoading = true
        showSpinner = false

        // Reveal the spinner only if this load is still the latest one after a
        // short delay, so fast responses (and fast failures) never flash it.
        Task { @MainActor in
            try? await Task.sleep(for: spinnerDelay)
            if generation == loadGeneration, isLoading {
                showSpinner = true
            }
        }

        do {
            let page = try await operations.search(
                resourceType: type,
                parameters: parameters,
                count: pageSize
            )
            guard generation == loadGeneration else { return }
            items = page.items
            total = page.total
            nextURL = page.nextURL
        } catch {
            guard generation == loadGeneration else { return }
            items = []
            total = nil
            nextURL = nil
            errorMessage = HFSAppModel.describe(error)
        }

        guard generation == loadGeneration else { return }
        isLoading = false
        showSpinner = false
    }

    private func loadMore() async {
        guard
            let url = nextURL,
            let type = selectedType,
            let operations = model.resourceOperations(),
            !isLoadingMore
        else { return }

        let generation = loadGeneration
        isLoadingMore = true
        defer { isLoadingMore = false }

        do {
            let page = try await operations.searchPage(url: url, fallbackType: type)
            guard generation == loadGeneration else { return }
            items.append(contentsOf: page.items)
            nextURL = page.nextURL
            if let pageTotal = page.total {
                total = pageTotal
            }
        } catch {
            guard generation == loadGeneration else { return }
            errorMessage = HFSAppModel.describe(error)
        }
    }
}
