import SwiftUI

#if canImport(AppKit)
import AppKit

/// A plain-text editor for code/JSON that disables macOS smart-quote, dash, and
/// text substitutions, which would otherwise corrupt JSON typed into a
/// `TextEditor`. Uses a monospaced font and no rich text.
struct HFSCodeEditor: NSViewRepresentable {
    @Binding var text: String
    var isEditable: Bool = true

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    func makeNSView(context: Context) -> NSScrollView {
        let scrollView = NSTextView.scrollableTextView()
        guard let textView = scrollView.documentView as? NSTextView else { return scrollView }

        textView.delegate = context.coordinator
        textView.isRichText = false
        textView.isAutomaticQuoteSubstitutionEnabled = false
        textView.isAutomaticDashSubstitutionEnabled = false
        textView.isAutomaticTextReplacementEnabled = false
        textView.isAutomaticSpellingCorrectionEnabled = false
        textView.isContinuousSpellCheckingEnabled = false
        textView.font = .monospacedSystemFont(ofSize: NSFont.systemFontSize, weight: .regular)
        textView.textContainerInset = NSSize(width: 8, height: 8)
        textView.allowsUndo = true
        textView.autoresizingMask = [.width]

        return scrollView
    }

    func updateNSView(_ scrollView: NSScrollView, context: Context) {
        guard let textView = scrollView.documentView as? NSTextView else { return }
        if textView.string != text {
            textView.string = text
        }
        textView.isEditable = isEditable
    }

    final class Coordinator: NSObject, NSTextViewDelegate {
        private let parent: HFSCodeEditor

        init(_ parent: HFSCodeEditor) { self.parent = parent }

        func textDidChange(_ notification: Notification) {
            guard let textView = notification.object as? NSTextView else { return }
            parent.text = textView.string
        }
    }
}

#elseif canImport(UIKit)
import UIKit

/// A plain-text editor for code/JSON that disables smart-quote, dash, and
/// autocorrect substitutions so JSON is not corrupted on entry.
struct HFSCodeEditor: UIViewRepresentable {
    @Binding var text: String
    var isEditable: Bool = true

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    func makeUIView(context: Context) -> UITextView {
        let textView = UITextView()
        textView.delegate = context.coordinator
        textView.smartQuotesType = .no
        textView.smartDashesType = .no
        textView.smartInsertDeleteType = .no
        textView.autocorrectionType = .no
        textView.autocapitalizationType = .none
        textView.spellCheckingType = .no
        textView.font = .monospacedSystemFont(ofSize: UIFont.systemFontSize, weight: .regular)
        textView.textContainerInset = UIEdgeInsets(top: 8, left: 8, bottom: 8, right: 8)
        return textView
    }

    func updateUIView(_ textView: UITextView, context: Context) {
        if textView.text != text {
            textView.text = text
        }
        textView.isEditable = isEditable
    }

    final class Coordinator: NSObject, UITextViewDelegate {
        private let parent: HFSCodeEditor

        init(_ parent: HFSCodeEditor) { self.parent = parent }

        func textViewDidChange(_ textView: UITextView) {
            parent.text = textView.text
        }
    }
}
#endif
