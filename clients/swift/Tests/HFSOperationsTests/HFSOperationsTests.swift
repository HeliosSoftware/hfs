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
        let operations = try makeOperations(returning: bundle)

        let page = try await operations.search(resourceType: "Patient", count: 20)

        XCTAssertEqual(page.items.count, 2)
        XCTAssertEqual(page.items.map(\.fhirID).sorted(), ["123", "456"])
        XCTAssertEqual(Set(page.items.map(\.resourceType)), ["Patient"])
        XCTAssertTrue(page.items[0].prettyJSON.contains("\"id\""))
        XCTAssertNil(page.nextURL)
    }

    func testSearchReturnsEmptyForBundleWithoutEntries() async throws {
        let bundle = Data(#"{"resourceType":"Bundle","type":"searchset"}"#.utf8)
        let operations = try makeOperations(returning: bundle)

        let page = try await operations.search(resourceType: "Observation")

        XCTAssertTrue(page.items.isEmpty)
        XCTAssertNil(page.total)
        XCTAssertNil(page.nextURL)
    }

    func testSearchParsesTotalAndNextLink() async throws {
        let bundle = Data(
            """
            {"resourceType":"Bundle","type":"searchset","total":42,
             "link":[
               {"relation":"self","url":"http://localhost:8080/Patient?_count=20"},
               {"relation":"next","url":"http://localhost:8080/Patient?_count=20&_offset=20"}
             ],
             "entry":[{"resource":{"resourceType":"Patient","id":"1"}}]}
            """.utf8
        )
        let operations = try makeOperations(returning: bundle)

        let page = try await operations.search(resourceType: "Patient")

        XCTAssertEqual(page.total, 42)
        XCTAssertEqual(page.items.count, 1)
        XCTAssertEqual(
            page.nextURL,
            URL(string: "http://localhost:8080/Patient?_count=20&_offset=20")
        )
    }

    func testSearchPageFollowsNextURL() async throws {
        let bundle = Data(
            #"{"resourceType":"Bundle","type":"searchset","entry":[{"resource":{"resourceType":"Patient","id":"21"}}]}"#.utf8
        )
        let operations = try makeOperations(returning: bundle)
        let next = try XCTUnwrap(URL(string: "http://localhost:8080/Patient?_count=20&_offset=20"))

        let page = try await operations.searchPage(url: next, fallbackType: "Patient")

        XCTAssertEqual(page.items.map(\.fhirID), ["21"])
    }

    private func makeOperations(returning data: Data) throws -> HFSResourceOperations {
        let client = HFSClient(
            configuration: HFSClientConfiguration(baseURL: try XCTUnwrap(URL(string: "http://localhost:8080"))),
            transport: StubTransport(statusCode: 200, data: data)
        )
        return HFSResourceOperations(client: client)
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
