import Darwin
import QuartzCore
import UIKit

@MainActor
public final class GpuiRuntimeBridge: NSObject {
    fileprivate typealias StartWithHostView = @convention(c) (UnsafeMutableRawPointer?) -> Void
    fileprivate typealias GetWindow = @convention(c) () -> UnsafeMutableRawPointer?
    fileprivate typealias WindowCommand = @convention(c) (UnsafeMutableRawPointer?) -> Void
    fileprivate typealias LayoutWindows = @convention(c) () -> Void
    fileprivate typealias LifecycleCommand = @convention(c) () -> Void

    private let symbols = SymbolTable()
    private weak var hostView: UIView?
    private var gpuiWindow: UnsafeMutableRawPointer?
    private var displayLink: CADisplayLink?

    public override init() {
        super.init()
    }

    public var isLinked: Bool {
        symbols.startWithHostView != nil
    }

    @discardableResult
    public func start(in hostView: UIView) -> Bool {
        guard let startWithHostView = symbols.startWithHostView else {
            return false
        }

        self.hostView = hostView
        startWithHostView(Unmanaged.passUnretained(hostView).toOpaque())
        gpuiWindow = symbols.getWindow?()
        symbols.layoutWindows?()
        ensureDisplayLink()
        return true
    }

    public func stopFramePump() {
        displayLink?.invalidate()
        displayLink = nil
    }

    public func didEnterBackground() {
        symbols.didEnterBackground?()
        stopFramePump()
    }

    public func willEnterForeground() {
        symbols.willEnterForeground?()
        if hostView != nil {
            ensureDisplayLink()
        }
    }

    public func layoutHostView() {
        symbols.layoutWindows?()
        forceFrame()
    }

    public func forceFrame() {
        refreshWindowPointerIfNeeded()
        symbols.forceFrame?(gpuiWindow)
    }

    private func ensureDisplayLink() {
        guard displayLink == nil else { return }
        let link = CADisplayLink(target: self, selector: #selector(tick))
        link.add(to: .main, forMode: .common)
        displayLink = link
    }

    @objc private func tick() {
        refreshWindowPointerIfNeeded()
        symbols.requestFrame?(gpuiWindow)
    }

    private func refreshWindowPointerIfNeeded() {
        if gpuiWindow == nil {
            gpuiWindow = symbols.getWindow?()
        }
    }
}

public final class GpuiHostView: UIView {
    public let bridge = GpuiRuntimeBridge()

    override public init(frame: CGRect) {
        super.init(frame: frame)
        configure()
    }

    public required init?(coder: NSCoder) {
        super.init(coder: coder)
        configure()
    }

    deinit {
        bridge.stopFramePump()
    }

    @discardableResult
    public func startGpui() -> Bool {
        bridge.start(in: self)
    }

    override public func layoutSubviews() {
        super.layoutSubviews()
        bridge.layoutHostView()
    }

    private func configure() {
        backgroundColor = UIColor(red: 0.03, green: 0.04, blue: 0.055, alpha: 1)
        layer.borderColor = UIColor(white: 1, alpha: 0.14).cgColor
        layer.borderWidth = 1
        layer.cornerRadius = 12
        clipsToBounds = true
    }
}

private final class SymbolTable {
    let startWithHostView: GpuiRuntimeBridge.StartWithHostView?
    let getWindow: GpuiRuntimeBridge.GetWindow?
    let requestFrame: GpuiRuntimeBridge.WindowCommand?
    let forceFrame: GpuiRuntimeBridge.WindowCommand?
    let layoutWindows: GpuiRuntimeBridge.LayoutWindows?
    let willEnterForeground: GpuiRuntimeBridge.LifecycleCommand?
    let didEnterBackground: GpuiRuntimeBridge.LifecycleCommand?

    init() {
        startWithHostView = Self.load("krusty_mobile_start_with_host_view", as: GpuiRuntimeBridge.StartWithHostView.self)
        getWindow = Self.load("gpui_ios_get_window", as: GpuiRuntimeBridge.GetWindow.self)
        requestFrame = Self.load("gpui_ios_request_frame", as: GpuiRuntimeBridge.WindowCommand.self)
        forceFrame = Self.load("gpui_ios_force_frame", as: GpuiRuntimeBridge.WindowCommand.self)
        layoutWindows = Self.load("gpui_ios_layout_windows", as: GpuiRuntimeBridge.LayoutWindows.self)
        willEnterForeground = Self.load("gpui_ios_will_enter_foreground", as: GpuiRuntimeBridge.LifecycleCommand.self)
        didEnterBackground = Self.load("gpui_ios_did_enter_background", as: GpuiRuntimeBridge.LifecycleCommand.self)
    }

    private static func load<T>(_ name: String, as _: T.Type) -> T? {
        guard let handle = dlopen(nil, RTLD_LAZY), let symbol = dlsym(handle, name) else {
            return nil
        }
        return unsafeBitCast(symbol, to: T.self)
    }
}
