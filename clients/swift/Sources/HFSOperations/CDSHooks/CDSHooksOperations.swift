import HFSClient

public struct HFSCDSHooksOperations: Sendable {
    private let client: HFSClient

    public init(client: HFSClient) {
        self.client = client
    }
}
