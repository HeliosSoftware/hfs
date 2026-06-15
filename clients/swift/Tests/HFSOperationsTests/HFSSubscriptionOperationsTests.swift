import Foundation
import HFSClient
import HFSCore
import HFSHTTP
import XCTest

@testable import HFSOperations

final class HFSSubscriptionOperationsTests: XCTestCase {
    func testSearchTargetsSubscriptionWithCount() async throws {
        let transport = RecordingTransport(statusCode: 200, data: emptyBundle)
        let operations = try makeOperations(transport: transport)

        _ = try await operations.search()

        XCTAssertEqual(transport.lastRequest?.url?.path, "/Subscription")
        XCTAssertEqual(queryItems(transport.lastRequest).first { $0.name == "_count" }?.value, "20")
    }

    func testSearchMapsStatusFilter() async throws {
        let transport = RecordingTransport(statusCode: 200, data: emptyBundle)
        let operations = try makeOperations(transport: transport)

        _ = try await operations.search(filters: SubscriptionFilters(status: "active"))

        XCTAssertEqual(queryItems(transport.lastRequest).first { $0.name == "status" }?.value, "active")
    }

    func testSearchOmitsEmptyStatus() async throws {
        let transport = RecordingTransport(statusCode: 200, data: emptyBundle)
        let operations = try makeOperations(transport: transport)

        _ = try await operations.search(filters: SubscriptionFilters(status: "  "))

        XCTAssertFalse(queryItems(transport.lastRequest).contains { $0.name == "status" })
    }

    func testSearchParsesR4ChannelFields() async throws {
        let bundle = Data(
            """
            {"resourceType":"Bundle","type":"searchset","total":1,
             "entry":[{"resource":{
               "resourceType":"Subscription","id":"s1","status":"active",
               "reason":"watch patients","criteria":"Patient?active=true",
               "channel":{"type":"rest-hook","endpoint":"https://example.com/hook","payload":"application/fhir+json"}
             }}]}
            """.utf8
        )
        let operations = try makeOperations(transport: RecordingTransport(statusCode: 200, data: bundle))

        let page = try await operations.search()

        XCTAssertEqual(page.total, 1)
        let item = try XCTUnwrap(page.items.first)
        XCTAssertEqual(item.fhirID, "s1")
        XCTAssertEqual(item.status, .active)
        XCTAssertEqual(item.channelType, "rest-hook")
        XCTAssertEqual(item.endpoint, "https://example.com/hook")
        XCTAssertEqual(item.topic, "Patient?active=true")
        XCTAssertEqual(item.reason, "watch patients")
    }

    func testSearchParsesR5ChannelFields() async throws {
        let bundle = Data(
            """
            {"resourceType":"Bundle","type":"searchset",
             "entry":[{"resource":{
               "resourceType":"Subscription","id":"s2","status":"requested",
               "topic":"http://example.org/topic/patient",
               "channelType":{"code":"websocket"},
               "endpoint":"wss://example.com/ws"
             }}]}
            """.utf8
        )
        let operations = try makeOperations(transport: RecordingTransport(statusCode: 200, data: bundle))

        let page = try await operations.search()
        let item = try XCTUnwrap(page.items.first)
        XCTAssertEqual(item.status, .requested)
        XCTAssertEqual(item.channelType, "websocket")
        XCTAssertEqual(item.endpoint, "wss://example.com/ws")
        XCTAssertEqual(item.topic, "http://example.org/topic/patient")
    }

    func testSearchParsesNextLink() async throws {
        let bundle = Data(
            """
            {"resourceType":"Bundle","type":"searchset",
             "link":[{"relation":"next","url":"http://localhost:8080/Subscription?_count=20&_offset=20"}],
             "entry":[{"resource":{"resourceType":"Subscription","id":"s1","status":"off"}}]}
            """.utf8
        )
        let operations = try makeOperations(transport: RecordingTransport(statusCode: 200, data: bundle))

        let page = try await operations.search()

        XCTAssertEqual(page.nextURL?.absoluteString, "http://localhost:8080/Subscription?_count=20&_offset=20")
        XCTAssertEqual(page.items.first?.status, .off)
    }

    func testSearchPageFollowsNextURL() async throws {
        let transport = RecordingTransport(statusCode: 200, data: emptyBundle)
        let operations = try makeOperations(transport: transport)
        let url = try XCTUnwrap(URL(string: "http://localhost:8080/Subscription?_count=20&_offset=20"))

        _ = try await operations.searchPage(url: url)

        XCTAssertEqual(transport.lastRequest?.url, url)
    }

    // MARK: - Helpers

    private let emptyBundle = Data(#"{"resourceType":"Bundle","type":"searchset"}"#.utf8)

    private func queryItems(_ request: URLRequest?) -> [URLQueryItem] {
        guard
            let url = request?.url,
            let components = URLComponents(url: url, resolvingAgainstBaseURL: false)
        else { return [] }
        return components.queryItems ?? []
    }

    private func makeOperations(transport: HFSHTTPTransport) throws -> HFSSubscriptionOperations {
        let client = HFSClient(
            configuration: HFSClientConfiguration(baseURL: try XCTUnwrap(URL(string: "http://localhost:8080"))),
            transport: transport
        )
        return HFSSubscriptionOperations(client: client)
    }
}

/// A transport that records the last request so tests can assert path/query.
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
