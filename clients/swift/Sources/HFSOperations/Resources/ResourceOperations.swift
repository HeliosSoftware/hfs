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

    /// Searches a resource type and returns a page of parsed list items.
    public func search(resourceType: String, count: Int = 20) async throws -> [ResourceListItem] {
        let request = try await client.makeRequest(
            pathComponents: [resourceType],
            queryItems: [URLQueryItem(name: "_count", value: String(count))]
        )
        let data = try await client.send(request)
        return try Self.parseBundle(data, fallbackType: resourceType)
    }

    static func parseBundle(_ data: Data, fallbackType: String) throws -> [ResourceListItem] {
        guard let root = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw HFSClientError.decoding("Expected a Bundle object.")
        }

        let entries = root["entry"] as? [[String: Any]] ?? []
        return try entries.compactMap { entry -> ResourceListItem? in
            guard let resource = entry["resource"] as? [String: Any] else { return nil }
            let fhirID = resource["id"] as? String ?? "(no id)"
            let type = resource["resourceType"] as? String ?? fallbackType
            let json = try JSONSerialization.data(
                withJSONObject: resource,
                options: [.prettyPrinted, .sortedKeys]
            )
            return ResourceListItem(
                fhirID: fhirID,
                resourceType: type,
                prettyJSON: String(decoding: json, as: UTF8.self)
            )
        }
    }
}
