import KrustyMobileShell
import UIKit

final class RootViewController: UIViewController {
    private let shell = KrustyMobileShell()
    private let statusLabel = UILabel()
    private let gpuiHost = GpuiHostView()
    private var didStartGpui = false

    override func viewDidLoad() {
        super.viewDidLoad()
        title = "Krusty Mobile"
        view.backgroundColor = UIColor(red: 0.04, green: 0.05, blue: 0.07, alpha: 1)
        shell.restoreServerURL()
        shell.onEvent = { [weak self] event in
            self?.statusLabel.text = Self.describe(event)
        }
        configureLayout()
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        guard !didStartGpui else { return }
        didStartGpui = true
        if gpuiHost.startGpui() {
            statusLabel.text = "GPUI Rust backend started"
        } else {
            statusLabel.text = "GPUI Rust backend not linked yet; shell bridges are available"
        }
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        gpuiHost.bridge.layoutHostView()
    }

    private func configureLayout() {
        let titleLabel = UILabel()
        titleLabel.text = "Krusty GPUI Mobile Shell"
        titleLabel.font = .preferredFont(forTextStyle: .title2)
        titleLabel.textColor = .white
        titleLabel.numberOfLines = 0

        let subtitle = UILabel()
        subtitle.text = "Native iOS shell hosting the chat-first GPUI surface plus WKWebView terminal/browser bridges."
        subtitle.font = .preferredFont(forTextStyle: .body)
        subtitle.textColor = UIColor(white: 1, alpha: 0.7)
        subtitle.numberOfLines = 0

        statusLabel.text = "Server: \(shell.config.serverURL.absoluteString)"
        statusLabel.font = .preferredFont(forTextStyle: .footnote)
        statusLabel.textColor = UIColor(white: 1, alpha: 0.58)
        statusLabel.numberOfLines = 0

        gpuiHost.translatesAutoresizingMaskIntoConstraints = false

        let browserButton = makeButton(title: "Open Browser Bridge", action: #selector(openBrowser))
        let terminalButton = makeButton(title: "Open Terminal Bridge", action: #selector(openTerminal))
        let composer = shell.makeComposerView()
        composer.onSend = { [weak self] text in
            self?.statusLabel.text = "Composer send: \(text)"
        }
        composer.onStop = { [weak self] in
            self?.statusLabel.text = "Composer stop requested"
        }

        let stack = UIStackView(arrangedSubviews: [
            titleLabel,
            subtitle,
            statusLabel,
            gpuiHost,
            browserButton,
            terminalButton,
            composer,
        ])
        stack.axis = .vertical
        stack.spacing = 16
        stack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(stack)

        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 20),
            stack.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -20),
            stack.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 24),
            gpuiHost.heightAnchor.constraint(equalToConstant: 460),
            composer.heightAnchor.constraint(greaterThanOrEqualToConstant: 64),
        ])
    }

    private func makeButton(title: String, action: Selector) -> UIButton {
        var configuration = UIButton.Configuration.filled()
        configuration.title = title
        configuration.baseBackgroundColor = UIColor(red: 0.94, green: 0.47, blue: 0.12, alpha: 1)
        configuration.baseForegroundColor = .black
        let button = UIButton(configuration: configuration)
        button.addTarget(self, action: action, for: .touchUpInside)
        return button
    }

    @objc private func openBrowser() {
        let controller = shell.makeBrowserController()
        present(UINavigationController(rootViewController: controller), animated: true)
    }

    @objc private func openTerminal() {
        let controller = shell.makeTerminalController()
        present(UINavigationController(rootViewController: controller), animated: true)
    }

    private static func describe(_ event: KrustyShellEvent) -> String {
        switch event {
        case .browserNavigated(let url):
            "Browser navigated: \(url.absoluteString)"
        case .browserClosed:
            "Browser closed"
        case .terminalInput(let input):
            "Terminal input: \(input)"
        case .terminalResize(let columns, let rows):
            "Terminal resized: \(columns)x\(rows)"
        case .attachmentRequested:
            "Attachment requested"
        case .deepLink(let url):
            "Deep link: \(url.absoluteString)"
        }
    }
}
