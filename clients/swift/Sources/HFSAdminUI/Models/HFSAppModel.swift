import Foundation
import HFSAuth
import HFSClient
import HFSCore
import HFSFHIR
import HFSHTTP
import HFSOperations
import Observation

/// Observable application state shared across the admin UI.
///
/// Owns the connection configuration, builds the ``HFSClient`` lazily on
/// connect, and tracks the live connection state so views can render real
/// status instead of placeholder text. Injected into the view tree with
/// `.environment(_:)` and read back with `@Environment(HFSAppModel.self)`.
@MainActor
@Observable
public final class HFSAppModel {
    public enum ConnectionState: Equatable, Sendable {
        case disconnected
        case connecting
        case connected
        case failed(String)
    }

    /// Facts derived from the server's `CapabilityStatement` for the Overview.
    public struct OverviewSummary: Equatable, Sendable {
        public var softwareName: String?
        public var softwareVersion: String?
        public var fhirVersion: String?
        public var resourceTypeCount: Int
        public var formats: [String]
        public var status: String?
        public var publisher: String?

        /// A human-readable software descriptor for captions.
        public var softwareLabel: String {
            switch (softwareName, softwareVersion) {
            case let (.some(name), .some(version)): "\(name) \(version)"
            case let (.some(name), nil): name
            case let (nil, .some(version)): "v\(version)"
            case (nil, nil): status?.capitalized ?? "Negotiated version"
            }
        }
    }

    /// Editable connection settings, bound directly to the Settings form.
    /// Mutations persist to `UserDefaults` so they survive relaunch.
    public var serverURLString: String { didSet { persistSettings() } }
    public var tenantIdentifier: String { didSet { persistSettings() } }
    public var fhirVersion: HFSFHIRVersion { didSet { persistSettings() } }

    /// When enabled, the app probes the saved server automatically on launch.
    public var autoConnect: Bool { didSet { persistSettings() } }

    /// Optional bearer token for secured servers, sent as `Authorization: Bearer`.
    ///
    /// Session-only by design: it is **never** written to `UserDefaults`, since a
    /// plist is not an appropriate store for a secret. A blank/whitespace value
    /// disables auth entirely (no header), so unsecured local servers are
    /// unaffected. A future step can promote this to Keychain alongside SMART.
    public var accessToken: String = ""

    /// Live connection state, updated by ``connect()`` / ``disconnect()``.
    public private(set) var connectionState: ConnectionState = .disconnected

    /// Parsed Overview facts, populated on connect and ``refreshOverview()``.
    public private(set) var overview: OverviewSummary?
    public private(set) var isLoadingOverview = false
    public private(set) var overviewError: String?

    /// Sorted resource types advertised by the server's CapabilityStatement.
    public private(set) var resourceTypes: [String] = []

    /// The connected client, available to feature screens once connected.
    public private(set) var client: HFSClient?

    /// Tracked Bulk Data export jobs, owned here so they survive navigating away
    /// from the Bulk Jobs screen and keep polling while running. Internal: the
    /// `BulkExportJob` type is internal to this UI module.
    private(set) var exportJobs: [BulkExportJob] = []
    private var exportPollTask: Task<Void, Never>?

    private let transport: HFSHTTPTransport
    private let defaults: UserDefaults
    private var canPersist = false

    private enum DefaultsKey {
        static let serverURL = "hfs.connection.serverURL"
        static let tenant = "hfs.connection.tenant"
        static let fhirVersion = "hfs.connection.fhirVersion"
        static let autoConnect = "hfs.connection.autoConnect"
    }

    public init(
        serverURLString: String = "http://localhost:8080",
        tenantIdentifier: String = "",
        fhirVersion: HFSFHIRVersion = .r4,
        autoConnect: Bool = true,
        accessToken: String = "",
        transport: HFSHTTPTransport = URLSessionHFSHTTPTransport(),
        defaults: UserDefaults = .standard
    ) {
        self.transport = transport
        self.defaults = defaults
        self.accessToken = accessToken

        // Restore persisted settings, falling back to the provided defaults.
        self.serverURLString = defaults.string(forKey: DefaultsKey.serverURL) ?? serverURLString
        self.tenantIdentifier = defaults.string(forKey: DefaultsKey.tenant) ?? tenantIdentifier
        self.fhirVersion = defaults.string(forKey: DefaultsKey.fhirVersion)
            .flatMap(HFSFHIRVersion.init(rawValue:)) ?? fhirVersion
        self.autoConnect = (defaults.object(forKey: DefaultsKey.autoConnect) as? Bool) ?? autoConnect

        // Enable persistence only after the initial load so restoring values
        // above does not write them straight back.
        canPersist = true
    }

    /// Writes the editable connection settings to `UserDefaults`.
    private func persistSettings() {
        guard canPersist else { return }
        defaults.set(serverURLString, forKey: DefaultsKey.serverURL)
        defaults.set(tenantIdentifier, forKey: DefaultsKey.tenant)
        defaults.set(fhirVersion.rawValue, forKey: DefaultsKey.fhirVersion)
        defaults.set(autoConnect, forKey: DefaultsKey.autoConnect)
    }

    public var isConnected: Bool {
        connectionState == .connected
    }

    public var isConnecting: Bool {
        connectionState == .connecting
    }

    /// Software name reported by the server, surfaced in the sidebar.
    public var serverName: String? {
        overview?.softwareName
    }

    /// Status text suitable for the sidebar and status strip.
    public var statusText: String {
        switch connectionState {
        case .disconnected: "Disconnected"
        case .connecting: "Connecting…"
        case .connected: "Connected"
        case .failed: "Connection failed"
        }
    }

    /// Tenant value for display, falling back to the server default.
    public var tenantDisplay: String {
        let trimmed = tenantIdentifier.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? "default" : trimmed
    }

    /// Whether a non-empty bearer token is configured for the next connection.
    public var hasAccessToken: Bool {
        !accessToken.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    /// Builds the access-token provider for the current settings.
    ///
    /// A blank or whitespace-only token yields ``NoAccessTokenProvider``, which
    /// sends no `Authorization` header at all — identical to the unauthenticated
    /// behavior, so unsecured local servers keep working unchanged.
    func makeTokenProvider() -> any HFSAccessTokenProvider {
        let trimmed = accessToken.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty
            ? NoAccessTokenProvider()
            : StaticAccessTokenProvider(token: trimmed)
    }

    /// Validates the configured URL, builds a client, and probes `/metadata`.
    public func connect() async {
        guard !isConnecting else { return }

        let trimmedURL = serverURLString.trimmingCharacters(in: .whitespacesAndNewlines)
        guard
            let url = URL(string: trimmedURL),
            let scheme = url.scheme?.lowercased(),
            scheme == "http" || scheme == "https",
            url.host != nil
        else {
            connectionState = .failed("Enter a valid http(s) server URL, e.g. http://localhost:8080.")
            return
        }

        connectionState = .connecting
        overviewError = nil

        let trimmedTenant = tenantIdentifier.trimmingCharacters(in: .whitespacesAndNewlines)
        let configuration = HFSClientConfiguration(
            baseURL: url,
            tenant: HFSTenantContext(identifier: trimmedTenant.isEmpty ? nil : trimmedTenant),
            defaultFHIRVersion: fhirVersion
        )
        let client = HFSClient(
            configuration: configuration,
            transport: transport,
            tokenProvider: makeTokenProvider()
        )

        do {
            let capability = try await client.capabilityStatement()
            self.client = client
            let parsed = Self.parseCapability(from: capability)
            overview = parsed.summary
            resourceTypes = parsed.resourceTypes
            connectionState = .connected
        } catch {
            self.client = nil
            overview = nil
            resourceTypes = []
            connectionState = .failed(Self.describe(error))
        }
    }

    /// Builds resource CRUD/search operations bound to the connected client.
    public func resourceOperations() -> HFSResourceOperations? {
        client.map(HFSResourceOperations.init)
    }

    /// Builds Bulk Data `$export` operations bound to the connected client.
    public func bulkDataOperations() -> HFSBulkDataOperations? {
        client.map(HFSBulkDataOperations.init)
    }

    /// Builds Subscription search operations bound to the connected client.
    public func subscriptionOperations() -> HFSSubscriptionOperations? {
        client.map(HFSSubscriptionOperations.init)
    }

    // MARK: - Bulk export jobs

    /// Kicks off an export, tracks it in ``exportJobs``, and starts polling.
    /// Returns an error message on failure, or `nil` on success (the new job is
    /// then `exportJobs.first`).
    func startExport(
        level: BulkExportLevel,
        groupID: String?,
        types: [String],
        since: String?
    ) async -> String? {
        guard let operations = bulkDataOperations() else { return "Not connected to a server." }
        do {
            let kickoff = try await operations.kickOff(
                level: level,
                groupID: groupID,
                types: types,
                since: since
            )
            let job = BulkExportJob(
                level: level,
                groupID: groupID,
                types: types,
                since: since,
                statusURL: kickoff.statusURL,
                startedAt: Date(),
                state: .running(progress: nil)
            )
            exportJobs.insert(job, at: 0)
            await pollExportJob(job.id)
            ensureExportPolling()
            return nil
        } catch {
            return Self.describe(error)
        }
    }

    /// Polls one running job's status URL and updates its state.
    func pollExportJob(_ id: BulkExportJob.ID) async {
        guard
            let operations = bulkDataOperations(),
            let job = exportJobs.first(where: { $0.id == id }),
            job.isRunning
        else { return }

        do {
            switch try await operations.status(url: job.statusURL) {
            case .inProgress(let progress):
                updateExportState(id, .running(progress: progress))
            case .complete(let manifest):
                updateExportState(id, .completed(manifest))
            }
        } catch {
            updateExportState(id, .failed(Self.describe(error)))
        }
    }

    /// Polls every running job once.
    func pollRunningExportJobs() async {
        for job in exportJobs where job.isRunning {
            await pollExportJob(job.id)
        }
    }

    /// Cancels and deletes a job via its status URL.
    func cancelExportJob(_ id: BulkExportJob.ID) async {
        guard
            let operations = bulkDataOperations(),
            let job = exportJobs.first(where: { $0.id == id })
        else { return }

        do {
            try await operations.cancel(url: job.statusURL)
            updateExportState(id, .cancelled)
        } catch {
            updateExportState(id, .failed(Self.describe(error)))
        }
    }

    /// Starts a background poller (if not already running) that refreshes running
    /// jobs every couple seconds, independent of which screen is visible.
    private func ensureExportPolling() {
        guard exportPollTask == nil else { return }
        exportPollTask = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(2))
                guard let self else { return }
                if self.exportJobs.contains(where: \.isRunning) {
                    await self.pollRunningExportJobs()
                }
            }
        }
    }

    private func updateExportState(_ id: BulkExportJob.ID, _ state: BulkExportJob.State) {
        guard let index = exportJobs.firstIndex(where: { $0.id == id }) else { return }
        exportJobs[index].state = state
    }

    /// Re-fetches `/metadata` for the connected server and refreshes the Overview.
    public func refreshOverview() async {
        guard let client else {
            overviewError = "Connect to a server first."
            return
        }

        isLoadingOverview = true
        overviewError = nil
        defer { isLoadingOverview = false }

        do {
            let capability = try await client.capabilityStatement()
            let parsed = Self.parseCapability(from: capability)
            overview = parsed.summary
            resourceTypes = parsed.resourceTypes
        } catch {
            overviewError = Self.describe(error)
        }
    }

    /// Clears the active connection without changing the edited settings.
    public func disconnect() {
        client = nil
        overview = nil
        overviewError = nil
        resourceTypes = []
        connectionState = .disconnected

        // Export jobs belong to a connection; stop polling and drop them.
        exportPollTask?.cancel()
        exportPollTask = nil
        exportJobs = []
    }

    private static func parseCapability(
        from resource: FHIRResource
    ) -> (summary: OverviewSummary, resourceTypes: [String]) {
        struct Capability: Decodable {
            struct Software: Decodable {
                let name: String?
                let version: String?
            }
            struct Rest: Decodable {
                struct Resource: Decodable { let type: String? }
                let resource: [Resource]?
            }
            let software: Software?
            let fhirVersion: String?
            let status: String?
            let publisher: String?
            let format: [String]?
            let rest: [Rest]?
        }

        let decoded = try? JSONDecoder().decode(Capability.self, from: resource.rawJSON)
        let types = (decoded?.rest?.first?.resource ?? [])
            .compactMap(\.type)
            .sorted()
        let summary = OverviewSummary(
            softwareName: decoded?.software?.name,
            softwareVersion: decoded?.software?.version,
            fhirVersion: decoded?.fhirVersion,
            resourceTypeCount: types.count,
            formats: decoded?.format ?? [],
            status: decoded?.status,
            publisher: decoded?.publisher
        )
        return (summary, types)
    }

    /// Maps client/network errors to a user-facing message. Shared with views.
    public static func describe(_ error: Error) -> String {
        switch error {
        case HFSClientError.httpStatus(let code):
            "Server returned HTTP \(code)."
        case HFSClientError.invalidBaseURL:
            "The server URL is invalid."
        case HFSClientError.invalidResponse:
            "The server returned an unexpected response."
        case let urlError as URLError:
            urlError.localizedDescription
        default:
            error.localizedDescription
        }
    }
}
