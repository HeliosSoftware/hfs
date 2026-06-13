import Foundation

public enum HFSFHIRVersion: String, Codable, Sendable, CaseIterable {
    case r4 = "R4"
    case r4b = "R4B"
    case r5 = "R5"
    case r6 = "R6"
}

public struct HFSTenantContext: Codable, Hashable, Sendable {
    public var identifier: String?

    public init(identifier: String? = nil) {
        self.identifier = identifier
    }
}

public struct HFSClientConfiguration: Sendable {
    public var baseURL: URL
    public var tenant: HFSTenantContext
    public var defaultFHIRVersion: HFSFHIRVersion

    public init(
        baseURL: URL,
        tenant: HFSTenantContext = HFSTenantContext(),
        defaultFHIRVersion: HFSFHIRVersion = .r4
    ) {
        self.baseURL = baseURL
        self.tenant = tenant
        self.defaultFHIRVersion = defaultFHIRVersion
    }
}

public enum HFSClientError: Error, Equatable, Sendable {
    case invalidBaseURL
    case invalidResponse
    case httpStatus(Int)
    case decoding(String)
    case missingResourceType
}
