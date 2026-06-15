import Foundation
import HFSClient
import HFSCore
import HFSFHIR

/// How a Bundle entry was matched, from `entry.search.mode`. `match` is the
/// primary result set; `include` entries are pulled in by `_include`/`_revinclude`.
public enum SearchEntryMode: String, Sendable, Hashable {
    case match
    case include
    case outcome
    case other

    init(raw: String?) {
        switch raw {
        case "match": self = .match
        case "include": self = .include
        case "outcome": self = .outcome
        default: self = .other
        }
    }
}

/// One resource parsed from a search Bundle, ready for list display.
public struct ResourceListItem: Identifiable, Sendable, Hashable {
    public let id: UUID
    public let fhirID: String
    public let resourceType: String
    public let mode: SearchEntryMode
    public let prettyJSON: String

    public init(
        fhirID: String,
        resourceType: String,
        mode: SearchEntryMode = .match,
        prettyJSON: String
    ) {
        self.id = UUID()
        self.fhirID = fhirID
        self.resourceType = resourceType
        self.mode = mode
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

    /// Searches a resource type and returns the first page.
    ///
    /// Beyond plain parameters, supports `_include`/`_revinclude` (to pull in
    /// referenced/referencing resources, tagged `.include` in the results) and
    /// `_sort`.
    public func search(
        resourceType: String,
        parameters: [ResourceSearchParameter] = [],
        includes: [String] = [],
        revIncludes: [String] = [],
        sort: String = "",
        count: Int = 20
    ) async throws -> ResourcePage {
        var queryItems = [URLQueryItem(name: "_count", value: String(count))]
        for parameter in parameters {
            let name = parameter.name.trimmingCharacters(in: .whitespacesAndNewlines)
            let value = parameter.value.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !name.isEmpty, !value.isEmpty else { continue }
            queryItems.append(URLQueryItem(name: name, value: value))
        }
        for include in Self.cleaned(includes) {
            queryItems.append(URLQueryItem(name: "_include", value: include))
        }
        for revInclude in Self.cleaned(revIncludes) {
            queryItems.append(URLQueryItem(name: "_revinclude", value: revInclude))
        }
        let trimmedSort = sort.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmedSort.isEmpty {
            queryItems.append(URLQueryItem(name: "_sort", value: trimmedSort))
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

    /// Creates a new resource of the given type (POST /[type]).
    public func create(resourceType: String, json: Data) async throws -> ResourceListItem {
        let request = try await client.makeRequest(
            pathComponents: [resourceType],
            method: "POST",
            body: json
        )
        let data = try await client.send(request)
        return Self.parseResource(data, fallbackType: resourceType, requestBody: json)
    }

    /// Replaces an existing resource (PUT /[type]/[id]).
    public func update(resourceType: String, id: String, json: Data) async throws -> ResourceListItem {
        let request = try await client.makeRequest(
            pathComponents: [resourceType, id],
            method: "PUT",
            body: json
        )
        let data = try await client.send(request)
        return Self.parseResource(data, fallbackType: resourceType, requestBody: json)
    }

    /// Deletes a resource (DELETE /[type]/[id]).
    public func delete(resourceType: String, id: String) async throws {
        let request = try await client.makeRequest(
            pathComponents: [resourceType, id],
            method: "DELETE"
        )
        _ = try await client.send(request)
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
            let mode = SearchEntryMode(raw: (entry["search"] as? [String: Any])?["mode"] as? String)
            return makeItem(from: resource, fallbackType: fallbackType, mode: mode)
        }
    }

    private static func cleaned(_ values: [String]) -> [String] {
        values
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
    }

    /// Parses a single resource response (create/update), falling back to the
    /// request body when the server returns no body (e.g. a 204 on update).
    static func parseResource(
        _ data: Data,
        fallbackType: String,
        requestBody: Data? = nil
    ) -> ResourceListItem {
        if
            let resource = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
            let item = makeItem(from: resource, fallbackType: fallbackType)
        {
            return item
        }
        if
            let requestBody,
            let resource = (try? JSONSerialization.jsonObject(with: requestBody)) as? [String: Any],
            let item = makeItem(from: resource, fallbackType: fallbackType)
        {
            return item
        }
        return ResourceListItem(
            fhirID: "(unknown)",
            resourceType: fallbackType,
            prettyJSON: String(decoding: data, as: UTF8.self)
        )
    }

    private static func makeItem(
        from resource: [String: Any],
        fallbackType: String,
        mode: SearchEntryMode = .match
    ) -> ResourceListItem? {
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
            mode: mode,
            prettyJSON: String(decoding: json, as: UTF8.self)
        )
    }
}
