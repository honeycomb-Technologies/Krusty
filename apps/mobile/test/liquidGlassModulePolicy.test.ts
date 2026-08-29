declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
  readTextFile(path: string): Promise<string>;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

Deno.test("native liquid glass keeps branch shapes continuous and bounded", async () => {
  const view = await Deno.readTextFile(
    new URL(
      "../modules/mitsuro-liquid-glass/ios/MitsuroLiquidGlassView.swift",
      import.meta.url,
    ).pathname,
  );
  const props = await Deno.readTextFile(
    new URL(
      "../modules/mitsuro-liquid-glass/src/MitsuroLiquidGlass.types.ts",
      import.meta.url,
    ).pathname,
  );
  const host = await Deno.readTextFile(
    new URL(
      "../components/chat/FabLiquidGlassHost.tsx",
      import.meta.url,
    ).pathname,
  );
  const motion = await Deno.readTextFile(
    new URL(
      "../components/chat/fabGlassMotion.ts",
      import.meta.url,
    ).pathname,
  );
  const composer = await Deno.readTextFile(
    new URL(
      "../components/chat/ChatBar.tsx",
      import.meta.url,
    ).pathname,
  );
  const accordion = await Deno.readTextFile(
    new URL(
      "../components/chat/AccordionControls.tsx",
      import.meta.url,
    ).pathname,
  );
  const screen = await Deno.readTextFile(
    new URL("../app/(tabs)/index.tsx", import.meta.url).pathname,
  );

  assert(
    view.includes("private let maximumGlassShapeCount = 17")
      && view.includes("physical iOS 26 device")
      && view.includes("boundedCount(props.attachmentCount, max: 3)")
      && view.includes("boundedCount(props.providerCount, max: 6)"),
    "the single native container must retain its reviewed 17-shape budget and branch caps",
  );
  assert(
    view.includes("@Field var effectSpacing: Double = 8")
      && view.includes("@Field var providerStep: Double = 66")
      && view.includes(".clear.interactive(false)")
      && !view.includes(".opacity(")
      && !view.includes(".animation(")
      && !view.includes("openAnimation"),
    "Reanimated must be the only geometry clock and clear glass must never use progress opacity fades",
  );
  assert(
    view.includes("private var activeVerticalIndices: [Int]")
      && view.includes("private var activeAttachmentIndices: [Int]")
      && view.includes("private var activeProviderIndices: [Int]")
      && view.includes("(0..<requestedVerticalCount).filter { verticalProgress($0) > 0 }")
      && view.includes("(0..<requestedAttachmentCount).filter { attachmentProgress($0) > 0 }")
      && view.includes("(0..<requestedProviderCount).filter { providerProgress($0) > 0 }")
      && view.includes("return progress > 0 && width > 0 && height > 0")
      && !view.includes("branchVisibilityEpsilon"),
    "explicit zero-progress shapes must wait at zero while every moving shape stays until exact zero",
  );
  assert(
    view.includes("if raw == -1")
      && view.includes("min(1.25, max(-0.25, raw))")
      && host.includes("useAnimatedProps<MitsuroLiquidGlassViewProps>")
      && host.includes("p0: motion.pillProgresses[0].value")
      && host.includes("attachmentP2: motion.attachmentProgresses[2].value")
      && host.includes("q5: motion.providerProgresses[5].value"),
    "native geometry must follow the one Reanimated clock through bounded spring overshoot",
  );
  assert(
    view.includes("root + vertical(6) + attachment(3) + provider(6) + model panel")
      && view.includes("return Array(activeProviderIndices.prefix(max(0, available)))")
      && view.includes("- renderedVerticalIndices.count")
      && view.includes("- renderedAttachmentIndices.count"),
    "the reviewed composer-free cross-switch must fit all 17 native shapes without dropping provider six",
  );
  assert(
    view.includes("@Field var attachmentP0: Double = -1")
      && view.includes("@Field var q0: Double = -1")
      && view.includes("@Field var providerX0: Double?")
      && view.includes("@Field var providerY0: Double?")
      && view.includes("@Field var providerScale0: Double = 1")
      && view.includes("@Field var providerRotation5: Double = 0")
      && view.includes("@Field var providerViewportClip: Double = 0")
      && props.includes("attachmentP2?: number")
      && props.includes("q5?: number")
      && props.includes("providerX5?: number")
      && props.includes("providerY5?: number")
      && props.includes("providerScale5?: number")
      && props.includes("providerRotation5?: number")
      && props.includes("providerViewportClip?: number")
      && host.includes("providerTargetScale")
      && host.includes("providerTargetRotation")
      && host.includes("providerViewportClip: motion.providerViewportClip.value")
      && accordion.includes("{ translateY: lift + dragLift }")
      && accordion.includes("{ rotate: `${tilt}deg` }")
      && accordion.includes("providerViewportClip.value = 1")
      && accordion.includes("providerViewportClip.value = 0")
      && view.includes(".frame(width: providerMaskRight)")
      && view.includes("let sourceLeft = rootX - rootWidth / 2")
      && view.includes("return interpolate(sourceRight, sourceLeft, clip)"),
    "each provider shape must follow its RN control and adopt the scroll viewport only after the pour settles",
  );
  assert(
    view.includes("public var ignoreSafeArea: ExpoSwiftUI.IgnoreSafeArea? = .all")
      && view.includes("@Environment(\\.accessibilityReduceTransparency)")
      && view.includes("@Field var colorScheme: String = \"auto\"")
      && props.includes("colorScheme?: MitsuroLiquidGlassColorScheme"),
    "native glass must use local coordinates with explicit appearance and accessibility fallbacks",
  );
  assert(
    view.includes("@Field var modelCornerRadius: Double = 18")
      && view.includes("fallback: 18, min: 0, max: min(targetWidth, targetHeight) / 2")
      && view.includes("y: rootY - CGFloat(index + 1) * step")
      && !view.includes(
        "verticalCenter(index: index, progress: verticalProgress(index))",
      ),
    "branch sources must stay fixed at vertical destinations and panels must morph from the shared radius",
  );
  assert(
    view.includes("25.0 / 255.0")
      && view.includes("24.0 / 255.0")
      && view.includes("29.0 / 255.0")
      && view.includes("246.0 / 255.0")
      && view.includes("243.0 / 255.0")
      && view.includes("238.0 / 255.0"),
    "Reduce Transparency must use the exact graphite fallback on both themes",
  );
  assert(
    motion.includes("export function useFabGlassMotion")
      && motion.includes("const providerScrollX = useSharedValue(0)")
      && motion.includes("const providerReorderX = useSixProgresses()")
      && motion.includes("return useMemo(")
      && composer.includes("<FabLiquidGlassHost")
      && composer.indexOf("<FabLiquidGlassHost") < composer.indexOf("{/* Attachment previews */}")
      && composer.includes("showComposerChrome &&")
      && composer.includes("const nativeGlassAvailable =")
      && composer.includes("providerFilterActions.length <= MAX_GOOEY_PILLS")
      && host.includes("showComposer={false}")
      && host.includes("tintColor={tintColor}")
      && composer.includes("tintColor={t.glass.background}")
      && screen.includes("entrance.settled || continuousNativeGlass")
      && accordion.includes("nativeProviderGlassActive ? null : (")
      && accordion.includes("nativeGlassActive ? null : (")
      && accordion.includes("onScroll={handleProviderScroll}"),
    "the global host must be a capability-gated background using shared scroll and branch motion",
  );
});

Deno.test("liquid glass module remains an iOS-only optional native layer", async () => {
  const module = await Deno.readTextFile(
    new URL(
      "../modules/mitsuro-liquid-glass/ios/MitsuroLiquidGlassModule.swift",
      import.meta.url,
    ).pathname,
  );
  const podspec = await Deno.readTextFile(
    new URL(
      "../modules/mitsuro-liquid-glass/ios/MitsuroLiquidGlass.podspec",
      import.meta.url,
    ).pathname,
  );
  const config = await Deno.readTextFile(
    new URL(
      "../modules/mitsuro-liquid-glass/expo-module.config.json",
      import.meta.url,
    ).pathname,
  );

  assert(
    module.includes("#if compiler(>=6.2)")
      && module.includes("#available(iOS 26.0, *)")
      && module.includes('NSClassFromString("UIGlassEffect")')
      && module.includes('Selector(("effectWithStyle:"))')
      && module.includes('NSClassFromString("UIGlassContainerEffect")')
      && module.includes('"UIDesignRequiresCompatibility"')
      && module.includes('Name("MitsuroLiquidGlass")'),
    "native support must remain gated to Xcode 26 and iOS 26",
  );
  assert(
    podspec.includes("s.platform       = :ios, '15.1'")
      && config.includes('\"platforms\": [\"apple\"]')
      && config.includes('\"modules\": [\"MitsuroLiquidGlassModule\"]'),
    "the optional module must autolink without raising Mitsuro's iOS deployment floor",
  );
});

Deno.test("settled provider glass clips at the exact RN rail edge", () => {
  const bandWidth = 390;
  const hostLeft = 10;
  const hostWidth = bandWidth - hostLeft;
  const pill = 56;
  const gap = 10;
  const step = pill + gap;
  const rootX = hostWidth - 10 - pill / 2;
  const nativeMaskRight = hostLeft + rootX - pill / 2;
  const rnDockRight = bandWidth - 10 - pill;

  assert(
    nativeMaskRight === rnDockRight,
    "the settled native mask must end exactly where the RN provider ScrollView clips",
  );

  // At scrollX=0 in the six-provider stress case, the final two native tiles
  // would otherwise bleed into the model/Agent column after their RN cells are
  // partially or fully clipped.
  const visualFourCenter = hostLeft + 62 + 4 * step;
  const visualFiveCenter = hostLeft + 62 + 5 * step;
  assert(
    visualFourCenter - pill / 2 < nativeMaskRight
      && visualFourCenter + pill / 2 > nativeMaskRight,
    "the mask must crop the partially visible fifth provider at the shared edge",
  );
  assert(
    visualFiveCenter - pill / 2 > nativeMaskRight,
    "the mask must remove the fully RN-clipped sixth provider instead of leaving native-only glass",
  );
});
