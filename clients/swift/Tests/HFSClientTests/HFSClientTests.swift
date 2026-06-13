import XCTest
@testable import HFSClient
import HFSCore

final class HFSClientTests: XCTestCase {
    func testTenantAwareRequestPath() async throws {
        let config = HFSClientConfiguration(
            baseURL: try XCTUnwrap(URL(string: "http://localhost:8080")),
            tenant: HFSTenantContext(identifier: "acme")
        )
        let client = HFSClient(configuration: config)

        let request = try await client.makeRequest(pathComponents: ["metadata"])

        XCTAssertEqual(request.url?.absoluteString, "http://localhost:8080/acme/metadata")
    }
}
