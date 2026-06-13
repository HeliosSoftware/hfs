import HFSClient

public struct HFSBulkDataOperations: Sendable {
    private let client: HFSClient

    public init(client: HFSClient) {
        self.client = client
    }
}
