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

    func testCreatePostsToTypePathAndParsesResponse() async throws {
        let transport = RecordingTransport(
            statusCode: 201,
            data: Data(#"{"resourceType":"Patient","id":"new-1","active":true}"#.utf8)
        )
        let operations = try makeOperations(transport: transport)

        let item = try await operations.create(
            resourceType: "Patient",
            json: Data(#"{"resourceType":"Patient"}"#.utf8)
        )

        XCTAssertEqual(transport.lastRequest?.httpMethod, "POST")
        XCTAssertEqual(transport.lastRequest?.url?.path, "/Patient")
        XCTAssertEqual(item.fhirID, "new-1")
        XCTAssertEqual(item.resourceType, "Patient")
    }

    func testUpdatePutsToInstancePath() async throws {
        let transport = RecordingTransport(
            statusCode: 200,
            data: Data(#"{"resourceType":"Patient","id":"p1"}"#.utf8)
        )
        let operations = try makeOperations(transport: transport)

        let item = try await operations.update(
            resourceType: "Patient",
            id: "p1",
            json: Data(#"{"resourceType":"Patient","id":"p1"}"#.utf8)
        )

        XCTAssertEqual(transport.lastRequest?.httpMethod, "PUT")
        XCTAssertEqual(transport.lastRequest?.url?.path, "/Patient/p1")
        XCTAssertEqual(item.fhirID, "p1")
    }

    func testUpdateFallsBackToRequestBodyOnEmptyResponse() async throws {
        let transport = RecordingTransport(statusCode: 200, data: Data())
        let operations = try makeOperations(transport: transport)

        let item = try await operations.update(
            resourceType: "Patient",
            id: "p9",
            json: Data(#"{"resourceType":"Patient","id":"p9"}"#.utf8)
        )

        XCTAssertEqual(item.fhirID, "p9")
        XCTAssertEqual(item.resourceType, "Patient")
    }

    func testDeleteSendsDeleteToInstancePath() async throws {
        let transport = RecordingTransport(statusCode: 204, data: Data())
        let operations = try makeOperations(transport: transport)

        try await operations.delete(resourceType: "Patient", id: "p1")

        XCTAssertEqual(transport.lastRequest?.httpMethod, "DELETE")
        XCTAssertEqual(transport.lastRequest?.url?.path, "/Patient/p1")
    }

    func testDeleteThrowsOnServerError() async throws {
        let operations = try makeOperations(transport: RecordingTransport(statusCode: 404, data: Data()))

        do {
            try await operations.delete(resourceType: "Patient", id: "missing")
            XCTFail("Expected delete to throw on a 404")
        } catch {
            // expected
        }
    }

    private func makeOperations(returning data: Data) throws -> HFSResourceOperations {
        try makeOperations(transport: StubTransport(statusCode: 200, data: data))
    }

    private func makeOperations(transport: HFSHTTPTransport) throws -> HFSResourceOperations {
        let client = HFSClient(
            configuration: HFSClientConfiguration(baseURL: try XCTUnwrap(URL(string: "http://localhost:8080"))),
            transport: transport
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

/// A transport that records the last request so tests can assert method/path.
private final class RecordingTransport: HFSHTTPTransport, @unchecked Sendable {
    let statusCode: Int
    let data: Data
    private(set) var lastRequest: URLRequest?

    init(statusCode: Int, data: Data) {
        self.statusCode = statusCode
        self.data = data
    }

    func send(_ request: URLRequest) async throws -> (Data, HTTPURLResponse) {
        lastRequest = request
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
