import Foundation
import HFSCore

public protocol HFSAccessTokenProvider: Sendable {
    func accessToken() async throws -> String?
}

public struct NoAccessTokenProvider: HFSAccessTokenProvider {
    public init() {}

    public func accessToken() async throws -> String? {
        nil
    }
}

public struct StaticAccessTokenProvider: HFSAccessTokenProvider {
    private let token: String

    public init(token: String) {
        self.token = token
    }

    public func accessToken() async throws -> String? {
        token
    }
}
