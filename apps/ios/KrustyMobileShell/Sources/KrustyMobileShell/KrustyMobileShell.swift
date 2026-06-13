import Foundation
import UIKit

public struct KrustyMobileShellConfig: Sendable {
    public var serverURL: URL
    public var terminalTitle: String

    public init(
        serverURL: URL = URL(string: "http://127.0.0.1:3000")!,
        terminalTitle: String = "Krusty Terminal"
    ) {
        self.serverURL = serverURL
        self.terminalTitle = terminalTitle
    }
}

public enum KrustyShellEvent: Sendable, Equatable {
    case browserNavigated(URL)
    case browserClosed
    case terminalInput(String)
    case terminalResize(columns: Int, rows: Int)
    case attachmentRequested
    case deepLink(URL)
}

@MainActor
public final class KrustyMobileShell {
    public var config: KrustyMobileShellConfig
    public var onEvent: (@MainActor (KrustyShellEvent) -> Void)?

    private let keychain = KeychainStore(service: "works.krusty.mobile")
    private let deepLinks = DeepLinkRouter()

    public init(config: KrustyMobileShellConfig = KrustyMobileShellConfig()) {
        self.config = config
    }

    public func saveServerURL(_ url: URL) throws {
        config.serverURL = url
        try keychain.set(url.absoluteString, forKey: "server_url")
    }

    public func restoreServerURL() {
        guard let raw = try? keychain.string(forKey: "server_url"), let url = URL(string: raw) else {
            return
        }
        config.serverURL = url
    }

    public func handleDeepLink(_ url: URL) {
        deepLinks.route(url) { [weak self] event in
            self?.onEvent?(event)
        }
    }

    public func makeBrowserController(initialURL: URL? = nil) -> BrowserBridgeViewController {
        let controller = BrowserBridgeViewController(initialURL: initialURL ?? config.serverURL)
        controller.onEvent = { [weak self] event in self?.onEvent?(event) }
        return controller
    }

    public func makeTerminalController(sessionID: String? = nil) -> TerminalBridgeViewController {
        let controller = TerminalBridgeViewController(title: config.terminalTitle, sessionID: sessionID)
        controller.onEvent = { [weak self] event in self?.onEvent?(event) }
        return controller
    }

    public func makeComposerView() -> NativeComposerView {
        let view = NativeComposerView()
        view.onAttachment = { [weak self] in self?.onEvent?(.attachmentRequested) }
        return view
    }
}
