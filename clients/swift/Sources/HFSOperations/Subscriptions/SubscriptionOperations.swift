import Foundation
import HFSClient
import HFSCore

/// FHIR `Subscription.status` codes.
public enum SubscriptionStatus: String, Sendable, CaseIterable, Identifiable, Hashable {
    case requested
    case active
    case error
    case off

    public var id: String { rawValue }

    public var label: String {
        switch self {
        case .requested: "Requested"
        case .active: "Active"
        case .error: "Error"
        case .off: "Off"
        }
    }
}

/// One `Subscription` parsed into a friendly summary for list/detail display.
///
/// Parsing is version-agnostic: it reads R5-style fields (`channelType`,
/// `endpoint`, `topic`) and falls back to R4-style fields (`channel.type`,
/// `channel.endpoint`, `criteria`).
public struct SubscriptionSummary: Identifiable, Sendable, Hashable {
    public let id: UUID
    public let fhirID: String
    public let statusCode: String?
    public let channelType: String?
    public let endpoint: String?
    public let topic: String?
    public let reason: String?
    public let prettyJSON: String

    public init(
        fhirID: String,
        statusCode: String?,
        channelType: String?,
        endpoint: String?,
        topic: String?,
        reason: String?,
        prettyJSON: String
    ) {
        self.id = UUID()
        self.fhirID = fhirID
        self.statusCode = statusCode
        self.channelType = channelType
        self.endpoint = endpoint
        self.topic = topic
        self.reason = reason
        self.prettyJSON = prettyJSON
    }

    public var status: SubscriptionStatus? { statusCode.flatMap(SubscriptionStatus.init(rawValue:)) }
}

/// Structured Subscription search filters, mapped to FHIR search parameters.
public struct SubscriptionFilters: Sendable, Hashable {
    public var status: String

    public init(status: String = "") {
        self.status = status
    }
}

/// One page of Subscriptions: the items plus paging metadata.
public struct SubscriptionPage: Sendable {
    public let items: [SubscriptionSummary]
    public let total: Int?
    public let nextURL: URL?

    public init(items: [SubscriptionSummary], total: Int?, nextURL: URL?) {
        self.items = items
        self.total = total
        self.nextURL = nextURL
    }
}

/// Searches FHIR `Subscription` resources with subscription-aware parsing.
///
/// Create/update/delete are intentionally left to the generic
/// `HFSResourceOperations`, since a `Subscription` is an ordinary FHIR resource.
public struct HFSSubscriptionOperations: Sendable {
    private let client: HFSClient

    public init(client: HFSClient) {
        self.client = client
    }

    /// Searches Subscriptions and returns the first page.
    public func search(filters: SubscriptionFilters = .init(), count: Int = 20) async throws -> SubscriptionPage {
        var queryItems = [URLQueryItem(name: "_count", value: String(count))]
        let status = filters.status.trimmingCharacters(in: .whitespacesAndNewlines)
        if !status.isEmpty {
            queryItems.append(URLQueryItem(name: "status", value: status))
        }

        let request = try await client.makeRequest(pathComponents: ["Subscription"], queryItems: queryItems)
        let data = try await client.send(request)
        return try Self.parsePage(data)
    }

    /// Follows a Bundle `next` link to fetch the next page.
    public func searchPage(url: URL) async throws -> SubscriptionPage {
        let request = try await client.makeRequest(url: url)
        let data = try await client.send(request)
        return try Self.parsePage(data)
    }

    static func parsePage(_ data: Data) throws -> SubscriptionPage {
        guard let root = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw HFSClientError.decoding("Expected a Bundle object.")
        }

        let entries = root["entry"] as? [[String: Any]] ?? []
        let items = entries.compactMap { entry -> SubscriptionSummary? in
            guard let resource = entry["resource"] as? [String: Any] else { return nil }
            return makeSummary(from: resource)
        }
        let total = root["total"] as? Int
        let nextURL = (root["link"] as? [[String: Any]])?
            .first { ($0["relation"] as? String) == "next" }
            .flatMap { $0["url"] as? String }
            .flatMap(URL.init(string:))

        return SubscriptionPage(items: items, total: total, nextURL: nextURL)
    }

    static func makeSummary(from resource: [String: Any]) -> SubscriptionSummary? {
        guard
            let json = try? JSONSerialization.data(
                withJSONObject: resource,
                options: [.prettyPrinted, .sortedKeys]
            )
        else { return nil }

        let channel = resource["channel"] as? [String: Any]
        // R5: channelType (Coding) + endpoint; R4: channel.type + channel.endpoint.
        let channelType = (resource["channelType"] as? [String: Any])?["code"] as? String
            ?? channel?["type"] as? String
        let endpoint = resource["endpoint"] as? String
            ?? channel?["endpoint"] as? String
        // R5: topic (canonical); R4: criteria (topic URL or query string).
        let topic = resource["topic"] as? String
            ?? resource["criteria"] as? String

        return SubscriptionSummary(
            fhirID: resource["id"] as? String ?? "(no id)",
            statusCode: resource["status"] as? String,
            channelType: channelType,
            endpoint: endpoint,
            topic: topic,
            reason: resource["reason"] as? String,
            prettyJSON: String(decoding: json, as: UTF8.self)
        )
    }
}
