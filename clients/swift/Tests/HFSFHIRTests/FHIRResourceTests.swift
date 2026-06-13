import XCTest
@testable import HFSFHIR

final class FHIRResourceTests: XCTestCase {
    func testResourceStoresRawPayload() {
        let data = Data(#"{"resourceType":"Patient","id":"example"}"#.utf8)
        let resource = FHIRResource(id: "example", resourceType: "Patient", rawJSON: data)

        XCTAssertEqual(resource.id, "example")
        XCTAssertEqual(resource.resourceType, "Patient")
        XCTAssertEqual(resource.rawJSON, data)
    }
}
