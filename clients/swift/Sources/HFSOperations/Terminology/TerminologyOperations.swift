import HFSClient

public struct HFSTerminologyOperations: Sendable {
    private let client: HFSClient

    public init(client: HFSClient) {
        self.client = client
    }
}
