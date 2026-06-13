import HFSClient

public struct HFSSubscriptionOperations: Sendable {
    private let client: HFSClient

    public init(client: HFSClient) {
        self.client = client
    }
}
