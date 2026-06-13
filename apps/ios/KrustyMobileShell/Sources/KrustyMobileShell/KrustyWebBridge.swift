import Foundation
import WebKit

public final class KrustyWebBridge: NSObject, WKScriptMessageHandler {
    public enum Handler: String, CaseIterable {
        case browserEvent = "krustyBrowserEvent"
        case terminalInput = "krustyTerminalInput"
        case terminalResize = "krustyTerminalResize"
    }

    public var onEvent: ((KrustyShellEvent) -> Void)?

    public func install(into configuration: WKWebViewConfiguration) {
        for handler in Handler.allCases {
            configuration.userContentController.add(self, name: handler.rawValue)
        }
    }

    public func uninstall(from configuration: WKWebViewConfiguration) {
        for handler in Handler.allCases {
            configuration.userContentController.removeScriptMessageHandler(forName: handler.rawValue)
        }
    }

    public func userContentController(
        _ userContentController: WKUserContentController,
        didReceive message: WKScriptMessage
    ) {
        guard let handler = Handler(rawValue: message.name) else { return }
        switch handler {
        case .browserEvent:
            if let raw = message.body as? String, let url = URL(string: raw) {
                onEvent?(.browserNavigated(url))
            }
        case .terminalInput:
            if let input = message.body as? String {
                onEvent?(.terminalInput(input))
            }
        case .terminalResize:
            guard
                let body = message.body as? [String: Any],
                let columns = body["columns"] as? Int,
                let rows = body["rows"] as? Int
            else { return }
            onEvent?(.terminalResize(columns: columns, rows: rows))
        }
    }
}
