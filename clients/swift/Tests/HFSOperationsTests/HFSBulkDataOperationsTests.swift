import Foundation
import HFSClient
import HFSCore
import HFSHTTP
import XCTest

@testable import HFSOperations

final class HFSBulkDataOperationsTests: XCTestCase {
    // MARK: - Kick-off

    func testKickOffSystemSendsPreferHeaderAndReturnsStatusURL() async throws {
        let transport = BulkStubTransport(
            statusCode: 202,
            data: Data(),
            headers: ["Content-Location": "http://localhost:8080/export-status/job-1"]
        )
        let operations = try makeOperations(transport: transport)

        let kickoff = try await operations.kickOff(level: .system)

        XCTAssertEqual(transport.lastRequest?.httpMethod, "GET")
        XCTAssertEqual(transport.lastRequest?.url?.path, "/$export")
        XCTAssertEqual(transport.lastRequest?.value(forHTTPHeaderField: "Prefer"), "respond-async")
        XCTAssertEqual(kickoff.statusURL.absoluteString, "http://localhost:8080/export-status/job-1")
    }

    func testKickOffPatientUsesPatientExportPath() async throws {
        let transport = BulkStubTransport(
            statusCode: 202,
            data: Data(),
            headers: ["Content-Location": "http://localhost:8080/export-status/job-2"]
        )
        let operations = try makeOperations(transport: transport)

        _ = try await operations.kickOff(level: .patient)

        XCTAssertEqual(transport.lastRequest?.url?.path, "/Patient/$export")
    }

    func testKickOffGroupIncludesGroupIDInPath() async throws {
        let transport = BulkStubTransport(
            statusCode: 202,
            data: Data(),
            headers: ["Content-Location": "http://localhost:8080/export-status/job-3"]
        )
        let operations = try makeOperations(transport: transport)

        _ = try await operations.kickOff(level: .group, groupID: "g42")

        XCTAssertEqual(transport.lastRequest?.url?.path, "/Group/g42/$export")
    }

    func testKickOffGroupWithoutIDThrows() async throws {
        let operations = try makeOperations(
            transport: BulkStubTransport(statusCode: 202, data: Data(), headers: [:])
        )

        do {
            _ = try await operations.kickOff(level: .group, groupID: "   ")
            XCTFail("Expected a group export without an id to throw")
        } catch {
            // expected
        }
    }

    func testKickOffAddsTypeAndSinceQuery() async throws {
        let transport = BulkStubTransport(
            statusCode: 202,
            data: Data(),
            headers: ["Content-Location": "http://localhost:8080/export-status/job-4"]
        )
        let operations = try makeOperations(transport: transport)

        _ = try await operations.kickOff(
            level: .patient,
            types: ["Patient", " Observation "],
            since: "2024-01-01T00:00:00Z"
        )

        let components = try XCTUnwrap(
            transport.lastRequest?.url.flatMap { URLComponents(url: $0, resolvingAgainstBaseURL: false) }
        )
        let items = components.queryItems ?? []
        XCTAssertEqual(items.first { $0.name == "_type" }?.value, "Patient,Observation")
        XCTAssertEqual(items.first { $0.name == "_since" }?.value, "2024-01-01T00:00:00Z")
    }

    func testKickOffMissingContentLocationThrows() async throws {
        let operations = try makeOperations(
            transport: BulkStubTransport(statusCode: 202, data: Data(), headers: [:])
        )

        do {
            _ = try await operations.kickOff(level: .system)
            XCTFail("Expected a missing Content-Location to throw")
        } catch {
            // expected
        }
    }

    // MARK: - Status polling

    func testStatusInProgressParsesProgressHeader() async throws {
        let transport = BulkStubTransport(
            statusCode: 202,
            data: Data(),
            headers: ["X-Progress": "50% complete"]
        )
        let operations = try makeOperations(transport: transport)

        let status = try await operations.status(
            url: try XCTUnwrap(URL(string: "http://localhost:8080/export-status/job-1"))
        )

        guard case .inProgress(let progress) = status else {
            return XCTFail("Expected .inProgress, got \(status)")
        }
        XCTAssertEqual(progress, "50% complete")
    }

    func testStatusCompleteParsesManifest() async throws {
        let manifest = Data(
            """
            {"transactionTime":"2024-01-01T00:00:00Z",
             "request":"http://localhost:8080/Patient/$export",
             "requiresAccessToken":true,
             "output":[
               {"type":"Patient","url":"http://localhost:8080/export-file/job-1/Patient-1","count":12},
               {"type":"Observation","url":"http://localhost:8080/export-file/job-1/Observation-1"}
             ],
             "error":[{"type":"OperationOutcome","url":"http://localhost:8080/export-file/job-1/error-1"}]}
            """.utf8
        )
        let operations = try makeOperations(
            transport: BulkStubTransport(statusCode: 200, data: manifest, headers: [:])
        )

        let status = try await operations.status(
            url: try XCTUnwrap(URL(string: "http://localhost:8080/export-status/job-1"))
        )

        guard case .complete(let parsed) = status else {
            return XCTFail("Expected .complete, got \(status)")
        }
        XCTAssertEqual(parsed.transactionTime, "2024-01-01T00:00:00Z")
        XCTAssertEqual(parsed.requiresAccessToken, true)
        XCTAssertEqual(parsed.output.count, 2)
        XCTAssertEqual(parsed.output.first?.type, "Patient")
        XCTAssertEqual(parsed.output.first?.count, 12)
        XCTAssertNil(parsed.output.last?.count)
        XCTAssertEqual(parsed.error.count, 1)
        XCTAssertTrue(parsed.deleted.isEmpty)
    }

    // MARK: - Cancel

    func testCancelSendsDeleteToStatusURL() async throws {
        let transport = BulkStubTransport(statusCode: 202, data: Data(), headers: [:])
        let operations = try makeOperations(transport: transport)

        try await operations.cancel(
            url: try XCTUnwrap(URL(string: "http://localhost:8080/export-status/job-1"))
        )

        XCTAssertEqual(transport.lastRequest?.httpMethod, "DELETE")
        XCTAssertEqual(transport.lastRequest?.url?.path, "/export-status/job-1")
    }

    // MARK: - Download

    func testDownloadFileFetchesRawBytes() async throws {
        let ndjson = Data(#"{"resourceType":"Patient","id":"1"}\#n{"resourceType":"Patient","id":"2"}"#.utf8)
        let transport = BulkStubTransport(statusCode: 200, data: ndjson, headers: [:])
        let operations = try makeOperations(transport: transport)
        let url = try XCTUnwrap(URL(string: "http://localhost:8080/export-file/job-1/Patient-1"))

        let data = try await operations.downloadFile(url: url)

        XCTAssertEqual(data, ndjson)
        XCTAssertEqual(transport.lastRequest?.httpMethod, "GET")
        XCTAssertEqual(transport.lastRequest?.url, url)
    }

    // MARK: - Helpers

    private func makeOperations(transport: HFSHTTPTransport) throws -> HFSBulkDataOperations {
        let client = HFSClient(
            configuration: HFSClientConfiguration(baseURL: try XCTUnwrap(URL(string: "http://localhost:8080"))),
            transport: transport
        )
        return HFSBulkDataOperations(client: client)
    }
}

/// A transport that records the last request and returns a fixed status code,
/// body, and response headers (for `Content-Location` / `X-Progress`).
private final class BulkStubTransport: HFSHTTPTransport, @unchecked Sendable {
    let statusCode: Int
    let data: Data
    let headers: [String: String]
    private(set) var lastRequest: URLRequest?

    init(statusCode: Int, data: Data, headers: [String: String]) {
        self.statusCode = statusCode
        self.data = data
        self.headers = headers
    }

    func send(_ request: URLRequest) async throws -> (Data, HTTPURLResponse) {
        lastRequest = request
        let url = request.url ?? URL(string: "http://localhost")!
        let response = HTTPURLResponse(
            url: url,
            statusCode: statusCode,
            httpVersion: nil,
            headerFields: headers
        )!
        return (data, response)
    }
}
