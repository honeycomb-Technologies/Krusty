declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
  readTextFile(path: URL): Promise<string>;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

Deno.test("FAB material strength is stable before and after interaction", async () => {
  const accordion = await Deno.readTextFile(
    new URL("../components/chat/AccordionControls.tsx", import.meta.url),
  );
  const composer = await Deno.readTextFile(
    new URL("../components/chat/ChatBar.tsx", import.meta.url),
  );
  const modelPopover = await Deno.readTextFile(
    new URL("../components/chat/ChatBarModelPopover.tsx", import.meta.url),
  );
  const material = await Deno.readTextFile(
    new URL("../components/ui/AdaptiveMaterial.tsx", import.meta.url),
  );
  const header = await Deno.readTextFile(
    new URL("../components/navigation/MobileAppHeader.tsx", import.meta.url),
  );

  assert(
    !accordion.includes("style={styles.fabMaterialLayer}")
      && !composer.includes("style={styles.fabMaterialLayer}"),
    "FABs must not reintroduce an extra compositor layer",
  );
  assert(
    accordion.includes("AdaptiveMaterial")
      && !accordion.includes("theme.colors.thinking + '18'")
      && (accordion.match(/backgroundColor: gooeyFill\(theme\.scheme\)/g)?.length ?? 0) === 4,
    "every moving FAB must keep an opaque graphite cover without a selected-state fill",
  );
  assert(
    (composer.match(/<AdaptiveMaterial/g)?.length ?? 0) === 2
      && (composer.match(/backgroundColor: gooeyFill\(theme\.scheme\)/g)?.length ?? 0) >= 2
      && modelPopover.includes("backgroundColor: surfaceColor")
      && modelPopover.includes("<AdaptiveMaterial")
      && composer.includes("liquidGlassOnly")
      && modelPopover.includes("liquidGlassOnly"),
    "composer, FAB, and model endpoints must use iOS-only glass over identical graphite fallbacks",
  );
  assert(
    material.includes('const [webBlurReady, setWebBlurReady]')
      && material.includes('requestAnimationFrame(() => setWebBlurReady(true))')
      && material.includes("intensity={webBlurReady ? resolvedBlurIntensity : 0}"),
    "web blur must promote from zero after the underlying frame is committed",
  );
  assert(
    !material.includes('isolation: "isolate"')
      && !material.includes("zIndex: -1")
      && !accordion.includes("isolation: 'isolate'")
      && !composer.includes("isolation: 'isolate'"),
    "floating materials must see the real page backdrop instead of an isolated negative stack",
  );
  assert(
    material.includes("export const AdaptiveMaterial = memo(AdaptiveMaterialComponent)")
      && material.includes("liquidGlassOnly")
      && material.includes("respectMotionGate")
      && !composer.includes("backgroundColor: t.glass.backgroundElevated,"),
    "composer and FABs must expose the shared material instead of private tints",
  );
  const liquidGlassBranch = material.slice(
    material.indexOf('if (resolvedMaterialMode === "liquid-glass")'),
    material.indexOf('if (resolvedMaterialMode === "blur")'),
  );
  assert(
    liquidGlassBranch.includes("<NativeGlassView")
      && liquidGlassBranch.includes("borderRadius={borderRadius}")
      && !liquidGlassBranch.includes("overlayColor"),
    "liquid glass must render the bare native effect with its native corner radius and no stacked scrim",
  );
  assert(
    (material.match(/\{ backgroundColor: overlayColor \}/g)?.length ?? 0) === 1,
    "only the blur fallback may add a translucent glass-fill overlay",
  );
  assert(
    accordion.includes("styles.branchGooeyLayer")
      && accordion.includes("styles.branchGooeyViewport")
      && accordion.includes("position: 'relative'")
      && (composer.match(/position: 'relative'/g)?.length ?? 0) >= 2,
    "horizontal branches and composer surfaces must stay locally anchored",
  );
  assert(
    (header.match(/position: ["']relative["']/g)?.length ?? 0) >= 3
      && /position: ["']relative["']/.test(modelPopover),
    "every non-absolute chat-chrome material host must bound its backdrop to the component",
  );
  assert(
    accordion.includes("styles.pillOuterCompact")
      && accordion.includes("styles.pillTraveler")
      && accordion.includes("<FabGlyph progress={progress}>")
      && accordion.includes("orientation=\"horizontal\"")
      && !accordion.includes("styles.pointerBoxNone,\n        animatedStyle,"),
    "main and secondary FAB controls must travel on the shared silhouette",
  );
  assert(
    composer.includes("if (Platform.OS === 'web') return;")
      && composer.includes("const kActive = accordionOpen;"),
    "web glass controls must stay mounted after first use without staying visually active",
  );
});

Deno.test("endpoint glass never sits under an animated transform or opacity", async () => {
  const entrance = await Deno.readTextFile(
    new URL("../hooks/useEntranceAnimation.ts", import.meta.url),
  );
  const accordion = await Deno.readTextFile(
    new URL("../components/chat/AccordionControls.tsx", import.meta.url),
  );
  const composer = await Deno.readTextFile(
    new URL("../components/chat/ChatBar.tsx", import.meta.url),
  );
  const modelPopover = await Deno.readTextFile(
    new URL("../components/chat/ChatBarModelPopover.tsx", import.meta.url),
  );
  const layout = await Deno.readTextFile(
    new URL("../app/_layout.tsx", import.meta.url),
  );
  const screen = await Deno.readTextFile(
    new URL("../app/(tabs)/index.tsx", import.meta.url),
  );

  assert(
    !entrance.includes("opacity:")
      && !entrance.includes("Opacity")
      && entrance.includes("runOnJS(markSettled)")
      && entrance.includes("requestAnimationFrame(() => setMaterialSafe(true))")
      && screen.includes("entrance.settled ? null : entrance.bottomBarStyle")
      && screen.includes("safe={entrance.materialSafe}"),
    "startup must remove transforms, commit a frame, and only then enable endpoint glass",
  );
  assert(
    !accordion.includes("opacity: opacityProgress")
      && !accordion.includes("opacity: revealOpacityProgress")
      && !accordion.includes("opacity: progress.value")
      && !accordion.includes("opacity: revealProgress.value"),
    "accordion glass pills must not put Reanimated opacity on the GlassView host",
  );
  assert(
    accordion.includes("function FabGlyph")
      && accordion.includes("FAB_POUR_GLYPH_REVEAL_END")
      && accordion.includes("FAB_POUR_GLYPH_SETTLE_Y")
      && accordion.includes("<FabGlyph progress={revealProgress}>")
      && accordion.includes("<FabGlyph progress={progress}>"),
    "provider dock glyphs may fade on their own layer while the stable tile travels",
  );
  assert(
    accordion.includes("styles.mergeColumn")
      && accordion.includes("pillsMounted")
      && accordion.includes("FAB_POUR_CLOSE_MS")
      && accordion.includes("<FabGooeyLayer")
      && (accordion.match(/orientation="horizontal"/g)?.length ?? 0) === 2
      && accordion.includes("from './fabGooey'")
      && !accordion.includes("crystallizeOnSettle")
      && accordion.includes("AdaptiveMaterial")
      && accordion.includes("styles.graphiteCover")
      && accordion.includes("active={isOpen && materialActive}")
      && accordion.includes("materialCommitFrame = requestAnimationFrame(beginRetraction)")
      && !accordion.includes("GlassMergeCluster")
      && !accordion.includes("GlassContainer")
      && !accordion.includes("DROPLET_STRETCH")
      && !accordion.includes("DROPLET_SCALE")
      && !accordion.includes("[0.01, 1]")
      && !accordion.includes("[24 + index * 6, 0]"),
    "FAB branches must pour as graphite while native glass stays in fixed settled siblings",
  );
  assert(
    accordion.includes("const OPEN_STAGGER_MS = FAB_POUR_OPEN_STAGGER_MS")
      && accordion.includes("FAB_POUR_OPEN_STAGGER_MS")
      && accordion.includes("const CLOSE_STAGGER_MS = 46"),
    "pour stagger must be a readable cascade, not a simultaneous pop",
  );
  assert(
    composer.includes("agent={")
      && composer.includes("pillsMounted={accordionVisible}")
      && composer.includes("bottom: inputRowBottom")
      && composer.includes("pourCloseDurationMs")
      && composer.includes("onCloseComplete")
      && !composer.includes("agentPoolStyle")
      && !composer.includes("active={!accordionVisible}")
      && !composer.includes("640")
      && !composer.includes("<GlassContainer")
      && !composer.includes("LiquidMaterialCluster")
      && composer.includes("fixedMaterial.coverStyle")
      && composer.includes("active={fixedMaterial.materialActive}"),
    "the Agent and composer must use fixed glass endpoints with a graphite-only crossfade",
  );
  assert(
    !composer.includes("modelPopoverOpacity")
      && !composer.includes("opacity: modelPopoverOpacity")
      && composer.includes("surfaceColor={gooeyFill(theme.scheme)}")
      && accordion.includes("modelPopoverProgress: SharedValue<number>")
      && accordion.includes("isOpen: providerPourOpen")
      && accordion.includes("openDelayMs: 0")
      && accordion.includes("closeDelayMs: 0")
      && accordion.includes("progress: modelPopoverProgress")
      && composer.includes("modelPopoverProgress={modelPopoverScale}")
      && composer.includes("modelPopoverCoverOpacity={modelPopoverCoverOpacity}")
      && composer.includes("modelPopoverCoverStyle={modelPopoverCoverStyle}")
      && composer.includes("materialActive={modelRailOpen && modelPopoverMaterialActive}")
      && composer.includes("modelPopoverScale.value < 0.999")
      && !composer.includes("modelPopoverScale.value = withSpring")
      && !composer.includes("modelPopoverScale.value = withTiming")
      && !composer.includes("height: PILL + (modelPopoverHeight - PILL) * modelPopoverScale.value")
      && !composer.includes("borderRadius: RADIUS + (PILL / 2 - RADIUS) * (1 - modelPopoverScale.value)")
      && composer.includes("modelPopoverTravelDistance")
      && composer.includes("modelPopoverTravelDistance + MODEL_POPOVER_HIDE_OVERSCAN")
      && composer.includes("progress / FAB_POUR_GLYPH_REVEAL_END")
      && composer.includes("FAB_POUR_GLYPH_SETTLE_Y")
      && composer.includes("modelPopoverContentStyle={modelPopoverContentStyle}")
      && composer.includes("providerBranchCloseDeadlineRef")
      && composer.includes("attachmentBranchCloseDeadlineRef")
      && composer.includes("const remainingBranchCloseMs = Math.max")
      && composer.includes("requestAccordionCloseRef.current = requestAccordionClose"),
    "the complete model surface must pour from its trigger and side branches must return before the main stack closes",
  );
  const modelMaterialIndex = modelPopover.indexOf("<AdaptiveMaterial");
  const translatedModelPanelIndex = modelPopover.indexOf(
    "<Animated.View",
    modelMaterialIndex + 1,
  );
  const translatedModelPanel = modelPopover.slice(translatedModelPanelIndex);
  assert(
    modelMaterialIndex >= 0
      && translatedModelPanelIndex > modelMaterialIndex
      && modelPopover.includes("active={materialActive}")
      && modelPopover.includes("modelPopoverCoverStyle")
      && modelPopover.includes("liquidGlassOnly")
      && !translatedModelPanel.includes("<AdaptiveMaterial"),
    "the model GlassView must be a fixed clip sibling, never a child of the translated panel",
  );
  assert(
    !modelPopover.includes("backgroundElevated")
      && !modelPopover.includes("selected && { backgroundColor")
      && modelPopover.includes("pressed && { backgroundColor: backgroundPressed }")
      && modelPopover.includes('accessibilityState={{ selected }}')
      && modelPopover.includes('pointerEvents={interactive ? "box-none" : "none"}')
      && modelPopover.includes("modelPopoverContentStyle")
      && modelPopover.includes("height: modelPopoverHeight")
      && modelPopover.includes("removeClippedSubviews={false}")
      && !modelPopover.includes("boxShadow")
      && composer.includes("interactive={modelRailOpen}")
      && modelPopover.includes("{selected && ("),
    "model selection must use only its checkmark and accessibility state, without a persistent row card",
  );
  assert(
    layout.includes("animation: 'none'"),
    "the root stack must not fade-in screens that host liquid glass",
  );
});

Deno.test("chat chrome icons stay on the thinking violet", async () => {
  const accordion = await Deno.readTextFile(
    new URL("../components/chat/AccordionControls.tsx", import.meta.url),
  );
  const header = await Deno.readTextFile(
    new URL("../components/navigation/MobileAppHeader.tsx", import.meta.url),
  );
  const composer = await Deno.readTextFile(
    new URL("../components/chat/ChatBar.tsx", import.meta.url),
  );

  assert(
    accordion.includes("const fabAccent = t.thinking") &&
      !accordion.includes("color={t.mutedForeground}") &&
      !accordion.includes("color={t.success}"),
    "accordion glyphs must stay on the thinking violet instead of gray or status green",
  );
  assert(
    header.includes("color={t.thinking}") &&
      !header.includes("color={t.mutedForeground}") &&
      !header.includes("color={active ? t.foreground : t.mutedForeground}"),
    "header chrome icons must stay on the thinking violet",
  );
  assert(
    composer.includes("color={t.thinking}") &&
      composer.includes("const kColor = t.thinking"),
    "composer chrome glyphs must stay on the thinking violet",
  );
});

Deno.test("chat chrome never wraps FABs in GlassContainer", async () => {
  const composer = await Deno.readTextFile(
    new URL("../components/chat/ChatBar.tsx", import.meta.url),
  );
  const accordion = await Deno.readTextFile(
    new URL("../components/chat/AccordionControls.tsx", import.meta.url),
  );
  const material = await Deno.readTextFile(
    new URL("../components/ui/AdaptiveMaterial.tsx", import.meta.url),
  );

  assert(
    !composer.includes("<GlassContainer")
      && !accordion.includes("<GlassContainer")
      && !composer.includes("from \"expo-glass-effect\"")
      && !accordion.includes("from \"expo-glass-effect\"")
      && !composer.includes("LiquidMaterialCluster")
      && !accordion.includes("GlassMergeCluster")
      && accordion.includes("style={styles.mergeColumn}")
      && accordion.includes("{agent}")
      && !accordion.includes("AgentGlassMorphColumn")
      && !accordion.includes("glassEffectId"),
    "UIKit GlassContainer and SwiftUI Host columns are out; pour uses a Skia silhouette",
  );
  const columnStart = accordion.indexOf("style={styles.mergeColumn}");
  const columnEnd = accordion.indexOf("</GestureDetector>", columnStart);
  const columnBody = accordion.slice(columnStart, columnEnd);
  const docks = accordion.slice(
    accordion.indexOf("{pillsMounted ? ("),
    columnStart,
  );
  assert(
    columnStart >= 0
      && columnEnd > columnStart
      && columnBody.includes("<AccordionPill")
      && columnBody.includes("{agent}")
      && !columnBody.includes("ProviderDockPill")
      && !columnBody.includes("InlineActionPill")
      && !columnBody.includes("DesktopFilterPill")
      && docks.includes("ProviderDockPill")
      && docks.includes("InlineActionPill")
      && docks.includes("DesktopFilterPill"),
    "provider and attachment branches stay structurally separate from the Agent column",
  );
  assert(
    material.includes('blurMethod={platform === "android" ? "none" : undefined}'),
    "Android chrome must use Material blur fills, not an untargeted dimezis backdrop",
  );
  assert(
    material.includes("if (!active) return null")
      && accordion.includes("function useDeferredPresence")
      && accordion.includes("function usePourMotion")
      && accordion.includes("runOnJS(finishClose)")
      && accordion.includes("providerDockMounted")
      && accordion.includes("attachActionsMounted")
      && accordion.includes("desktopFiltersMounted")
      && accordion.includes("providerProgresses")
      && accordion.includes("attachProgresses")
      && accordion.includes("materialActive")
      && accordion.includes("AdaptiveMaterial")
      && accordion.includes("materialAllowed: false"),
    "secondary branches must retain deferred presence, fixed endpoints, and a graphite-only scroll rail",
  );
});

Deno.test("liquid glass stays on chat chrome, not drawers or sheets", async () => {
  const solidSurfaces = [
    "../components/sheets/AppBottomSheet.tsx",
    "../components/SettingsModal.tsx",
    "../components/ReportsViewer.tsx",
    "../components/DirectoryPicker.tsx",
    "../components/chat/ImagePreviewModal.tsx",
    "../components/ui/GlassCard.tsx",
    "../components/chat/SessionDrawer.tsx",
    "../components/ToolboxPanel.tsx",
    "../components/settings/sections.tsx",
  ];
  const sources = await Promise.all(
    solidSurfaces.map((file) =>
      Deno.readTextFile(new URL(file, import.meta.url))
    ),
  );
  const leaked = solidSurfaces.filter((file, index) =>
    sources[index].includes("AdaptiveMaterial")
  );

  assert(
    leaked.length === 0,
    `AdaptiveMaterial is chat chrome only; unexpected uses: ${leaked.join(", ")}`,
  );
  assert(
    sources[0].includes("backgroundColor: t.background"),
    "session and toolbox drawers must use a solid theme fill",
  );
  assert(
    sources[5].includes(
      "backgroundColor: elevated ? t.surfaceElevated : t.surface",
    ),
    "settings and list cards must stay solid theme surfaces",
  );
});
