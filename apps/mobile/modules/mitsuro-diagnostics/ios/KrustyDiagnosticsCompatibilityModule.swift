import ExpoModulesCore
import Foundation

/**
 * Native bridge for an older OTA JavaScript bundle running on a new binary.
 * New JavaScript resolves MitsuroDiagnostics first and writes only new state.
 */
public class KrustyDiagnosticsCompatibilityModule: Module {
  public func definition() -> ModuleDefinition {
    mitsuroDiagnosticsDefinition(moduleName: "KrustyDiagnostics")
  }
}

func legacyMetricKitPayloadDirectory() -> URL {
  let base = FileManager.default.urls(
    for: .applicationSupportDirectory,
    in: .userDomainMask
  )[0]
  return base.appendingPathComponent("KrustyDiagnostics/MetricKit", isDirectory: true)
}
