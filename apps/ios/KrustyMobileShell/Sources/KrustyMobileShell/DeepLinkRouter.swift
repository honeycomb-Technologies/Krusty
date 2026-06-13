import Foundation

public final class DeepLinkRouter {
    public init() {}

    public func route(_ url: URL, emit: (KrustyShellEvent) -> Void) {
        emit(.deepLink(url))
    }
}
