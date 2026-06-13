import Foundation

public struct FHIRResource: Codable, Identifiable, Sendable {
    public var id: String?
    public var resourceType: String
    public var rawJSON: Data

    public init(id: String? = nil, resourceType: String, rawJSON: Data) {
        self.id = id
        self.resourceType = resourceType
        self.rawJSON = rawJSON
    }
}

public struct FHIRBundle: Codable, Sendable {
    public var rawJSON: Data

    public init(rawJSON: Data) {
        self.rawJSON = rawJSON
    }
}

public struct FHIROperationOutcome: Codable, Sendable {
    public var rawJSON: Data

    public init(rawJSON: Data) {
        self.rawJSON = rawJSON
    }
}
