import HFSCore
import SwiftUI

struct HFSSettingsView: View {
    @Environment(HFSAppModel.self) private var model

    var body: some View {
        @Bindable var model = model

        Form {
            Section("Server") {
                TextField(
                    "Base URL",
                    text: $model.serverURLString,
                    prompt: Text("http://localhost:8080")
                )
                #if !os(macOS)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .keyboardType(.URL)
                #endif

                TextField(
                    "Tenant",
                    text: $model.tenantIdentifier,
                    prompt: Text("default")
                )
                #if !os(macOS)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                #endif

                Picker("FHIR Version", selection: $model.fhirVersion) {
                    ForEach(HFSFHIRVersion.allCases, id: \.self) { version in
                        Text(version.rawValue).tag(version)
                    }
                }
            }

            Section {
                SecureField(
                    "Bearer Token",
                    text: $model.accessToken,
                    prompt: Text("Leave empty for unsecured servers")
                )
                #if !os(macOS)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                #endif
            } header: {
                Text("Authentication")
            } footer: {
                Text(
                    "Sent as an Authorization: Bearer header. Leave empty to connect "
                        + "without auth. The token is kept for this session only and is "
                        + "not saved to disk."
                )
            }

            Section {
                Toggle("Connect on launch", isOn: $model.autoConnect)
            } footer: {
                Text("Server settings are saved automatically and restored next time.")
            }

            Section("Status") {
                LabeledContent("Connection") {
                    connectionStatusLabel
                }

                LabeledContent("Auth") {
                    if model.hasAccessToken {
                        Label("Bearer token", systemImage: "lock.fill")
                            .foregroundStyle(.secondary)
                    } else {
                        Label("None", systemImage: "lock.open")
                            .foregroundStyle(.secondary)
                    }
                }

                if let serverName = model.serverName {
                    LabeledContent("Server", value: serverName)
                }

                if case .failed(let message) = model.connectionState {
                    Label(message, systemImage: "exclamationmark.triangle.fill")
                        .foregroundStyle(.red)
                        .font(.callout)
                        .labelStyle(.titleAndIcon)
                }
            }

            Section {
                if model.isConnected {
                    Button("Disconnect", role: .destructive) {
                        model.disconnect()
                    }
                } else {
                    Button {
                        Task { await model.connect() }
                    } label: {
                        HStack(spacing: 8) {
                            if model.isConnecting {
                                ProgressView()
                                    .controlSize(.small)
                            }
                            Text(model.isConnecting ? "Connecting…" : "Connect")
                        }
                    }
                    .disabled(model.isConnecting)
                }
            } footer: {
                Text("Connecting probes the server's CapabilityStatement at /metadata.")
            }
        }
        .formStyle(.grouped)
    }

    @ViewBuilder
    private var connectionStatusLabel: some View {
        switch model.connectionState {
        case .disconnected:
            Label("Disconnected", systemImage: "circle")
                .foregroundStyle(.secondary)
        case .connecting:
            Label("Connecting…", systemImage: "circle.dotted")
                .foregroundStyle(.secondary)
        case .connected:
            Label("Connected", systemImage: "checkmark.circle.fill")
                .foregroundStyle(.green)
        case .failed:
            Label("Failed", systemImage: "exclamationmark.triangle.fill")
                .foregroundStyle(.red)
        }
    }
}
