import Foundation
import HFSCore
import HFSHTTP
import XCTest

@testable import HFSAdminUI

@MainActor
final class HFSAppModelTests: XCTestCase {
    func testConnectSucceedsAndReadsServerName() async {
        let json = Data(
            """
            {"resourceType":"CapabilityStatement","software":{"name":"HFS Test Server"},"fhirVersion":"4.0.1"}
            """.utf8
        )
        let model = HFSAppModel(
            serverURLString: "http://localhost:8080",
            transport: StubTransport(statusCode: 200, data: json)
        )

        await model.connect()

        XCTAssertEqual(model.connectionState, .connected)
        XCTAssertTrue(model.isConnected)
        XCTAssertEqual(model.serverName, "HFS Test Server")
        XCTAssertNotNil(model.client)
    }

    func testConnectFailsOnServerError() async {
        let model = HFSAppModel(
            serverURLString: "http://localhost:8080",
            transport: StubTransport(statusCode: 503, data: Data())
        )

        await model.connect()

        guard case .failed = model.connectionState else {
            return XCTFail("Expected .failed, got \(model.connectionState)")
        }
        XCTAssertNil(model.client)
        XCTAssertFalse(model.isConnected)
    }

    func testConnectRejectsURLWithoutScheme() async {
        let model = HFSAppModel(
            serverURLString: "localhost:8080",
            transport: StubTransport(statusCode: 200, data: Data())
        )

        await model.connect()

        guard case .failed = model.connectionState else {
            return XCTFail("Expected .failed for a URL without an http(s) scheme")
        }
    }

    func testDisconnectClearsConnectionState() async {
        let json = Data(#"{"resourceType":"CapabilityStatement"}"#.utf8)
        let model = HFSAppModel(
            serverURLString: "http://localhost:8080",
            transport: StubTransport(statusCode: 200, data: json)
        )

        await model.connect()
        XCTAssertTrue(model.isConnected)

        model.disconnect()

        XCTAssertEqual(model.connectionState, .disconnected)
        XCTAssertNil(model.client)
        XCTAssertNil(model.serverName)
    }

    func testConnectPopulatesOverviewSummary() async {
        let json = Data(
            """
            {"resourceType":"CapabilityStatement","status":"active",
             "software":{"name":"HFS","version":"0.1.47"},"fhirVersion":"4.0.1",
             "format":["json","xml"],
             "rest":[{"mode":"server","resource":[{"type":"Patient"},{"type":"Observation"},{"type":"Encounter"}]}]}
            """.utf8
        )
        let model = HFSAppModel(
            serverURLString: "http://localhost:8080",
            transport: StubTransport(statusCode: 200, data: json)
        )

        await model.connect()

        XCTAssertEqual(model.connectionState, .connected)
        XCTAssertEqual(model.overview?.resourceTypeCount, 3)
        XCTAssertEqual(model.overview?.fhirVersion, "4.0.1")
        XCTAssertEqual(model.overview?.softwareName, "HFS")
        XCTAssertEqual(model.overview?.softwareLabel, "HFS 0.1.47")
        XCTAssertEqual(model.overview?.formats, ["json", "xml"])
    }

    func testRefreshOverviewWithoutClientSetsError() async {
        let model = HFSAppModel(transport: StubTransport(statusCode: 200, data: Data()))

        await model.refreshOverview()

        XCTAssertNotNil(model.overviewError)
        XCTAssertNil(model.overview)
    }

    func testRefreshOverviewReloadsAfterConnect() async {
        let json = Data(
            #"{"resourceType":"CapabilityStatement","rest":[{"resource":[{"type":"Patient"}]}]}"#.utf8
        )
        let model = HFSAppModel(
            serverURLString: "http://localhost:8080",
            transport: StubTransport(statusCode: 200, data: json)
        )

        await model.connect()
        XCTAssertEqual(model.overview?.resourceTypeCount, 1)

        await model.refreshOverview()
        XCTAssertEqual(model.overview?.resourceTypeCount, 1)
        XCTAssertNil(model.overviewError)
        XCTAssertFalse(model.isLoadingOverview)
    }

    func testDisconnectClearsOverview() async {
        let json = Data(#"{"resourceType":"CapabilityStatement","rest":[{"resource":[{"type":"Patient"}]}]}"#.utf8)
        let model = HFSAppModel(
            serverURLString: "http://localhost:8080",
            transport: StubTransport(statusCode: 200, data: json)
        )

        await model.connect()
        XCTAssertNotNil(model.overview)

        model.disconnect()
        XCTAssertNil(model.overview)
    }

    func testTenantDisplayFallsBackToDefault() {
        let model = HFSAppModel(tenantIdentifier: "  ")
        XCTAssertEqual(model.tenantDisplay, "default")

        model.tenantIdentifier = "acme"
        XCTAssertEqual(model.tenantDisplay, "acme")
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
