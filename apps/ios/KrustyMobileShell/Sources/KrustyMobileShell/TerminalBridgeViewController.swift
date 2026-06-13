import UIKit
import WebKit

public final class TerminalBridgeViewController: UIViewController {
    public var onEvent: ((KrustyShellEvent) -> Void)? {
        didSet { bridge.onEvent = onEvent }
    }

    private let bridge = KrustyWebBridge()
    private let terminalTitle: String
    private let sessionID: String?
    private var webView: WKWebView?

    public init(title: String, sessionID: String?) {
        self.terminalTitle = title
        self.sessionID = sessionID
        super.init(nibName: nil, bundle: nil)
        self.title = title
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("init(coder:) has not been implemented")
    }

    public override func loadView() {
        let configuration = WKWebViewConfiguration()
        bridge.install(into: configuration)
        let webView = WKWebView(frame: .zero, configuration: configuration)
        webView.isOpaque = false
        webView.backgroundColor = .black
        self.webView = webView
        view = webView
    }

    public override func viewDidLoad() {
        super.viewDidLoad()
        webView?.loadHTMLString(Self.terminalHTML(title: terminalTitle, sessionID: sessionID), baseURL: nil)
    }

    public func write(_ text: String) {
        let literal = Self.javaScriptStringLiteral(text)
        webView?.evaluateJavaScript("window.krustyTerminal?.write(\(literal))")
    }

    deinit {
        if let configuration = webView?.configuration {
            bridge.uninstall(from: configuration)
        }
    }

    private static func terminalHTML(title: String, sessionID: String?) -> String {
        let safeTitle = htmlEscaped(title)
        let safeSessionID = htmlEscaped(sessionID ?? "new")
        return """
        <!doctype html>
        <html>
        <head>
          <meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover" />
          <style>
            html, body { margin: 0; height: 100%; background: #080b10; color: #e7edf7; font: 13px ui-monospace, Menlo, monospace; }
            #term { box-sizing: border-box; min-height: 100%; padding: 12px; white-space: pre-wrap; word-break: break-word; }
            .muted { color: #8f9bad; }
          </style>
        </head>
        <body>
          <div id="term"><span class="muted">\(safeTitle) · \(safeSessionID)</span>\n$ </div>
          <script>
            const term = document.getElementById('term');
            window.krustyTerminal = {
              write(text) { term.textContent += text; window.scrollTo(0, document.body.scrollHeight); }
            };
            window.addEventListener('keydown', event => {
              window.webkit?.messageHandlers?.krustyTerminalInput?.postMessage(event.key);
            });
            function reportSize() {
              const columns = Math.max(20, Math.floor(window.innerWidth / 8));
              const rows = Math.max(8, Math.floor(window.innerHeight / 17));
              window.webkit?.messageHandlers?.krustyTerminalResize?.postMessage({ columns, rows });
            }
            window.addEventListener('resize', reportSize);
            reportSize();
          </script>
        </body>
        </html>
        """
    }

    private static func javaScriptStringLiteral(_ value: String) -> String {
        guard
            let data = try? JSONEncoder().encode(value),
            let literal = String(data: data, encoding: .utf8)
        else {
            return "\"\""
        }
        return literal
    }

    private static func htmlEscaped(_ value: String) -> String {
        value
            .replacingOccurrences(of: "&", with: "&amp;")
            .replacingOccurrences(of: "<", with: "&lt;")
            .replacingOccurrences(of: ">", with: "&gt;")
            .replacingOccurrences(of: "\"", with: "&quot;")
            .replacingOccurrences(of: "'", with: "&#39;")
    }
}
