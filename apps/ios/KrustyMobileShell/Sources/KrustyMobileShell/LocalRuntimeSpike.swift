import Foundation

public enum LocalRuntimeStatus: Sendable, Equatable {
    case unavailable
    case spikeOnly(reason: String)
    case booting
    case ready
    case failed(String)
}

public struct LocalRuntimePlan: Sendable, Equatable {
    public var status: LocalRuntimeStatus
    public var notes: [String]

    public static let litterIshSpike = LocalRuntimePlan(
        status: .spikeOnly(reason: "litter-ish is GPL/iSH-derived and must stay isolated until legal, App Store, and host↔guest RPC gates pass."),
        notes: [
            "Do not link GPL litter-ish code into MIT Krusty crates.",
            "Use a separate Xcode target or submodule checkout for runtime experiments.",
            "First gates: boot Alpine ARM64, run small CLI workloads, prove networking/filesystem, then build host↔guest RPC.",
        ]
    )
}
