import HFSOperations
import SwiftUI

struct HFSResourcesView: View {
    @Environment(HFSAppModel.self) private var model

    @State private var selectedType: String?
    @State private var typeFilter = ""
    @State private var items: [ResourceListItem] = []
    @State private var isLoading = false
    @State private var errorMessage: String?
    @State private var selectedItem: ResourceListItem?

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
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .task(id: selectedType) {
            if let type = selectedType {
                await load(type: type)
            }
        }
    }

    // MARK: - Resource type column

    private var typeColumn: some View {
        VStack(spacing: 0) {
            HStack(spacing: 6) {
                Image(systemName: "magnifyingglass")
                    .foregroundStyle(.secondary)
                TextField("Filter types", text: $typeFilter)
                    .textFieldStyle(.plain)
            }
            .padding(8)

            Divider()

            List(filteredTypes, id: \.self, selection: $selectedType) { type in
                Text(type)
            }
            .listStyle(.sidebar)
        }
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
                resultsContent(type: type)
            }
        } else {
            ContentUnavailableView {
                Label("Select a Resource Type", systemImage: "folder")
            } description: {
                Text("Choose a resource type to load a page of results.")
            }
        }
    }

    private func resultsHeader(type: String) -> some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text(type)
                    .font(.headline)
                Text("\(items.count) loaded")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Spacer()

            Button {
                Task { await load(type: type) }
            } label: {
                Label("Reload", systemImage: "arrow.clockwise")
            }
            .disabled(isLoading)
        }
        .padding(12)
    }

    @ViewBuilder
    private func resultsContent(type: String) -> some View {
        if isLoading {
            ProgressView("Loading \(type)…")
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if let errorMessage {
            ContentUnavailableView {
                Label("Couldn’t Load", systemImage: "exclamationmark.triangle")
            } description: {
                Text(errorMessage)
            } actions: {
                Button("Retry") { Task { await load(type: type) } }
            }
        } else if items.isEmpty {
            ContentUnavailableView {
                Label("No \(type) Resources", systemImage: "tray")
            } description: {
                Text("The server returned no resources of this type.")
            }
        } else {
            List(selection: $selectedItem) {
                ForEach(items) { item in
                    resourceRow(item).tag(item)
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

    private func load(type: String) async {
        guard let operations = model.resourceOperations() else { return }
        isLoading = true
        errorMessage = nil
        selectedItem = nil
        defer { isLoading = false }

        do {
            items = try await operations.search(resourceType: type, count: 20)
        } catch {
            items = []
            errorMessage = HFSAppModel.describe(error)
        }
    }
}
