import UIKit
import WebKit

public final class BrowserBridgeViewController: UIViewController, WKNavigationDelegate {
    public var onEvent: ((KrustyShellEvent) -> Void)? {
        didSet { bridge.onEvent = onEvent }
    }

    private let initialURL: URL
    private let bridge = KrustyWebBridge()
    private var webView: WKWebView?

    public init(initialURL: URL) {
        self.initialURL = initialURL
        super.init(nibName: nil, bundle: nil)
        title = "Browser"
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    public override func loadView() {
        let configuration = WKWebViewConfiguration()
        bridge.install(into: configuration)
        let webView = WKWebView(frame: .zero, configuration: configuration)
        webView.navigationDelegate = self
        webView.allowsBackForwardNavigationGestures = true
        self.webView = webView
        view = webView
    }

    public override func viewDidLoad() {
        super.viewDidLoad()
        webView?.load(URLRequest(url: initialURL))
    }

    public func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        guard let url = webView.url else { return }
        onEvent?(.browserNavigated(url))
    }

    public override func viewDidDisappear(_ animated: Bool) {
        super.viewDidDisappear(animated)
        if isBeingDismissed || navigationController?.isBeingDismissed == true {
            onEvent?(.browserClosed)
        }
    }

    deinit {
        if let configuration = webView?.configuration {
            bridge.uninstall(from: configuration)
        }
    }
}
