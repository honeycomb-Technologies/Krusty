import CryptoKit
import ExpoModulesCore
import Foundation
import MetricKit
import os

private final class KrustyPerformanceSignposts {
  static let shared = KrustyPerformanceSignposts()

  private let signposter = OSSignposter(
    subsystem: Bundle.main.bundleIdentifier ?? "io.krusty.mobile",
    category: "Performance"
  )
  private let lock = NSLock()
  private var intervals: [Int: OSSignpostIntervalState] = [:]
  private let allowedNames: Set<String> = [
    "app.launch", "new_chat.shell", "new_chat.session_bind", "session.open",
    "stream.connect", "stream.first_event", "stream.flush", "stream.finish",
    "session.snapshot_transform", "session.cache_compact",
    "transcript.derive", "transcript.first_paint", "mode.switch", "toolbox.open",
    "live_activity.update",
  ]

  func begin(spanId: Int, name: String) {
    guard allowedNames.contains(name) else { return }
    lock.lock()
    defer { lock.unlock() }
    guard intervals[spanId] == nil else { return }
    intervals[spanId] = signposter.beginInterval(
      "KrustyPerformance",
      id: signposter.makeSignpostID(),
      "phase=\(name, privacy: .public)"
    )
  }

  func end(spanId: Int, name: String) {
    guard allowedNames.contains(name) else { return }
    lock.lock()
    let state = intervals.removeValue(forKey: spanId)
    lock.unlock()
    guard let state else { return }
    signposter.endInterval(
      "KrustyPerformance",
      state,
      "phase=\(name, privacy: .public)"
    )
  }
}

private let maxDiagnostics = 8
private let maxStacksPerDiagnostic = 8
private let maxFramesPerStack = 32
private let maxFramesPerPayload = 256
private let maxFrameSampleCount = 1_000_000

private struct StoredMetricPayload: Codable {
  let id: String
  let kind: String
  let receivedAtMs: Int64
  let summary: MetricKitSummary
}

private enum MetricKitSummary: Codable {
  case v1(MetricKitV1Summary)
  case v2(MetricKitV2Summary)

  private enum CodingKeys: String, CodingKey {
    case summarySchemaVersion
  }

  init(from decoder: Decoder) throws {
    let container = try decoder.container(keyedBy: CodingKeys.self)
    switch try container.decode(Int.self, forKey: .summarySchemaVersion) {
    case 1:
      self = .v1(try MetricKitV1Summary(from: decoder))
    case 2:
      self = .v2(try MetricKitV2Summary(from: decoder))
    default:
      throw DecodingError.dataCorruptedError(
        forKey: .summarySchemaVersion,
        in: container,
        debugDescription: "Unsupported MetricKit summary schema"
      )
    }
  }

  func encode(to encoder: Encoder) throws {
    switch self {
    case .v1(let summary):
      try summary.encode(to: encoder)
    case .v2(let summary):
      try summary.encode(to: encoder)
    }
  }
}

private struct MetricKitV1Summary: Codable {
  let summarySchemaVersion: Int
  let sourcePayloadBytes: Int
  let hasApplicationLaunchMetrics: Bool
  let hasApplicationResponsivenessMetrics: Bool
  let hasMemoryMetrics: Bool
  let hasCpuMetrics: Bool
  let hasDiskIoMetrics: Bool
  let hasDisplayMetrics: Bool
  let hasNetworkTransferMetrics: Bool
  let hasApplicationExitMetrics: Bool
  let hasCellularConditionMetrics: Bool
  let hasLocationActivityMetrics: Bool
  let hasAnimationMetrics: Bool
  let crashDiagnosticCount: Int
  let hangDiagnosticCount: Int
  let cpuExceptionDiagnosticCount: Int
  let diskWriteExceptionDiagnosticCount: Int
}

private struct MetricKitV2Summary: Codable {
  let summarySchemaVersion: Int
  let periodStartMs: Int64
  let periodEndMs: Int64
  let diagnostics: [MetricKitDiagnostic]
}

private struct MetricKitDiagnostic: Codable {
  let type: String
  let appVersion: String
  let buildVersion: String
  let architecture: String
  let stacks: [MetricKitStack]
}

private struct MetricKitStack: Codable {
  let fingerprintSha256: String
  let threadAttributed: Bool
  let frames: [MetricKitFrame]
}

private struct MetricKitFrame: Codable {
  let binaryUuid: String
  let binaryName: String
  let offset: String
  let sampleCount: Int
}

private struct NativeMetricFrameRecord: Record {
  @Field var binaryUuid: String = ""
  @Field var binaryName: String = ""
  @Field var offset: String = ""
  @Field var sampleCount: Int = 0
}

private struct NativeMetricStackRecord: Record {
  @Field var fingerprintSha256: String = ""
  @Field var threadAttributed: Bool = false
  @Field var frames: [NativeMetricFrameRecord] = []
}

private struct NativeMetricDiagnosticRecord: Record {
  @Field var type: String = ""
  @Field var appVersion: String = ""
  @Field var buildVersion: String = ""
  @Field var architecture: String = ""
  @Field var stacks: [NativeMetricStackRecord] = []
}

private struct NativeMetricPayloadRecord: Record {
  @Field var id: String = ""
  @Field var kind: String = ""
  @Field var receivedAtMs: Int64 = 0
  @Field var summarySchemaVersion: Int = 1
  @Field var sourcePayloadBytes: Int?
  @Field var hasApplicationLaunchMetrics: Bool?
  @Field var hasApplicationResponsivenessMetrics: Bool?
  @Field var hasMemoryMetrics: Bool?
  @Field var hasCpuMetrics: Bool?
  @Field var hasDiskIoMetrics: Bool?
  @Field var hasDisplayMetrics: Bool?
  @Field var hasNetworkTransferMetrics: Bool?
  @Field var hasApplicationExitMetrics: Bool?
  @Field var hasCellularConditionMetrics: Bool?
  @Field var hasLocationActivityMetrics: Bool?
  @Field var hasAnimationMetrics: Bool?
  @Field var crashDiagnosticCount: Int?
  @Field var hangDiagnosticCount: Int?
  @Field var cpuExceptionDiagnosticCount: Int?
  @Field var diskWriteExceptionDiagnosticCount: Int?
  @Field var periodStartMs: Int64?
  @Field var periodEndMs: Int64?
  @Field var diagnostics: [NativeMetricDiagnosticRecord]?
}

private final class KrustyMetricKitCollector: NSObject, MXMetricManagerSubscriber {
  static let shared = KrustyMetricKitCollector()

  private let queue = DispatchQueue(label: "io.krusty.mobile.metrics", qos: .utility)
  private let encoder = JSONEncoder()
  private var observing = false
  private let maxStoredPayloads = 16
  private let maxStoredBytes = 128 * 1024

  private override init() {
    super.init()
  }

  func start() {
    queue.sync {
      guard !observing else { return }
      observing = true
      MXMetricManager.shared.add(self)
    }
  }

  func stop() {
    queue.sync {
      guard observing else { return }
      observing = false
      MXMetricManager.shared.remove(self)
    }
  }

  func didReceive(_ payloads: [MXMetricPayload]) {
    persist(payloads.map { ("metric", summarize(metric: $0)) })
  }

  func didReceive(_ payloads: [MXDiagnosticPayload]) {
    persist(payloads.compactMap { payload in
      summarize(diagnostic: payload).map { ("diagnostic", $0) }
    })
  }

  func list() -> [NativeMetricPayloadRecord] {
    queue.sync {
      let files = payloadFiles()
      var records: [NativeMetricPayloadRecord] = []
      for file in files {
        guard
          let data = try? Data(contentsOf: file),
          let stored = try? JSONDecoder().decode(StoredMetricPayload.self, from: data)
        else {
          try? FileManager.default.removeItem(at: file)
          continue
        }
        var record = NativeMetricPayloadRecord()
        record.id = stored.id
        record.kind = stored.kind
        record.receivedAtMs = stored.receivedAtMs
        switch stored.summary {
        case .v1(let summary):
          record.summarySchemaVersion = 1
          record.sourcePayloadBytes = summary.sourcePayloadBytes
          record.hasApplicationLaunchMetrics = summary.hasApplicationLaunchMetrics
          record.hasApplicationResponsivenessMetrics = summary.hasApplicationResponsivenessMetrics
          record.hasMemoryMetrics = summary.hasMemoryMetrics
          record.hasCpuMetrics = summary.hasCpuMetrics
          record.hasDiskIoMetrics = summary.hasDiskIoMetrics
          record.hasDisplayMetrics = summary.hasDisplayMetrics
          record.hasNetworkTransferMetrics = summary.hasNetworkTransferMetrics
          record.hasApplicationExitMetrics = summary.hasApplicationExitMetrics
          record.hasCellularConditionMetrics = summary.hasCellularConditionMetrics
          record.hasLocationActivityMetrics = summary.hasLocationActivityMetrics
          record.hasAnimationMetrics = summary.hasAnimationMetrics
          record.crashDiagnosticCount = summary.crashDiagnosticCount
          record.hangDiagnosticCount = summary.hangDiagnosticCount
          record.cpuExceptionDiagnosticCount = summary.cpuExceptionDiagnosticCount
          record.diskWriteExceptionDiagnosticCount = summary.diskWriteExceptionDiagnosticCount
        case .v2(let summary):
          record.summarySchemaVersion = 2
          record.periodStartMs = summary.periodStartMs
          record.periodEndMs = summary.periodEndMs
          record.diagnostics = summary.diagnostics.map(nativeDiagnosticRecord)
        }
        records.append(record)
      }
      return records
    }
  }

  func acknowledge(_ ids: [String]) {
    let accepted = Set(ids)
    guard !accepted.isEmpty else { return }
    queue.sync {
      for file in payloadFiles() {
        guard
          let data = try? Data(contentsOf: file),
          let stored = try? JSONDecoder().decode(StoredMetricPayload.self, from: data),
          accepted.contains(stored.id)
        else { continue }
        try? FileManager.default.removeItem(at: file)
      }
    }
  }

  private func persist(_ payloads: [(String, MetricKitSummary)]) {
    queue.async {
      let directory = self.payloadDirectory()
      try? FileManager.default.createDirectory(
        at: directory,
        withIntermediateDirectories: true,
        attributes: [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication]
      )
      for (kind, summary) in payloads {
        let stored = StoredMetricPayload(
          id: UUID().uuidString.lowercased(),
          kind: kind,
          receivedAtMs: Int64(Date().timeIntervalSince1970 * 1000),
          summary: summary
        )
        guard let encoded = try? self.encoder.encode(stored) else { continue }
        let file = directory.appendingPathComponent("\(stored.receivedAtMs)-\(stored.id).json")
        try? encoded.write(
          to: file,
          options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication]
        )
      }
      self.enforceBounds()
    }
  }

  private func summarize(metric payload: MXMetricPayload) -> MetricKitSummary {
    .v1(MetricKitV1Summary(
      summarySchemaVersion: 1,
      sourcePayloadBytes: 0,
      hasApplicationLaunchMetrics: payload.applicationLaunchMetrics != nil,
      hasApplicationResponsivenessMetrics: payload.applicationResponsivenessMetrics != nil,
      hasMemoryMetrics: payload.memoryMetrics != nil,
      hasCpuMetrics: payload.cpuMetrics != nil,
      hasDiskIoMetrics: payload.diskIOMetrics != nil,
      hasDisplayMetrics: payload.displayMetrics != nil,
      hasNetworkTransferMetrics: payload.networkTransferMetrics != nil,
      hasApplicationExitMetrics: payload.applicationExitMetrics != nil,
      hasCellularConditionMetrics: payload.cellularConditionMetrics != nil,
      hasLocationActivityMetrics: payload.locationActivityMetrics != nil,
      hasAnimationMetrics: payload.animationMetrics != nil,
      crashDiagnosticCount: 0,
      hangDiagnosticCount: 0,
      cpuExceptionDiagnosticCount: 0,
      diskWriteExceptionDiagnosticCount: 0
    ))
  }

  private func summarize(diagnostic payload: MXDiagnosticPayload) -> MetricKitSummary? {
    let periodStartMs = milliseconds(payload.timeStampBegin)
    let periodEndMs = milliseconds(payload.timeStampEnd)
    guard periodStartMs > 0, periodEndMs >= periodStartMs else { return nil }

    var diagnostics: [MetricKitDiagnostic] = []
    var totalFrames = 0

    func append(
      _ type: String,
      _ diagnostic: MXDiagnostic,
      _ callStackTree: MXCallStackTree
    ) {
      guard diagnostics.count < maxDiagnostics,
            let summary = summarizeDiagnostic(
              type: type,
              diagnostic: diagnostic,
              callStackTree: callStackTree,
              totalFrames: &totalFrames
            )
      else { return }
      diagnostics.append(summary)
    }

    for diagnostic in payload.crashDiagnostics ?? [] {
      append("crash", diagnostic, diagnostic.callStackTree)
    }
    for diagnostic in payload.hangDiagnostics ?? [] {
      append("hang", diagnostic, diagnostic.callStackTree)
    }
    for diagnostic in payload.cpuExceptionDiagnostics ?? [] {
      append("cpu_exception", diagnostic, diagnostic.callStackTree)
    }
    for diagnostic in payload.diskWriteExceptionDiagnostics ?? [] {
      append("disk_write_exception", diagnostic, diagnostic.callStackTree)
    }

    guard !diagnostics.isEmpty else { return nil }
    return .v2(MetricKitV2Summary(
      summarySchemaVersion: 2,
      periodStartMs: periodStartMs,
      periodEndMs: periodEndMs,
      diagnostics: diagnostics
    ))
  }

  private func summarizeDiagnostic(
    type: String,
    diagnostic: MXDiagnostic,
    callStackTree: MXCallStackTree,
    totalFrames: inout Int
  ) -> MetricKitDiagnostic? {
    guard
      let appVersion = boundedLabel(diagnostic.applicationVersion, maxBytes: 32),
      let buildVersion = boundedLabel(diagnostic.metaData.applicationBuildVersion, maxBytes: 32),
      let architecture = boundedLabel(diagnostic.metaData.platformArchitecture, maxBytes: 16),
      let decoded = try? JSONSerialization.jsonObject(with: callStackTree.jsonRepresentation()),
      let wrapper = decoded as? [String: Any],
      let tree = wrapper["callStackTree"] as? [String: Any],
      let rawStacks = tree["callStacks"] as? [Any]
    else {
      return nil
    }

    var stacks: [MetricKitStack] = []
    for rawStack in rawStacks.prefix(maxStacksPerDiagnostic) {
      guard totalFrames < maxFramesPerPayload,
            let stackObject = rawStack as? [String: Any],
            let roots = stackObject["callStackRootFrames"] as? [Any]
      else { continue }
      var frames: [MetricKitFrame] = []
      collectFrames(roots, frames: &frames, totalFrames: &totalFrames)
      guard !frames.isEmpty else { continue }
      let threadAttributed = stackObject["threadAttributed"] as? Bool ?? false
      stacks.append(MetricKitStack(
        fingerprintSha256: stackFingerprint(
          threadAttributed: threadAttributed,
          frames: frames
        ),
        threadAttributed: threadAttributed,
        frames: frames
      ))
    }
    guard !stacks.isEmpty else { return nil }
    return MetricKitDiagnostic(
      type: type,
      appVersion: appVersion,
      buildVersion: buildVersion,
      architecture: architecture,
      stacks: stacks
    )
  }

  private func collectFrames(
    _ nodes: [Any],
    frames: inout [MetricKitFrame],
    totalFrames: inout Int
  ) {
    for rawNode in nodes {
      guard frames.count < maxFramesPerStack, totalFrames < maxFramesPerPayload else { return }
      guard let node = rawNode as? [String: Any] else { continue }
      if let frame = metricFrame(node) {
        frames.append(frame)
        totalFrames += 1
      }
      if let subframes = node["subFrames"] as? [Any] {
        collectFrames(subframes, frames: &frames, totalFrames: &totalFrames)
      }
    }
  }

  private func enforceBounds() {
    var files = payloadFiles()
    var totalBytes = files.reduce(0) { partial, file in
      partial + ((try? file.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0)
    }
    while files.count > maxStoredPayloads || totalBytes > maxStoredBytes {
      let oldest = files.removeFirst()
      totalBytes -= (try? oldest.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0
      try? FileManager.default.removeItem(at: oldest)
    }
  }

  private func payloadFiles() -> [URL] {
    let keys: [URLResourceKey] = [.isRegularFileKey, .fileSizeKey]
    return ((try? FileManager.default.contentsOfDirectory(
      at: payloadDirectory(),
      includingPropertiesForKeys: keys,
      options: [.skipsHiddenFiles]
    )) ?? [])
      .filter { $0.pathExtension == "json" }
      .sorted { $0.lastPathComponent < $1.lastPathComponent }
  }

  private func payloadDirectory() -> URL {
    let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
    return base.appendingPathComponent("KrustyDiagnostics/MetricKit", isDirectory: true)
  }
}

private func nativeDiagnosticRecord(_ diagnostic: MetricKitDiagnostic) -> NativeMetricDiagnosticRecord {
  var record = NativeMetricDiagnosticRecord()
  record.type = diagnostic.type
  record.appVersion = diagnostic.appVersion
  record.buildVersion = diagnostic.buildVersion
  record.architecture = diagnostic.architecture
  record.stacks = diagnostic.stacks.map { stack in
    var stackRecord = NativeMetricStackRecord()
    stackRecord.fingerprintSha256 = stack.fingerprintSha256
    stackRecord.threadAttributed = stack.threadAttributed
    stackRecord.frames = stack.frames.map { frame in
      var frameRecord = NativeMetricFrameRecord()
      frameRecord.binaryUuid = frame.binaryUuid
      frameRecord.binaryName = frame.binaryName
      frameRecord.offset = frame.offset
      frameRecord.sampleCount = frame.sampleCount
      return frameRecord
    }
    return stackRecord
  }
  return record
}

private func metricFrame(_ object: [String: Any]) -> MetricKitFrame? {
  guard
    let rawUuid = object["binaryUUID"] as? String,
    let uuid = UUID(uuidString: rawUuid)?.uuidString.lowercased(),
    let rawName = object["binaryName"] as? String,
    let binaryName = boundedBasename(rawName),
    let offset = decimalUInt64(object["offsetIntoBinaryTextSegment"]),
    let sampleCount = boundedUInt64(object["sampleCount"], maximum: UInt64(maxFrameSampleCount))
  else {
    return nil
  }
  return MetricKitFrame(
    binaryUuid: uuid,
    binaryName: binaryName,
    offset: offset,
    sampleCount: Int(sampleCount)
  )
}

private func decimalUInt64(_ value: Any?) -> String? {
  guard let number = value as? NSNumber,
        CFGetTypeID(number) != CFBooleanGetTypeID()
  else { return nil }
  let text = number.stringValue
  guard !text.isEmpty,
        text.allSatisfy(\.isNumber),
        let parsed = UInt64(text)
  else { return nil }
  return String(parsed)
}

private func boundedUInt64(_ value: Any?, maximum: UInt64) -> UInt64? {
  guard let text = decimalUInt64(value),
        let parsed = UInt64(text),
        parsed <= maximum
  else { return nil }
  return parsed
}

private func boundedBasename(_ value: String) -> String? {
  let basename = (value as NSString).lastPathComponent
  guard !basename.isEmpty,
        basename.utf8.count <= 96,
        !basename.contains("/"),
        !basename.contains("\\"),
        !basename.contains("?"),
        !basename.contains("://"),
        !basename.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains)
  else { return nil }
  return basename
}

private func boundedLabel(_ value: String, maxBytes: Int) -> String? {
  guard !value.isEmpty,
        value.utf8.count <= maxBytes,
        value.utf8.allSatisfy({ byte in
          (byte >= 48 && byte <= 57)
            || (byte >= 65 && byte <= 90)
            || (byte >= 97 && byte <= 122)
            || byte == 45
            || byte == 46
            || byte == 95
        })
  else { return nil }
  return value
}

private func stackFingerprint(
  threadAttributed: Bool,
  frames: [MetricKitFrame]
) -> String {
  let canonical = ([threadAttributed ? "1" : "0"] + frames.map { frame in
    "\(frame.binaryUuid)|\(frame.binaryName)|\(frame.offset)|\(frame.sampleCount)"
  }).joined(separator: "\n")
  return SHA256.hash(data: Data(canonical.utf8))
    .map { String(format: "%02x", $0) }
    .joined()
}

private func milliseconds(_ date: Date) -> Int64 {
  Int64((date.timeIntervalSince1970 * 1_000).rounded())
}

public class KrustyDiagnosticsModule: Module {
  public func definition() -> ModuleDefinition {
    Name("KrustyDiagnostics")

    OnCreate {
      KrustyMetricKitCollector.shared.start()
    }

    OnDestroy {
      KrustyMetricKitCollector.shared.stop()
    }

    Function("isMetricKitAvailable") { () -> Bool in
      true
    }

    Function("getBuildNumber") { () -> String? in
      Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String
    }

    Function("beginPerformanceSpan") { (spanId: Int, name: String) -> Void in
      KrustyPerformanceSignposts.shared.begin(spanId: spanId, name: name)
    }

    Function("endPerformanceSpan") { (spanId: Int, name: String) -> Void in
      KrustyPerformanceSignposts.shared.end(spanId: spanId, name: name)
    }

    AsyncFunction("listMetricKitPayloads") { () -> [NativeMetricPayloadRecord] in
      KrustyMetricKitCollector.shared.list()
    }

    AsyncFunction("acknowledgeMetricKitPayloads") { (ids: [String]) -> Void in
      KrustyMetricKitCollector.shared.acknowledge(ids)
    }
  }
}
