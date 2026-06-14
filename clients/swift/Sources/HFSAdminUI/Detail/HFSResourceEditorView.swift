import SwiftUI

/// A modal JSON editor used to create or update a FHIR resource.
///
/// Validates that the text is well-formed JSON before handing it to `onSave`,
/// which performs the network call and returns an error message to display
/// (or `nil` on success, which dismisses the sheet).
struct HFSResourceEditorView: View {
    let title: String
    let saveButtonTitle: String
    let onSave: (Data) async -> String?

    @Environment(\.dismiss) private var dismiss
    @State private var json: String
    @State private var isSaving = false
    @State private var errorMessage: String?

    init(
        title: String,
        saveButtonTitle: String,
        initialJSON: String,
        onSave: @escaping (Data) async -> String?
    ) {
        self.title = title
        self.saveButtonTitle = saveButtonTitle
        self.onSave = onSave
        _json = State(initialValue: initialJSON)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text(title)
                    .font(.headline)
                Spacer()
            }
            .padding()

            Divider()

            HFSCodeEditor(text: $json, isEditable: !isSaving)
                .frame(minWidth: 480, minHeight: 360)

            if let errorMessage {
                Divider()
                Label(errorMessage, systemImage: "exclamationmark.triangle.fill")
                    .foregroundStyle(.red)
                    .font(.callout)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding()
            }

            Divider()

            HStack(spacing: 12) {
                Spacer()

                Button("Cancel", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)

                Button {
                    Task { await save() }
                } label: {
                    HStack(spacing: 6) {
                        if isSaving {
                            ProgressView().controlSize(.small)
                        }
                        Text(saveButtonTitle)
                    }
                }
                .buttonStyle(.borderedProminent)
                .keyboardShortcut(.defaultAction)
                .disabled(isSaving)
            }
            .padding()
        }
        .frame(minWidth: 520, minHeight: 480)
    }

    private func save() async {
        guard
            let data = json.data(using: .utf8),
            (try? JSONSerialization.jsonObject(with: data)) != nil
        else {
            errorMessage = "The editor does not contain valid JSON."
            return
        }

        isSaving = true
        errorMessage = nil
        let result = await onSave(data)
        isSaving = false

        if let result {
            errorMessage = result
        } else {
            dismiss()
        }
    }
}
