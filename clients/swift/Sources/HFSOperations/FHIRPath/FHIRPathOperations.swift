import HFSClient

public struct HFSFHIRPathOperations: Sendable {
    private let client: HFSClient

    public init(client: HFSClient) {
        self.client = client
    }
}
