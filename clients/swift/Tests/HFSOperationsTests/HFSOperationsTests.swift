import Foundation
import HFSClient
import HFSCore
import HFSHTTP
import XCTest

@testable import HFSOperations

final class HFSOperationsTests: XCTestCase {
    func testOperationsCanBeConstructed() throws {
        let config = HFSClientConfiguration(baseURL: try XCTUnwrap(URL(string: "http://localhost:8080")))
        let client = HFSClient(configuration: config)

        _ = HFSOperations(client: client)
    }

    func testSearchParsesBundleEntries() async throws {
        let bundle = Data(
            """
            {"resourceType":"Bundle","type":"searchset","entry":[
              {"resource":{"resourceType":"Patient","id":"123","active":true}},
              {"resource":{"resourceType":"Patient","id":"456"}}
            ]}
            """.utf8
        )
        let client = HFSClient(
            configuration: HFSClientConfiguration(baseURL: try XCTUnwrap(URL(string: "http://localhost:8080"))),
            transport: StubTransport(statusCode: 200, data: bundle)
        )
        let operations = HFSResourceOperations(client: client)

        let items = try await operations.search(resourceType: "Patient", count: 20)

        XCTAssertEqual(items.count, 2)
        XCTAssertEqual(items.map(\.fhirID).sorted(), ["123", "456"])
        XCTAssertEqual(Set(items.map(\.resourceType)), ["Patient"])
        XCTAssertTrue(items[0].prettyJSON.contains("\"id\""))
    }

    func testSearchReturnsEmptyForBundleWithoutEntries() async throws {
        let bundle = Data(#"{"resourceType":"Bundle","type":"searchset"}"#.utf8)
        let client = HFSClient(
            configuration: HFSClientConfiguration(baseURL: try XCTUnwrap(URL(string: "http://localhost:8080"))),
            transport: StubTransport(statusCode: 200, data: bundle)
        )
        let operations = HFSResourceOperations(client: client)

        let items = try await operations.search(resourceType: "Observation")

        XCTAssertTrue(items.isEmpty)
    }
}

private struct StubTransport: HFSHTTPTransport {
    let statusCode: Int
    let data: Data

    func send(_ request: URLRequest) async throws -> (Data, HTTPURLResponse) {
        let url = request.url ?? URL(string: "http://localhost")!
        let response = HTTPURLResponse(
            url: url,
            statusCode: statusCode,
            httpVersion: nil,
            headerFields: nil
        )!
        return (data, response)
    }
}
