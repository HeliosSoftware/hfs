import Foundation
import HFSClient
import HFSCore
import HFSFHIR

/// One resource parsed from a search Bundle, ready for list display.
public struct ResourceListItem: Identifiable, Sendable, Hashable {
    public let id: UUID
    public let fhirID: String
    public let resourceType: String
    public let prettyJSON: String

    public init(fhirID: String, resourceType: String, prettyJSON: String) {
        self.id = UUID()
        self.fhirID = fhirID
        self.resourceType = resourceType
        self.prettyJSON = prettyJSON
    }
}

/// A single FHIR search parameter (name + value), edited in the UI.
public struct ResourceSearchParameter: Identifiable, Sendable, Hashable {
    public let id: UUID
    public var name: String
    public var value: String

    public init(name: String = "", value: String = "") {
        self.id = UUID()
        self.name = name
        self.value = value
    }
}

/// One page of search results: the items plus paging metadata.
public struct ResourcePage: Sendable {
    public let items: [ResourceListItem]
    public let total: Int?
    public let nextURL: URL?

    public init(items: [ResourceListItem], total: Int?, nextURL: URL?) {
        self.items = items
        self.total = total
        self.nextURL = nextURL
    }
}

public struct HFSResourceOperations: Sendable {
    private let client: HFSClient

    public init(client: HFSClient) {
        self.client = client
    }

    public func read(resourceType: String, id: String) async throws -> FHIRResource {
        let request = try await client.makeRequest(pathComponents: [resourceType, id])
        let data = try await client.send(request)
        return FHIRResource(id: id, resourceType: resourceType, rawJSON: data)
    }

    /// Searches a resource type with optional parameters and returns the first page.
    public func search(
        resourceType: String,
        parameters: [ResourceSearchParameter] = [],
        count: Int = 20
    ) async throws -> ResourcePage {
        var queryItems = [URLQueryItem(name: "_count", value: String(count))]
        for parameter in parameters {
            let name = parameter.name.trimmingCharacters(in: .whitespacesAndNewlines)
            let value = parameter.value.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !name.isEmpty, !value.isEmpty else { continue }
            queryItems.append(URLQueryItem(name: name, value: value))
        }

        let request = try await client.makeRequest(pathComponents: [resourceType], queryItems: queryItems)
        let data = try await client.send(request)
        return try Self.parsePage(data, fallbackType: resourceType)
    }

    /// Follows a Bundle `next` link to fetch the next page of results.
    public func searchPage(url: URL, fallbackType: String) async throws -> ResourcePage {
        let request = try await client.makeRequest(url: url)
        let data = try await client.send(request)
        return try Self.parsePage(data, fallbackType: fallbackType)
    }

    static func parsePage(_ data: Data, fallbackType: String) throws -> ResourcePage {
        guard let root = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw HFSClientError.decoding("Expected a Bundle object.")
        }

        let items = parseEntries(root, fallbackType: fallbackType)
        let total = root["total"] as? Int
        let nextURL = (root["link"] as? [[String: Any]])?
            .first { ($0["relation"] as? String) == "next" }
            .flatMap { $0["url"] as? String }
            .flatMap(URL.init(string:))

        return ResourcePage(items: items, total: total, nextURL: nextURL)
    }

    private static func parseEntries(_ root: [String: Any], fallbackType: String) -> [ResourceListItem] {
        let entries = root["entry"] as? [[String: Any]] ?? []
        return entries.compactMap { entry -> ResourceListItem? in
            guard let resource = entry["resource"] as? [String: Any] else { return nil }
            let fhirID = resource["id"] as? String ?? "(no id)"
            let type = resource["resourceType"] as? String ?? fallbackType
            guard
                let json = try? JSONSerialization.data(
                    withJSONObject: resource,
                    options: [.prettyPrinted, .sortedKeys]
                )
            else { return nil }
            return ResourceListItem(
                fhirID: fhirID,
                resourceType: type,
                prettyJSON: String(decoding: json, as: UTF8.self)
            )
        }
    }
}
