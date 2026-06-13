import Foundation
#if canImport(FoundationNetworking)
import FoundationNetworking
#endif
import HFSCore

public protocol HFSHTTPTransport: Sendable {
    func send(_ request: URLRequest) async throws -> (Data, HTTPURLResponse)
}

public struct URLSessionHFSHTTPTransport: HFSHTTPTransport {
    private let session: URLSession

    public init(session: URLSession = .shared) {
        self.session = session
    }

    public func send(_ request: URLRequest) async throws -> (Data, HTTPURLResponse) {
        let (data, response) = try await session.data(for: request)
        guard let httpResponse = response as? HTTPURLResponse else {
            throw HFSClientError.invalidResponse
        }
        return (data, httpResponse)
    }
}
