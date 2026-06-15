import HFSOperations
import SwiftUI

struct HFSResourcesView: View {
    @Environment(HFSAppModel.self) private var model

    @State private var selectedType: String?
    @State private var typeFilter = ""
    @State private var parameters: [ResourceSearchParameter] = []
    @State private var includesText = ""
    @State private var revIncludesText = ""
    @State private var sortText = ""
    @State private var items: [ResourceListItem] = []
    @State private var total: Int?
    @State private var nextURL: URL?
    @State private var isLoading = false
    @State private var showSpinner = false
    @State private var isLoadingMore = false
    @State private var errorMessage: String?
    @State private var selectedItem: ResourceListItem?
    @State private var loadGeneration = 0
    @State private var editorMode: EditorMode?
    @State private var deleteCandidate: ResourceListItem?

    /// Set by the toolbar "New Resource" action in the root view.
    @Binding var newResourceRequested: Bool

    private let pageSize = 20
    private let spinnerDelay = Duration.milliseconds(300)

    private enum EditorMode: Identifiable {
        case create(type: String)
        case edit(item: ResourceListItem)

        var id: String {
            switch self {
            case .create(let type): "create-\(type)"
            case .edit(let item): "edit-\(item.id)"
            }
        }
    }

    var body: some View {
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
            includesText = ""
            revIncludesText = ""
            sortText = ""
            await runSearch()
        }
        .onChange(of: newResourceRequested) { _, requested in
            guard requested else { return }
            editorMode = .create(type: selectedType ?? "Patient")
            newResourceRequested = false
        }
        .sheet(item: $editorMode) { mode in
            editorSheet(for: mode)
        }
        .confirmationDialog(
            deleteCandidate.map { "Delete \($0.resourceType)/\($0.fhirID)?" } ?? "Delete resource?",
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
            Text("This permanently deletes the resource on the server.")
        }
    }

    @ViewBuilder
    private func editorSheet(for mode: EditorMode) -> some View {
        switch mode {
        case .create(let type):
            HFSResourceEditorView(
                title: "New \(type)",
                saveButtonTitle: "Create",
                initialJSON: "{\n  \"resourceType\" : \"\(type)\"\n}"
            ) { data in
                await performCreate(data)
            }
        case .edit(let item):
            HFSResourceEditorView(
                title: "Edit \(item.resourceType)/\(item.fhirID)",
                saveButtonTitle: "Save",
                initialJSON: item.prettyJSON
            ) { data in
                await performUpdate(item, data)
            }
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
            HStack(spacing: 0) {
                resourceSearchForm(type: type)

                Divider()

                resultsContent(type: type)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)

                if let item = selectedItem {
                    Divider()

                    jsonColumn(item)
                }
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

    private var countSummary: String {
        if isLoading {
            return "Loading…"
        }
        let matches = items.filter { $0.mode != .include }.count
        let includes = items.filter { $0.mode == .include }.count
        var summary = total.map { "\(matches) of \($0)" } ?? "\(matches)"
        if includes > 0 { summary += " (+\(includes) included)" }
        return summary
    }

    private func splitCSV(_ text: String) -> [String] {
        text.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
    }

    private func resourceSearchForm(type: String) -> some View {
        Form {
            Section("Resource") {
                LabeledContent("Type", value: type)
                LabeledContent("Loaded", value: countSummary)

                Button {
                    Task { await runSearch() }
                } label: {
                    Label("Reload", systemImage: "arrow.clockwise")
                }
                .disabled(isLoading)
            }

            Section("Search Parameters") {
                if parameters.isEmpty {
                    Text("No parameters")
                        .foregroundStyle(.secondary)
                }

                ForEach($parameters) { $parameter in
                    HStack(spacing: 8) {
                        TextField("Parameter", text: $parameter.name)
                            .textFieldStyle(.roundedBorder)
                            .frame(maxWidth: 170)
                        TextField("Value", text: $parameter.value)
                            .textFieldStyle(.roundedBorder)
                        Button {
                            parameters.removeAll { $0.id == parameter.id }
                        } label: {
                            Label("Remove Parameter", systemImage: "minus.circle.fill")
                        }
                        .labelStyle(.iconOnly)
                        .buttonStyle(.borderless)
                        .foregroundStyle(.secondary)
                    }
                }

                ControlGroup {
                    Button {
                        parameters.append(ResourceSearchParameter())
                    } label: {
                        Label("Add Parameter", systemImage: "plus")
                    }

                    Button {
                        Task { await runSearch() }
                    } label: {
                        Label("Search", systemImage: "magnifyingglass")
                    }
                    .buttonStyle(.borderedProminent)
                    .disabled(isLoading)
                }
            }

            Section("Modifiers") {
                TextField("_include", text: $includesText, prompt: Text("Observation:patient"))
                    .textFieldStyle(.roundedBorder)
                    #if !os(macOS)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    #endif
                TextField("_revinclude", text: $revIncludesText, prompt: Text("Observation:patient"))
                    .textFieldStyle(.roundedBorder)
                    #if !os(macOS)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    #endif
                TextField("_sort", text: $sortText, prompt: Text("-_lastUpdated"))
                    .textFieldStyle(.roundedBorder)
                    #if !os(macOS)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    #endif
            }
        }
        .formStyle(.grouped)
        .frame(minWidth: 280, idealWidth: 320, maxWidth: 380)
        .frame(maxHeight: .infinity)
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
        HStack(spacing: 8) {
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

            Spacer()

            if item.mode == .include {
                HFSStatusBadge(title: "include", systemImage: "arrow.turn.down.right", tint: .secondary)
            }
        }
    }

    // MARK: - JSON detail column

    private func jsonColumn(_ item: ResourceListItem) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            inspectorHeader(item)

            Divider()

            ScrollView {
                Text(item.prettyJSON)
                    .font(.system(.caption, design: .monospaced))
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding()
            }
        }
        .frame(minWidth: 280, idealWidth: 360, maxWidth: 460)
        .frame(maxHeight: .infinity, alignment: .topLeading)
    }

    private func inspectorHeader(_ item: ResourceListItem) -> some View {
        HStack(alignment: .top) {
            VStack(alignment: .leading, spacing: 2) {
                Text(item.resourceType)
                    .font(.headline)
                Text(item.fhirID)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }

            Spacer()

            Button {
                editorMode = .edit(item: item)
            } label: {
                Label("Edit", systemImage: "pencil")
            }
            .labelStyle(.iconOnly)
            .buttonStyle(.borderless)
            .help("Edit this resource")

            Button(role: .destructive) {
                deleteCandidate = item
            } label: {
                Label("Delete", systemImage: "trash")
            }
            .labelStyle(.iconOnly)
            .buttonStyle(.borderless)
            .foregroundStyle(.red)
            .help("Delete this resource")

            Button {
                selectedItem = nil
            } label: {
                Label("Close", systemImage: "xmark.circle.fill")
            }
            .labelStyle(.iconOnly)
            .buttonStyle(.borderless)
            .foregroundStyle(.secondary)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal)
        .padding(.vertical, 10)
    }

    // MARK: - Create / update / delete

    /// Returns an error message to show in the editor, or `nil` on success.
    private func performCreate(_ data: Data) async -> String? {
        guard let operations = model.resourceOperations() else { return "Not connected to a server." }
        guard
            let object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
            let type = (object["resourceType"] as? String), !type.isEmpty
        else {
            return "JSON must include a non-empty \"resourceType\"."
        }

        do {
            let created = try await operations.create(resourceType: type, json: data)
            if type == selectedType {
                await runSearch()
                selectedItem = items.first { $0.fhirID == created.fhirID }
            } else {
                // Switch to the created resource's type; .task reloads the list.
                selectedType = type
            }
            return nil
        } catch {
            return HFSAppModel.describe(error)
        }
    }

    /// Returns an error message to show in the editor, or `nil` on success.
    private func performUpdate(_ item: ResourceListItem, _ data: Data) async -> String? {
        guard let operations = model.resourceOperations() else { return "Not connected to a server." }

        do {
            let updated = try await operations.update(
                resourceType: item.resourceType,
                id: item.fhirID,
                json: data
            )
            await runSearch()
            selectedItem = items.first { $0.fhirID == updated.fhirID }
            return nil
        } catch {
            return HFSAppModel.describe(error)
        }
    }

    private func performDelete(_ item: ResourceListItem) async {
        guard let operations = model.resourceOperations() else { return }

        do {
            try await operations.delete(resourceType: item.resourceType, id: item.fhirID)
            selectedItem = nil
            await runSearch()
        } catch {
            errorMessage = HFSAppModel.describe(error)
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
                includes: splitCSV(includesText),
                revIncludes: splitCSV(revIncludesText),
                sort: sortText,
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
