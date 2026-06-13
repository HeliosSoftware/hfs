import HFSClient

public struct HFSOperations: Sendable {
    public let resources: HFSResourceOperations
    public let bulkData: HFSBulkDataOperations
    public let audit: HFSAuditOperations
    public let subscriptions: HFSSubscriptionOperations
    public let terminology: HFSTerminologyOperations
    public let fhirPath: HFSFHIRPathOperations
    public let sqlOnFHIR: HFSSQLOnFHIROperations
    public let cdsHooks: HFSCDSHooksOperations

    public init(client: HFSClient) {
        self.resources = HFSResourceOperations(client: client)
        self.bulkData = HFSBulkDataOperations(client: client)
        self.audit = HFSAuditOperations(client: client)
        self.subscriptions = HFSSubscriptionOperations(client: client)
        self.terminology = HFSTerminologyOperations(client: client)
        self.fhirPath = HFSFHIRPathOperations(client: client)
        self.sqlOnFHIR = HFSSQLOnFHIROperations(client: client)
        self.cdsHooks = HFSCDSHooksOperations(client: client)
    }
}
