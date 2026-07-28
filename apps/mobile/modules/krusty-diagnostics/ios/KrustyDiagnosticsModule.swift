import ExpoModulesCore
import Foundation
import MetricKit

private struct StoredMetricPayload: Codable {
  let id: String
  let kind: String
  let receivedAtMs: Int64
  let summary: MetricKitSummary
}

private struct MetricKitSummary: Codable {
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

private struct NativeMetricPayloadRecord: Record {
  @Field var id: String = ""
  @Field var kind: String = ""
  @Field var receivedAtMs: Int64 = 0
  @Field var summarySchemaVersion: Int = 1
  @Field var sourcePayloadBytes: Int = 0
  @Field var hasApplicationLaunchMetrics: Bool = false
  @Field var hasApplicationResponsivenessMetrics: Bool = false
  @Field var hasMemoryMetrics: Bool = false
  @Field var hasCpuMetrics: Bool = false
  @Field var hasDiskIoMetrics: Bool = false
  @Field var hasDisplayMetrics: Bool = false
  @Field var hasNetworkTransferMetrics: Bool = false
  @Field var hasApplicationExitMetrics: Bool = false
  @Field var hasCellularConditionMetrics: Bool = false
  @Field var hasLocationActivityMetrics: Bool = false
  @Field var hasAnimationMetrics: Bool = false
  @Field var crashDiagnosticCount: Int = 0
  @Field var hangDiagnosticCount: Int = 0
  @Field var cpuExceptionDiagnosticCount: Int = 0
  @Field var diskWriteExceptionDiagnosticCount: Int = 0
}

private final class KrustyMetricKitCollector: NSObject, MXMetricManagerSubscriber {
  static let shared = KrustyMetricKitCollector()

  private let queue = DispatchQueue(label: "io.krusty.mobile.metrics", qos: .utility)
  private let encoder = JSONEncoder()
  private var observing = false
  private let maxSourcePayloadBytes = 2 * 1024 * 1024
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
    persist(payloads.map { ("metric", $0.jsonRepresentation()) })
  }

  func didReceive(_ payloads: [MXDiagnosticPayload]) {
    persist(payloads.map { ("diagnostic", $0.jsonRepresentation()) })
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
        record.summarySchemaVersion = stored.summary.summarySchemaVersion
        record.sourcePayloadBytes = stored.summary.sourcePayloadBytes
        record.hasApplicationLaunchMetrics = stored.summary.hasApplicationLaunchMetrics
        record.hasApplicationResponsivenessMetrics = stored.summary.hasApplicationResponsivenessMetrics
        record.hasMemoryMetrics = stored.summary.hasMemoryMetrics
        record.hasCpuMetrics = stored.summary.hasCpuMetrics
        record.hasDiskIoMetrics = stored.summary.hasDiskIoMetrics
        record.hasDisplayMetrics = stored.summary.hasDisplayMetrics
        record.hasNetworkTransferMetrics = stored.summary.hasNetworkTransferMetrics
        record.hasApplicationExitMetrics = stored.summary.hasApplicationExitMetrics
        record.hasCellularConditionMetrics = stored.summary.hasCellularConditionMetrics
        record.hasLocationActivityMetrics = stored.summary.hasLocationActivityMetrics
        record.hasAnimationMetrics = stored.summary.hasAnimationMetrics
        record.crashDiagnosticCount = stored.summary.crashDiagnosticCount
        record.hangDiagnosticCount = stored.summary.hangDiagnosticCount
        record.cpuExceptionDiagnosticCount = stored.summary.cpuExceptionDiagnosticCount
        record.diskWriteExceptionDiagnosticCount = stored.summary.diskWriteExceptionDiagnosticCount
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

  private func persist(_ payloads: [(String, Data)]) {
    queue.async {
      let directory = self.payloadDirectory()
      try? FileManager.default.createDirectory(
        at: directory,
        withIntermediateDirectories: true,
        attributes: [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication]
      )
      for (kind, data) in payloads where data.count <= self.maxSourcePayloadBytes {
        guard let summary = self.summarize(kind: kind, data: data) else { continue }
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

  private func summarize(kind: String, data: Data) -> MetricKitSummary? {
    guard
      let decoded = try? JSONSerialization.jsonObject(with: data),
      let object = decoded as? [String: Any]
    else {
      return nil
    }
    let isMetric = kind == "metric"
    let isDiagnostic = kind == "diagnostic"
    return MetricKitSummary(
      summarySchemaVersion: 1,
      sourcePayloadBytes: data.count,
      hasApplicationLaunchMetrics: isMetric && object["applicationLaunchMetrics"] != nil,
      hasApplicationResponsivenessMetrics: isMetric && object["applicationResponsivenessMetrics"] != nil,
      hasMemoryMetrics: isMetric && object["memoryMetrics"] != nil,
      hasCpuMetrics: isMetric && object["cpuMetrics"] != nil,
      hasDiskIoMetrics: isMetric && object["diskIOMetrics"] != nil,
      hasDisplayMetrics: isMetric && object["displayMetrics"] != nil,
      hasNetworkTransferMetrics: isMetric && object["networkTransferMetrics"] != nil,
      hasApplicationExitMetrics: isMetric && object["applicationExitMetrics"] != nil,
      hasCellularConditionMetrics: isMetric && object["cellularConditionMetrics"] != nil,
      hasLocationActivityMetrics: isMetric && object["locationActivityMetrics"] != nil,
      hasAnimationMetrics: isMetric && object["animationMetrics"] != nil,
      crashDiagnosticCount: isDiagnostic
        ? boundedCollectionCount(object["crashDiagnostics"])
        : 0,
      hangDiagnosticCount: isDiagnostic
        ? boundedCollectionCount(object["hangDiagnostics"])
        : 0,
      cpuExceptionDiagnosticCount: isDiagnostic
        ? boundedCollectionCount(object["cpuExceptionDiagnostics"])
        : 0,
      diskWriteExceptionDiagnosticCount: isDiagnostic
        ? boundedCollectionCount(object["diskWriteExceptionDiagnostics"])
        : 0
    )
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

private func boundedCollectionCount(_ value: Any?) -> Int {
  guard let collection = value as? [Any] else { return 0 }
  return min(collection.count, 1_000)
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

    AsyncFunction("listMetricKitPayloads") { () -> [NativeMetricPayloadRecord] in
      KrustyMetricKitCollector.shared.list()
    }

    AsyncFunction("acknowledgeMetricKitPayloads") { (ids: [String]) -> Void in
      KrustyMetricKitCollector.shared.acknowledge(ids)
    }
  }
}
