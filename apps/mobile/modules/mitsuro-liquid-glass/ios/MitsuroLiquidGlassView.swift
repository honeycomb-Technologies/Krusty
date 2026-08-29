import ExpoModulesCore
import SwiftUI

public final class MitsuroLiquidGlassViewProps: ExpoSwiftUI.ViewProps, ExpoSwiftUI.SafeAreaControllable {
  public var ignoreSafeArea: ExpoSwiftUI.IgnoreSafeArea? = .all

  @Field var mode: String = "global"
  @Field var open: Bool = false
  @Field var count: Int = 0

  @Field var rootX: Double = 28
  @Field var rootY: Double = 28
  @Field var rootWidth: Double = 56
  @Field var rootHeight: Double = 56
  @Field var rootCornerRadius: Double = 18

  @Field var showComposer: Bool = false
  @Field var composerX: Double = 0
  @Field var composerY: Double = 0
  @Field var composerWidth: Double = 0
  @Field var composerHeight: Double = 0
  @Field var composerCornerRadius: Double = 18

  @Field var verticalStep: Double = 66
  @Field var p0: Double = -1
  @Field var p1: Double = -1
  @Field var p2: Double = -1
  @Field var p3: Double = -1
  @Field var p4: Double = -1
  @Field var p5: Double = -1

  @Field var attachmentOpen: Bool = false
  @Field var attachmentCount: Int = 0
  @Field var attachmentP0: Double = -1
  @Field var attachmentP1: Double = -1
  @Field var attachmentP2: Double = -1
  @Field var attachmentSourceIndex: Int = 4
  @Field var attachmentStep: Double = 66

  @Field var providerOpen: Bool = false
  @Field var providerCount: Int = 0
  @Field var q0: Double = -1
  @Field var q1: Double = -1
  @Field var q2: Double = -1
  @Field var q3: Double = -1
  @Field var q4: Double = -1
  @Field var q5: Double = -1
  @Field var providerX0: Double?
  @Field var providerX1: Double?
  @Field var providerX2: Double?
  @Field var providerX3: Double?
  @Field var providerX4: Double?
  @Field var providerX5: Double?
  @Field var providerY0: Double?
  @Field var providerY1: Double?
  @Field var providerY2: Double?
  @Field var providerY3: Double?
  @Field var providerY4: Double?
  @Field var providerY5: Double?
  @Field var providerScale0: Double = 1
  @Field var providerScale1: Double = 1
  @Field var providerScale2: Double = 1
  @Field var providerScale3: Double = 1
  @Field var providerScale4: Double = 1
  @Field var providerScale5: Double = 1
  @Field var providerRotation0: Double = 0
  @Field var providerRotation1: Double = 0
  @Field var providerRotation2: Double = 0
  @Field var providerRotation3: Double = 0
  @Field var providerRotation4: Double = 0
  @Field var providerRotation5: Double = 0
  @Field var providerViewportClip: Double = 0
  @Field var providerScrollShift: Double = 0
  @Field var providerSourceIndex: Int = 5
  @Field var providerStep: Double = 66

  @Field var modelOpen: Bool = false
  @Field var modelProgress: Double = -1
  @Field var modelSourceIndex: Int = 5
  @Field var modelX: Double = 0
  @Field var modelY: Double = 0
  @Field var modelWidth: Double = 0
  @Field var modelHeight: Double = 0
  @Field var modelCornerRadius: Double = 18

  @Field var effectSpacing: Double = 8
  @Field var tintColor: Color?
  @Field var colorScheme: String = "auto"
}

/// Keep this ceiling until the complete FAB interaction has been profiled on a
/// physical iOS 26 device. Raising it can turn native glass into the bottleneck.
private let maximumGlassShapeCount = 17

private enum MitsuroLiquidGlassMode: String {
  case global
  case vertical
  case horizontal
  case panel
}

private enum MitsuroLiquidGlassColorScheme: String {
  case auto
  case light
  case dark
}

private struct ExplicitColorSchemeModifier: ViewModifier {
  let colorScheme: ColorScheme?

  @ViewBuilder
  func body(content: Content) -> some View {
    if let colorScheme {
      content.environment(\.colorScheme, colorScheme)
    } else {
      content
    }
  }
}

public struct MitsuroLiquidGlassView: ExpoSwiftUI.View, ExpoSwiftUI.WithHostingView {
  @ObservedObject public var props: MitsuroLiquidGlassViewProps
  @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
  @Environment(\.colorScheme) private var inheritedColorScheme
  @Namespace private var glassNamespace

  public init(props: MitsuroLiquidGlassViewProps) {
    self.props = props
  }

  public var body: some View {
    Group {
      #if compiler(>=6.2) // Xcode 26
      if #available(iOS 26.0, *), MitsuroLiquidGlassSupport.isAvailable {
        liquidGlassBody
      } else {
        transparentFallback
      }
      #else
      transparentFallback
      #endif
    }
    .modifier(ExplicitColorSchemeModifier(colorScheme: explicitColorScheme))
    .ignoresSafeArea()
    .frame(maxWidth: .infinity, maxHeight: .infinity)
    .background(Color.clear)
    .allowsHitTesting(false)
    .accessibilityHidden(true)
  }

  private var transparentFallback: some View {
    Color.clear
  }

  #if compiler(>=6.2) // Xcode 26
  @available(iOS 26.0, *)
  private var liquidGlassBody: some View {
    GlassEffectContainer(spacing: sanitized(props.effectSpacing, fallback: 8, min: 0, max: 160)) {
      ZStack(alignment: .topLeading) {
        if composerShapeCount == 1 {
          glassSurface(
            id: "composer",
            x: sanitized(props.composerX, fallback: 0, min: -2_000, max: 2_000),
            y: sanitized(props.composerY, fallback: 0, min: -2_000, max: 2_000),
            width: composerWidth,
            height: composerHeight,
            cornerRadius: sanitized(
              props.composerCornerRadius,
              fallback: 18,
              min: 0,
              max: min(composerWidth, composerHeight) / 2
            )
          )
        }

        glassSurface(
          id: "agent-root",
          x: rootX,
          y: rootY,
          width: rootWidth,
          height: rootHeight,
          cornerRadius: rootCornerRadius
        )

        if rendersVertical {
          verticalSurfaces
        }
        if rendersHorizontal {
          attachmentSurfaces
          providerSurfaces
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            .mask(alignment: .topLeading) {
              Rectangle()
                .frame(width: providerMaskRight)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            }
        }
        if modelPanelShouldRender {
          modelPanelSurface
        }
      }
      .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }
  }

  @available(iOS 26.0, *)
  @ViewBuilder
  private var verticalSurfaces: some View {
    ForEach(renderedVerticalIndices, id: \.self) { index in
      let progress = verticalProgress(index)
      let center = verticalCenter(index: index, progress: progress)
      glassSurface(
        id: "vertical-\(index)",
        x: center.x,
        y: center.y,
        width: rootWidth,
        height: rootHeight,
        cornerRadius: rootCornerRadius
      )
    }
  }

  @available(iOS 26.0, *)
  @ViewBuilder
  private var attachmentSurfaces: some View {
    let source = sourceCenter(index: props.attachmentSourceIndex)
    let step = sanitized(props.attachmentStep, fallback: 66, min: 1, max: 240)
    ForEach(renderedAttachmentIndices, id: \.self) { index in
      let progress = attachmentProgress(index)
      glassSurface(
        id: "attachment-\(index)",
        x: interpolate(source.x, source.x - step * CGFloat(index + 1), progress),
        y: source.y,
        width: rootWidth,
        height: rootHeight,
        cornerRadius: rootCornerRadius
      )
    }
  }

  @available(iOS 26.0, *)
  @ViewBuilder
  private var providerSurfaces: some View {
    let source = sourceCenter(index: props.providerSourceIndex)
    let step = sanitized(props.providerStep, fallback: 66, min: 1, max: 240)
    let scrollShift = sanitized(props.providerScrollShift, fallback: 0, min: -4_000, max: 4_000)
    ForEach(renderedProviderIndices, id: \.self) { index in
      let progress = providerProgress(index)
      let fallbackX = source.x - step * CGFloat(index + 1) - scrollShift
      let targetX = sanitized(providerX(index), fallback: fallbackX, min: -4_000, max: 4_000)
      let targetY = sanitized(providerY(index), fallback: source.y, min: -4_000, max: 4_000)
      let targetScale = sanitized(providerScale(index), fallback: 1, min: 0.5, max: 1.5)
      let targetRotation = sanitized(providerRotation(index), fallback: 0, min: -30, max: 30)
      glassSurface(
        id: "provider-\(index)",
        x: interpolate(source.x, targetX, progress),
        y: interpolate(source.y, targetY, progress),
        width: rootWidth,
        height: rootHeight,
        cornerRadius: rootCornerRadius,
        scale: interpolate(1, targetScale, progress),
        rotation: interpolate(0, targetRotation, progress)
      )
    }
  }

  @available(iOS 26.0, *)
  @ViewBuilder
  private var modelPanelSurface: some View {
    let progress = branchProgress(props.modelProgress, open: props.modelOpen)
    let targetWidth = sanitized(props.modelWidth, fallback: 0, min: 0, max: 1_600)
    let targetHeight = sanitized(props.modelHeight, fallback: 0, min: 0, max: 2_000)
    let source = sourceCenter(index: props.modelSourceIndex)
    let targetX = sanitized(props.modelX, fallback: source.x, min: -2_000, max: 2_000)
    let targetY = sanitized(props.modelY, fallback: source.y, min: -2_000, max: 2_000)
    let width = interpolate(rootWidth, targetWidth, progress)
    let height = interpolate(rootHeight, targetHeight, progress)
    glassSurface(
      id: "model-panel",
      x: interpolate(source.x, targetX, progress),
      y: interpolate(source.y, targetY, progress),
      width: width,
      height: height,
      cornerRadius: interpolate(
        rootCornerRadius,
        sanitized(props.modelCornerRadius, fallback: 18, min: 0, max: min(targetWidth, targetHeight) / 2),
        progress
      )
    )
  }

  @available(iOS 26.0, *)
  @ViewBuilder
  private func glassSurface(
    id: String,
    x: CGFloat,
    y: CGFloat,
    width: CGFloat,
    height: CGFloat,
    cornerRadius: CGFloat,
    scale: CGFloat = 1,
    rotation: CGFloat = 0
  ) -> some View {
    if reduceTransparency {
      RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
        .fill(reduceTransparencyFill)
        .frame(width: width, height: height)
        .scaleEffect(scale)
        .rotationEffect(.degrees(rotation))
        .position(x: x, y: y)
    } else {
      Color.clear
        .frame(width: width, height: height)
        .glassEffect(
          .clear.interactive(false).tint(props.tintColor),
          in: RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
        )
        .glassEffectID(id, in: glassNamespace)
        .glassEffectTransition(.matchedGeometry)
        .scaleEffect(scale)
        .rotationEffect(.degrees(rotation))
        .position(x: x, y: y)
    }
  }
  #endif

  private var parsedMode: MitsuroLiquidGlassMode {
    MitsuroLiquidGlassMode(rawValue: props.mode) ?? .global
  }

  private var rendersVertical: Bool {
    parsedMode == .global || parsedMode == .vertical
  }

  private var rendersHorizontal: Bool {
    parsedMode == .global || parsedMode == .horizontal
  }

  private var rendersPanel: Bool {
    parsedMode == .global || parsedMode == .panel
  }

  private var rootX: CGFloat {
    sanitized(props.rootX, fallback: 28, min: -2_000, max: 2_000)
  }

  private var rootY: CGFloat {
    sanitized(props.rootY, fallback: 28, min: -2_000, max: 2_000)
  }

  private var rootWidth: CGFloat {
    sanitized(props.rootWidth, fallback: 56, min: 1, max: 320)
  }

  private var rootHeight: CGFloat {
    sanitized(props.rootHeight, fallback: 56, min: 1, max: 320)
  }

  private var rootCornerRadius: CGFloat {
    sanitized(
      props.rootCornerRadius,
      fallback: 18,
      min: 0,
      max: min(rootWidth, rootHeight) / 2
    )
  }

  private var composerWidth: CGFloat {
    sanitized(props.composerWidth, fallback: 0, min: 0, max: 2_000)
  }

  private var composerHeight: CGFloat {
    sanitized(props.composerHeight, fallback: 0, min: 0, max: 1_000)
  }

  private var composerShapeCount: Int {
    props.showComposer && composerWidth > 0 && composerHeight > 0 ? 1 : 0
  }

  private var modelPanelShouldRender: Bool {
    guard rendersPanel else { return false }
    let progress = branchProgress(props.modelProgress, open: props.modelOpen)
    let width = sanitized(props.modelWidth, fallback: 0, min: 0, max: 1_600)
    let height = sanitized(props.modelHeight, fallback: 0, min: 0, max: 2_000)
    return progress > 0 && width > 0 && height > 0
  }

  private var modelPanelShapeCount: Int {
    modelPanelShouldRender ? 1 : 0
  }

  private var requestedVerticalCount: Int {
    boundedCount(props.count, max: 6)
  }

  private var requestedAttachmentCount: Int {
    boundedCount(props.attachmentCount, max: 3)
  }

  private var requestedProviderCount: Int {
    boundedCount(props.providerCount, max: 6)
  }

  private var activeVerticalIndices: [Int] {
    guard rendersVertical else { return [] }
    return (0..<requestedVerticalCount).filter { verticalProgress($0) > 0 }
  }

  private var activeAttachmentIndices: [Int] {
    guard rendersHorizontal else { return [] }
    return (0..<requestedAttachmentCount).filter { attachmentProgress($0) > 0 }
  }

  private var activeProviderIndices: [Int] {
    guard rendersHorizontal else { return [] }
    return (0..<requestedProviderCount).filter { providerProgress($0) > 0 }
  }

  /// Provider glass shares the model FAB as its source, so moving shapes need
  /// the source square while pouring. Once the rail settles, collapse the mask
  /// to the source's left edge—the exact right edge of the RN ScrollView.
  /// Close clears this only after the rail snaps back to its aligned end.
  private var providerMaskRight: CGFloat {
    let clip = sanitized(props.providerViewportClip, fallback: 0, min: 0, max: 1)
    let sourceLeft = rootX - rootWidth / 2
    let sourceRight = rootX + rootWidth / 2
    return interpolate(sourceRight, sourceLeft, clip)
  }

  private var renderedVerticalIndices: [Int] {
    let available = maximumGlassShapeCount - 1 - composerShapeCount - modelPanelShapeCount
    return Array(activeVerticalIndices.prefix(max(0, available)))
  }

  private var renderedAttachmentIndices: [Int] {
    let available = maximumGlassShapeCount
      - 1
      - composerShapeCount
      - modelPanelShapeCount
      - renderedVerticalIndices.count
    return Array(activeAttachmentIndices.prefix(max(0, available)))
  }

  private var renderedProviderIndices: [Int] {
    let available = maximumGlassShapeCount
      - 1
      - composerShapeCount
      - modelPanelShapeCount
      - renderedVerticalIndices.count
      - renderedAttachmentIndices.count
    // Integration intentionally leaves composer out of this host. The maximum
    // cross-switch frame is therefore exactly 17 shapes:
    // root + vertical(6) + attachment(3) + provider(6) + model panel.
    return Array(activeProviderIndices.prefix(max(0, available)))
  }

  private var explicitColorScheme: ColorScheme? {
    switch MitsuroLiquidGlassColorScheme(rawValue: props.colorScheme) ?? .auto {
    case .auto:
      return nil
    case .light:
      return .light
    case .dark:
      return .dark
    }
  }

  private var effectiveColorScheme: ColorScheme {
    explicitColorScheme ?? inheritedColorScheme
  }

  private var reduceTransparencyFill: Color {
    switch effectiveColorScheme {
    case .dark:
      return Color(red: 25.0 / 255.0, green: 24.0 / 255.0, blue: 29.0 / 255.0)
    case .light:
      return Color(red: 246.0 / 255.0, green: 243.0 / 255.0, blue: 238.0 / 255.0)
    @unknown default:
      return Color(red: 25.0 / 255.0, green: 24.0 / 255.0, blue: 29.0 / 255.0)
    }
  }

  private func verticalProgress(_ index: Int) -> CGFloat {
    let raw: Double
    switch index {
    case 0: raw = props.p0
    case 1: raw = props.p1
    case 2: raw = props.p2
    case 3: raw = props.p3
    case 4: raw = props.p4
    case 5: raw = props.p5
    default: return 0
    }
    return branchProgress(raw, open: props.open)
  }

  private func attachmentProgress(_ index: Int) -> CGFloat {
    let raw: Double
    switch index {
    case 0: raw = props.attachmentP0
    case 1: raw = props.attachmentP1
    case 2: raw = props.attachmentP2
    default: return 0
    }
    return branchProgress(raw, open: props.attachmentOpen)
  }

  private func providerProgress(_ index: Int) -> CGFloat {
    let raw: Double
    switch index {
    case 0: raw = props.q0
    case 1: raw = props.q1
    case 2: raw = props.q2
    case 3: raw = props.q3
    case 4: raw = props.q4
    case 5: raw = props.q5
    default: return 0
    }
    return branchProgress(raw, open: props.providerOpen)
  }

  private func providerX(_ index: Int) -> Double? {
    switch index {
    case 0: return props.providerX0
    case 1: return props.providerX1
    case 2: return props.providerX2
    case 3: return props.providerX3
    case 4: return props.providerX4
    case 5: return props.providerX5
    default: return nil
    }
  }

  private func providerY(_ index: Int) -> Double? {
    switch index {
    case 0: return props.providerY0
    case 1: return props.providerY1
    case 2: return props.providerY2
    case 3: return props.providerY3
    case 4: return props.providerY4
    case 5: return props.providerY5
    default: return nil
    }
  }

  private func providerScale(_ index: Int) -> Double {
    switch index {
    case 0: return props.providerScale0
    case 1: return props.providerScale1
    case 2: return props.providerScale2
    case 3: return props.providerScale3
    case 4: return props.providerScale4
    case 5: return props.providerScale5
    default: return 1
    }
  }

  private func providerRotation(_ index: Int) -> Double {
    switch index {
    case 0: return props.providerRotation0
    case 1: return props.providerRotation1
    case 2: return props.providerRotation2
    case 3: return props.providerRotation3
    case 4: return props.providerRotation4
    case 5: return props.providerRotation5
    default: return 0
    }
  }

  private func verticalCenter(index: Int, progress: CGFloat) -> CGPoint {
    let step = sanitized(props.verticalStep, fallback: 66, min: 1, max: 320)
    return CGPoint(
      x: rootX,
      y: rootY - CGFloat(index + 1) * step * progress
    )
  }

  private func sourceCenter(index: Int) -> CGPoint {
    guard index >= 0, index < 6 else {
      return CGPoint(x: rootX, y: rootY)
    }
    let step = sanitized(props.verticalStep, fallback: 66, min: 1, max: 320)
    return CGPoint(
      x: rootX,
      y: rootY - CGFloat(index + 1) * step
    )
  }

  private func branchProgress(_ raw: Double, open: Bool) -> CGFloat {
    guard raw.isFinite else {
      return open ? 1 : 0
    }
    // -1 is the explicit non-Reanimated sentinel. Preserve bounded spring
    // overshoot for real numeric props so native glass stays registered to the
    // React Native traveler through its final bounce instead of pinning early.
    if raw == -1 {
      return open ? 1 : 0
    }
    return CGFloat(min(1.25, max(-0.25, raw)))
  }

  private func boundedCount(_ value: Int, max maximum: Int) -> Int {
    min(maximum, max(0, value))
  }

  private func sanitized(
    _ value: Double,
    fallback: CGFloat,
    min minimum: CGFloat,
    max maximum: CGFloat
  ) -> CGFloat {
    guard value.isFinite else { return fallback }
    return Swift.min(maximum, Swift.max(minimum, CGFloat(value)))
  }

  private func sanitized(
    _ value: Double?,
    fallback: CGFloat,
    min minimum: CGFloat,
    max maximum: CGFloat
  ) -> CGFloat {
    guard let value else { return fallback }
    return sanitized(value, fallback: fallback, min: minimum, max: maximum)
  }

  private func interpolate(_ from: CGFloat, _ to: CGFloat, _ progress: CGFloat) -> CGFloat {
    from + (to - from) * progress
  }
}
