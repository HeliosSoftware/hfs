import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif
import HFSAuth
import HFSCore
import HFSFHIR
import HFSHTTP

public actor HFSClient {
    public let configuration: HFSClientConfiguration
    private let transport: HFSHTTPTransport
    private let tokenProvider: HFSAccessTokenProvider

    public init(
        configuration: HFSClientConfiguration,
        transport: HFSHTTPTransport = URLSessionHFSHTTPTransport(),
        tokenProvider: HFSAccessTokenProvider = NoAccessTokenProvider()
    ) {
        self.configuration = configuration
        self.transport = transport
        self.tokenProvider = tokenProvider
    }

    public func capabilityStatement() async throws -> FHIRResource {
        let request = try await makeRequest(pathComponents: ["metadata"])
        let data = try await send(request)
        return FHIRResource(resourceType: "CapabilityStatement", rawJSON: data)
    }

    public func send(_ request: URLRequest) async throws -> Data {
        let (data, response) = try await transport.send(request)
        try validate(response)
        return data
    }

    public func makeRequest(
        pathComponents: [String],
        queryItems: [URLQueryItem] = [],
        method: String = "GET",
        body: Data? = nil
    ) async throws -> URLRequest {
        var components = URLComponents(
            url: tenantAwareBaseURL().appendingPathComponents(pathComponents),
            resolvingAgainstBaseURL: false
        )
        components?.queryItems = queryItems.isEmpty ? nil : queryItems

        guard let url = components?.url else {
            throw HFSClientError.invalidBaseURL
        }

        return try await makeRequest(url: url, method: method, body: body)
    }

    /// Builds a request for an absolute URL, e.g. a Bundle `next` page link.
    public func makeRequest(
        url: URL,
        method: String = "GET",
        body: Data? = nil
    ) async throws -> URLRequest {
        var request = URLRequest(url: url)
        request.httpMethod = method
        request.httpBody = body
        request.setValue("application/fhir+json", forHTTPHeaderField: "Accept")

        if body != nil {
            request.setValue("application/fhir+json", forHTTPHeaderField: "Content-Type")
        }

        if let token = try await tokenProvider.accessToken() {
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }

        return request
    }

    public func validate(_ response: HTTPURLResponse) throws {
        guard (200..<300).contains(response.statusCode) else {
            throw HFSClientError.httpStatus(response.statusCode)
        }
    }

    private func tenantAwareBaseURL() -> URL {
        guard let tenant = configuration.tenant.identifier, !tenant.isEmpty else {
            return configuration.baseURL
        }
        return configuration.baseURL.appendingPathComponent(tenant)
    }
}

private extension URL {
    func appendingPathComponents(_ components: [String]) -> URL {
        components.reduce(self) { url, component in
            url.appendingPathComponent(component)
        }
    }
}
