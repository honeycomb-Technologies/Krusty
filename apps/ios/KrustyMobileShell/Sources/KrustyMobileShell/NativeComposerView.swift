import UIKit

public final class NativeComposerView: UIView, UITextViewDelegate {
    public var onSend: ((String) -> Void)?
    public var onStop: (() -> Void)?
    public var onAttachment: (() -> Void)?

    public var isStreaming: Bool = false {
        didSet { updateActionButton() }
    }

    private let textView = UITextView()
    private let actionButton = UIButton(type: .system)
    private let attachButton = UIButton(type: .system)

    public override init(frame: CGRect) {
        super.init(frame: frame)
        configure()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    private func configure() {
        backgroundColor = UIColor(red: 0.06, green: 0.08, blue: 0.11, alpha: 1)
        layer.borderWidth = 1 / UIScreen.main.scale
        layer.borderColor = UIColor(white: 1, alpha: 0.12).cgColor

        textView.delegate = self
        textView.backgroundColor = .clear
        textView.textColor = .white
        textView.font = .preferredFont(forTextStyle: .body)
        textView.isScrollEnabled = false
        textView.textContainerInset = UIEdgeInsets(top: 10, left: 8, bottom: 10, right: 8)

        attachButton.setTitle("＋", for: .normal)
        attachButton.addTarget(self, action: #selector(attachPressed), for: .touchUpInside)

        actionButton.addTarget(self, action: #selector(actionPressed), for: .touchUpInside)
        updateActionButton()

        let stack = UIStackView(arrangedSubviews: [attachButton, textView, actionButton])
        stack.axis = .horizontal
        stack.alignment = .bottom
        stack.spacing = 8
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)

        NSLayoutConstraint.activate([
            attachButton.widthAnchor.constraint(equalToConstant: 40),
            actionButton.widthAnchor.constraint(equalToConstant: 44),
            stack.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 8),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -8),
            stack.topAnchor.constraint(equalTo: topAnchor, constant: 8),
            stack.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -8),
        ])
    }

    private func updateActionButton() {
        actionButton.setTitle(isStreaming ? "■" : "↑", for: .normal)
        actionButton.tintColor = isStreaming ? .systemRed : .systemOrange
    }

    @objc private func actionPressed() {
        if isStreaming {
            onStop?()
            return
        }
        let text = textView.text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        textView.text = ""
        onSend?(text)
    }

    @objc private func attachPressed() {
        onAttachment?()
    }
}
