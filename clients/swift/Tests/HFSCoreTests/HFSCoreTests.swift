import XCTest
@testable import HFSCore

final class HFSCoreTests: XCTestCase {
    func testDefaultConfigurationUsesR4() throws {
        let config = HFSClientConfiguration(baseURL: try XCTUnwrap(URL(string: "http://localhost:8080")))

        XCTAssertEqual(config.defaultFHIRVersion, .r4)
        XCTAssertNil(config.tenant.identifier)
    }
}
