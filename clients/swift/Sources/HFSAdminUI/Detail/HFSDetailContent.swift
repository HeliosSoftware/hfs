import SwiftUI

struct HFSDetailContent: View {
    @Environment(HFSAppModel.self) private var model
    var destination: HFSAdminDestination

    var body: some View {
        switch destination {
        case .settings:
            HFSSettingsView()
        case .resources:
            HFSResourcesView()
        default:
            scaffold
        }
    }

    private var scaffold: some View {
        ScrollView {
            HStack(alignment: .top, spacing: 0) {
                VStack(alignment: .leading, spacing: 20) {
                    HFSDetailSummary(destination: destination)
                    HFSStatusStrip()

                    if destination == .overview {
                        overviewBanner
                    }

                    HFSMetricGrid(tiles: tiles)

                    HFSWorkspacePanel(destination: destination)
                }
                .frame(maxWidth: 1180, alignment: .topLeading)
                .padding(24)

                Spacer(minLength: 0)
            }
            .frame(maxWidth: .infinity, alignment: .topLeading)
        }
        .task(id: "\(destination.rawValue)|\(model.isConnected)") {
            await loadOverviewIfNeeded()
        }
    }

    private var tiles: [HFSPlaceholderTileModel] {
        guard destination == .overview, let overview = model.overview else {
            return destination.tiles
        }
        return [
            HFSPlaceholderTileModel(
                title: "Capability",
                value: "\(overview.resourceTypeCount)",
                caption: "REST resource types",
                systemImage: "square.stack.3d.up"
            ),
            HFSPlaceholderTileModel(
                title: "FHIR Version",
                value: overview.fhirVersion ?? "—",
                caption: overview.softwareLabel,
                systemImage: "cross.case"
            ),
            HFSPlaceholderTileModel(
                title: "Tenants",
                value: "1",
                caption: model.tenantDisplay,
                systemImage: "building.2"
            )
        ]
    }

    @ViewBuilder
    private var overviewBanner: some View {
        if model.isLoadingOverview, model.overview == nil {
            GroupBox {
                HStack(spacing: 10) {
                    ProgressView()
                        .controlSize(.small)
                    Text("Loading server capability…")
                        .foregroundStyle(.secondary)
                    Spacer(minLength: 0)
                }
            }
        } else if let error = model.overviewError, model.overview == nil {
            GroupBox {
                Label(error, systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(.red)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
    }

    private func loadOverviewIfNeeded() async {
        guard
            destination == .overview,
            model.isConnected,
            model.overview == nil,
            !model.isLoadingOverview
        else { return }
        await model.refreshOverview()
    }
}

private struct HFSDetailSummary: View {
    var destination: HFSAdminDestination

    var body: some View {
        HStack(alignment: .center, spacing: 14) {
            Image(systemName: destination.systemImage)
                .font(.system(size: 28, weight: .semibold))
                .symbolRenderingMode(.hierarchical)
                .foregroundStyle(.secondary)
                .frame(width: 44, height: 44)

            VStack(alignment: .leading, spacing: 4) {
                Text(destination.title)
                    .font(.title2.weight(.semibold))

                Text(destination.summary)
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }

            Spacer()
        }
    }
}

private struct HFSMetricGrid: View {
    var tiles: [HFSPlaceholderTileModel]

    var body: some View {
        LazyVGrid(columns: gridColumns, alignment: .leading, spacing: 16) {
            ForEach(tiles) { tile in
                HFSPlaceholderTile(tile: tile)
            }
        }
    }

    private var gridColumns: [GridItem] {
        [
            GridItem(.adaptive(minimum: 240, maximum: 320), spacing: 16, alignment: .top)
        ]
    }
}
