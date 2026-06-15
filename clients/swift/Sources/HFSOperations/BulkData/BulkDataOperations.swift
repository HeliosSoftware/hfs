import Foundation
import HFSClient
import HFSCore

/// The level a FHIR Bulk Data `$export` is kicked off at.
public enum BulkExportLevel: String, Sendable, CaseIterable, Identifiable, Hashable {
    case system
    case patient
    case group

    public var id: String { rawValue }

    /// Human-readable label for pickers and rows.
    public var label: String {
        switch self {
        case .system: "System"
        case .patient: "Patient"
        case .group: "Group"
        }
    }

    /// The FHIR operation path for this level. `group` needs a Group id appended.
    var pathComponents: [String] {
        switch self {
        case .system: ["$export"]
        case .patient: ["Patient", "$export"]
        case .group: ["Group"]
        }
    }
}

/// The result of a successful `$export` kick-off: the polling/status URL the
/// server returned in `Content-Location`.
public struct BulkExportKickoff: Sendable, Hashable {
    public let statusURL: URL

    public init(statusURL: URL) {
        self.statusURL = statusURL
    }
}

/// One output, error, or deleted file entry in a completed export manifest.
public struct BulkExportFile: Identifiable, Sendable, Hashable {
    public let id: UUID
    public let type: String
    public let url: URL
    public let count: Int?

    public init(type: String, url: URL, count: Int?) {
        self.id = UUID()
        self.type = type
        self.url = url
        self.count = count
    }
}

/// A parsed completed-export manifest (the `200` response body).
public struct BulkExportManifest: Sendable, Hashable {
    public let transactionTime: String?
    public let request: String?
    public let requiresAccessToken: Bool?
    public let output: [BulkExportFile]
    public let error: [BulkExportFile]
    public let deleted: [BulkExportFile]

    public init(
        transactionTime: String?,
        request: String?,
        requiresAccessToken: Bool?,
        output: [BulkExportFile],
        error: [BulkExportFile],
        deleted: [BulkExportFile]
    ) {
        self.transactionTime = transactionTime
        self.request = request
        self.requiresAccessToken = requiresAccessToken
        self.output = output
        self.error = error
        self.deleted = deleted
    }
}

/// The state of a polled export job: still running (`202`) or complete (`200`).
public enum BulkExportStatus: Sendable {
    /// `202 Accepted`; `progress` is the server's `X-Progress` header, if any.
    case inProgress(progress: String?)
    /// `200 OK` with the manifest body.
    case complete(BulkExportManifest)
}

/// FHIR Bulk Data Access `$export` operations: kick-off, poll, cancel.
///
/// Mirrors the HFS async export lifecycle — a kick-off returns a `202` with a
/// `Content-Location` status URL, which is polled until it returns `200` with a
/// manifest, and can be cancelled with `DELETE`.
public struct HFSBulkDataOperations: Sendable {
    private let client: HFSClient

    public init(client: HFSClient) {
        self.client = client
    }

    /// Kicks off an async export and returns the status URL to poll.
    ///
    /// - Parameters:
    ///   - level: system, patient, or group.
    ///   - groupID: required when `level == .group`, ignored otherwise.
    ///   - types: optional `_type` filter (resource type names).
    ///   - since: optional `_since` instant (FHIR `instant`).
    public func kickOff(
        level: BulkExportLevel,
        groupID: String? = nil,
        types: [String] = [],
        since: String? = nil
    ) async throws -> BulkExportKickoff {
        var path = level.pathComponents
        if level == .group {
            let trimmedID = (groupID ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
            guard !trimmedID.isEmpty else {
                throw HFSClientError.decoding("A group export requires a Group id.")
            }
            path.append(trimmedID)
            path.append("$export")
        }

        var queryItems: [URLQueryItem] = []
        let cleanedTypes = types
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
        if !cleanedTypes.isEmpty {
            queryItems.append(URLQueryItem(name: "_type", value: cleanedTypes.joined(separator: ",")))
        }
        if let since = since?.trimmingCharacters(in: .whitespacesAndNewlines), !since.isEmpty {
            queryItems.append(URLQueryItem(name: "_since", value: since))
        }

        var request = try await client.makeRequest(pathComponents: path, queryItems: queryItems)
        request.setValue("respond-async", forHTTPHeaderField: "Prefer")

        let (_, response) = try await client.sendReturningResponse(request)
        guard
            let location = response.value(forHTTPHeaderField: "Content-Location"),
            let statusURL = URL(string: location, relativeTo: request.url)?.absoluteURL
        else {
            throw HFSClientError.invalidResponse
        }
        return BulkExportKickoff(statusURL: statusURL)
    }

    /// Polls a job's status URL, returning in-progress or the completed manifest.
    public func status(url: URL) async throws -> BulkExportStatus {
        let request = try await client.makeRequest(url: url)
        let (data, response) = try await client.sendReturningResponse(request)

        if response.statusCode == 200 {
            return .complete(try Self.parseManifest(data))
        }
        // 202 (or any other 2xx): still processing.
        return .inProgress(progress: response.value(forHTTPHeaderField: "X-Progress"))
    }

    /// Cancels and deletes a job via its status URL (`DELETE`).
    public func cancel(url: URL) async throws {
        let request = try await client.makeRequest(url: url, method: "DELETE")
        _ = try await client.send(request)
    }

    static func parseManifest(_ data: Data) throws -> BulkExportManifest {
        guard let root = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw HFSClientError.decoding("Expected an export manifest object.")
        }

        return BulkExportManifest(
            transactionTime: root["transactionTime"] as? String,
            request: root["request"] as? String,
            requiresAccessToken: root["requiresAccessToken"] as? Bool,
            output: parseFiles(root["output"]),
            error: parseFiles(root["error"]),
            deleted: parseFiles(root["deleted"])
        )
    }

    private static func parseFiles(_ value: Any?) -> [BulkExportFile] {
        guard let entries = value as? [[String: Any]] else { return [] }
        return entries.compactMap { entry -> BulkExportFile? in
            guard
                let urlString = entry["url"] as? String,
                let url = URL(string: urlString)
            else { return nil }
            let type = entry["type"] as? String ?? "(unknown)"
            return BulkExportFile(type: type, url: url, count: entry["count"] as? Int)
        }
    }
}
