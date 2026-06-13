import HFSClient

public struct HFSSQLOnFHIROperations: Sendable {
    private let client: HFSClient

    public init(client: HFSClient) {
        self.client = client
    }
}
