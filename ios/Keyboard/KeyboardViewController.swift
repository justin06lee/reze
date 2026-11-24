import RezeUI
import SwiftUI
import UIKit

// The keyboard extension: the view controller iOS asks a keyboard for, and
// nothing more. What the keyboard shows and does is `KeyboardPanel` and
// `KeyboardState` in RezeUI; this hands them the text field being typed into.

final class KeyboardViewController: UIInputViewController {
    /// As tall as the system keyboard with its suggestions bar, so switching
    /// to this one does not make the app above it jump.
    private static let height: CGFloat = 291

    private lazy var state = KeyboardState(proxy: KeyboardState.Proxy(
        insert: { [weak self] text in self?.textDocumentProxy.insertText(text) },
        deleteBackward: { [weak self] in self?.textDocumentProxy.deleteBackward() },
        nextKeyboard: { [weak self] in self?.advanceToNextInputMode() },
        context: { [weak self] in self?.textDocumentProxy.documentContextBeforeInput ?? "" },
        fullAccess: { [weak self] in self?.hasFullAccess ?? false },
        needsGlobe: { [weak self] in self?.needsInputModeSwitchKey ?? true }
    ))

    override func viewDidLoad() {
        super.viewDidLoad()

        let panel = UIHostingController(rootView: KeyboardPanel(state: state))
        panel.view.backgroundColor = .clear
        panel.view.translatesAutoresizingMaskIntoConstraints = false
        addChild(panel)
        view.addSubview(panel.view)

        // Just short of required: iOS sizes a keyboard itself for a moment as
        // it comes up, and a required height fighting that is logged as a
        // broken constraint every time.
        let height = view.heightAnchor.constraint(equalToConstant: Self.height)
        height.priority = UILayoutPriority(999)
        NSLayoutConstraint.activate([
            panel.view.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            panel.view.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            panel.view.topAnchor.constraint(equalTo: view.topAnchor),
            panel.view.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            height,
        ])
        panel.didMove(toParent: self)
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        // Each time it comes up: the app may have fetched since the last.
        state.reload()
    }

    override func textDidChange(_ textInput: UITextInput?) {
        super.textDidChange(textInput)
        state.look()
    }

    override func selectionDidChange(_ textInput: UITextInput?) {
        super.selectionDidChange(textInput)
        state.look()
    }
}
