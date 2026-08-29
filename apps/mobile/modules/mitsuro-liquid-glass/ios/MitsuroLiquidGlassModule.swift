import ExpoModulesCore
import Foundation

enum MitsuroLiquidGlassSupport {
  static var isAvailable: Bool {
    #if compiler(>=6.2) // Xcode 26
    if #available(iOS 26.0, *) {
      guard
        let glassEffectClass = NSClassFromString("UIGlassEffect") as? NSObject.Type,
        glassEffectClass.responds(to: Selector(("effectWithStyle:"))),
        NSClassFromString("UIGlassContainerEffect") != nil
      else {
        return false
      }

      if let requiresCompatibility = Bundle.main.infoDictionary?["UIDesignRequiresCompatibility"] as? Bool {
        return !requiresCompatibility
      }
      return true
    }
    #endif
    return false
  }
}

public final class MitsuroLiquidGlassModule: Module {
  public func definition() -> ModuleDefinition {
    Name("MitsuroLiquidGlass")

    Constant("isSupported") {
      MitsuroLiquidGlassSupport.isAvailable
    }

    View(MitsuroLiquidGlassView.self)
  }
}
