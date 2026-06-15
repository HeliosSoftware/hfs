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
            transport: StubTransport(statusCode: 200, data: json),
            defaults: Self.ephemeralDefaults()
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
            transport: StubTransport(statusCode: 503, data: Data()),
            defaults: Self.ephemeralDefaults()
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
            transport: StubTransport(statusCode: 200, data: Data()),
            defaults: Self.ephemeralDefaults()
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
            transport: StubTransport(statusCode: 200, data: json),
            defaults: Self.ephemeralDefaults()
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
            transport: StubTransport(statusCode: 200, data: json),
            defaults: Self.ephemeralDefaults()
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
        let model = HFSAppModel(
            transport: StubTransport(statusCode: 200, data: Data()),
            defaults: Self.ephemeralDefaults()
        )

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
            transport: StubTransport(statusCode: 200, data: json),
            defaults: Self.ephemeralDefaults()
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
            transport: StubTransport(statusCode: 200, data: json),
            defaults: Self.ephemeralDefaults()
        )

        await model.connect()
        XCTAssertNotNil(model.overview)

        model.disconnect()
        XCTAssertNil(model.overview)
    }

    func testTenantDisplayFallsBackToDefault() {
        let model = HFSAppModel(tenantIdentifier: "  ", defaults: Self.ephemeralDefaults())
        XCTAssertEqual(model.tenantDisplay, "default")

        model.tenantIdentifier = "acme"
        XCTAssertEqual(model.tenantDisplay, "acme")
    }

    // MARK: - Settings persistence

    func testSettingsPersistAcrossInstances() {
        let defaults = Self.ephemeralDefaults()

        let first = HFSAppModel(defaults: defaults)
        first.serverURLString = "https://fhir.example.com"
        first.tenantIdentifier = "acme"
        first.fhirVersion = .r5
        first.autoConnect = false

        // A fresh model over the same defaults restores the saved values.
        let second = HFSAppModel(defaults: defaults)
        XCTAssertEqual(second.serverURLString, "https://fhir.example.com")
        XCTAssertEqual(second.tenantIdentifier, "acme")
        XCTAssertEqual(second.fhirVersion, .r5)
        XCTAssertFalse(second.autoConnect)
    }

    func testInitDefaultsWhenNothingPersisted() {
        let model = HFSAppModel(defaults: Self.ephemeralDefaults())
        XCTAssertEqual(model.serverURLString, "http://localhost:8080")
        XCTAssertEqual(model.tenantIdentifier, "")
        XCTAssertEqual(model.fhirVersion, .r4)
        XCTAssertTrue(model.autoConnect)
    }

    func testInitDoesNotOverwritePersistedValuesWithProvidedDefaults() {
        let defaults = Self.ephemeralDefaults()
        HFSAppModel(defaults: defaults).serverURLString = "https://saved.example.com"

        // Even when a different default URL is supplied, the persisted one wins.
        let restored = HFSAppModel(serverURLString: "http://other:9000", defaults: defaults)
        XCTAssertEqual(restored.serverURLString, "https://saved.example.com")
    }

    // MARK: - Authentication

    func testConnectWithoutTokenSendsNoAuthorizationHeader() async {
        let json = Data(#"{"resourceType":"CapabilityStatement"}"#.utf8)
        let transport = CapturingTransport(statusCode: 200, data: json)
        let model = HFSAppModel(
            serverURLString: "http://localhost:8080",
            transport: transport,
            defaults: Self.ephemeralDefaults()
        )

        await model.connect()

        XCTAssertTrue(model.isConnected)
        XCTAssertFalse(model.hasAccessToken)
        XCTAssertNil(transport.lastRequest?.value(forHTTPHeaderField: "Authorization"))
    }

    func testBlankTokenIsTreatedAsNoAuth() async {
        let json = Data(#"{"resourceType":"CapabilityStatement"}"#.utf8)
        let transport = CapturingTransport(statusCode: 200, data: json)
        let model = HFSAppModel(
            serverURLString: "http://localhost:8080",
            accessToken: "   ",
            transport: transport,
            defaults: Self.ephemeralDefaults()
        )

        await model.connect()

        XCTAssertFalse(model.hasAccessToken)
        XCTAssertNil(transport.lastRequest?.value(forHTTPHeaderField: "Authorization"))
    }

    func testConnectWithTokenSendsBearerHeader() async {
        let json = Data(#"{"resourceType":"CapabilityStatement"}"#.utf8)
        let transport = CapturingTransport(statusCode: 200, data: json)
        let model = HFSAppModel(
            serverURLString: "http://localhost:8080",
            accessToken: "secret-token",
            transport: transport,
            defaults: Self.ephemeralDefaults()
        )

        await model.connect()

        XCTAssertTrue(model.isConnected)
        XCTAssertTrue(model.hasAccessToken)
        XCTAssertEqual(
            transport.lastRequest?.value(forHTTPHeaderField: "Authorization"),
            "Bearer secret-token"
        )
    }

    func testAccessTokenIsNotPersisted() {
        let defaults = Self.ephemeralDefaults()

        let first = HFSAppModel(defaults: defaults)
        first.accessToken = "secret-token"

        // A bearer token is session-only and must never be restored from disk.
        let second = HFSAppModel(defaults: defaults)
        XCTAssertEqual(second.accessToken, "")
        XCTAssertFalse(second.hasAccessToken)
    }

    private static func ephemeralDefaults() -> UserDefaults {
        UserDefaults(suiteName: "hfs.tests.\(UUID().uuidString)")!
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

/// A transport that records the most recent request so tests can assert on the
/// headers the client produced (e.g. `Authorization`).
private final class CapturingTransport: HFSHTTPTransport, @unchecked Sendable {
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
