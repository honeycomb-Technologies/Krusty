import {
  GlassView,
  type GlassViewProps,
  isGlassEffectAPIAvailable,
  isLiquidGlassAvailable,
} from "expo-glass-effect";
import { type ComponentType, memo, useEffect, useState } from "react";
import {
  Platform,
  type StyleProp,
  StyleSheet,
  View,
  type ViewStyle,
} from "react-native";

import { useReduceTransparency } from "../../hooks/useReduceTransparency";
import { useThemeContext } from "../../hooks/useTheme";
import { BlurView } from "../../platform/blur";
import {
  resolveAdaptiveMaterialBlurIntensity,
  resolveAdaptiveMaterialMode,
  resolveAdaptiveMaterialOverlayColor,
  resolveLiquidGlassTintColor,
  type AdaptiveMaterialTone,
} from "./adaptiveMaterialPolicy";

const platform = Platform.OS;
const glassApiAvailable = isGlassEffectAPIAvailable();
const liquidGlassAvailable = isLiquidGlassAvailable();

// expo-glass-effect binds a native `borderRadius` prop (setBorderRadius on the
// effect view) that its public TS types do not declare. Shaping the effect
// natively keeps the glass sampling correct instead of relying on JS-side
// overflow clipping alone.
const NativeGlassView = GlassView as ComponentType<
  GlassViewProps & { borderRadius?: number }
>;

export type { AdaptiveMaterialTone } from "./adaptiveMaterialPolicy";

interface AdaptiveMaterialProps {
  style?: StyleProp<ViewStyle>;
  tone?: AdaptiveMaterialTone;
  blurIntensity?: number;
  borderRadius?: number;
  fallbackColor?: string;
  testID?: string;
  /**
   * Glass is presence, not camouflage. A closed or not-yet-arrived control
   * must not keep a GlassView in the tree — scale/opacity cannot hide it.
   */
  active?: boolean;
  /** iOS 26 press/glint. FABs only — do not put this on large chrome. */
  interactive?: boolean;
}

/**
 * Background-only material layer for floating Mitsuro chrome.
 *
 * The interactive control remains the parent Pressable/View. This layer owns
 * only presentation and therefore never captures taps or accessibility focus.
 */
function AdaptiveMaterialComponent({
  style,
  tone = "regular",
  blurIntensity,
  borderRadius,
  fallbackColor,
  testID,
  active = true,
  interactive = false,
}: AdaptiveMaterialProps) {
  const { theme } = useThemeContext();
  const reduceTransparency = useReduceTransparency();
  const [webBlurReady, setWebBlurReady] = useState(platform !== "web");
  const t = theme.colors;
  const materialMode = resolveAdaptiveMaterialMode({
    platform,
    reduceTransparency,
    glassApiAvailable,
    liquidGlassAvailable,
  });
  const materialSurfaces = {
    glassBackground: t.glass.background,
    glassBackgroundElevated: t.glass.backgroundElevated,
    glassBackgroundPressed: t.glass.backgroundPressed,
  };
  const resolvedBlurIntensity = blurIntensity ??
    resolveAdaptiveMaterialBlurIntensity(
      tone,
      theme.colors.glassBlur,
      theme.colors.glassBlurIntense,
    );
  const overlayColor = resolveAdaptiveMaterialOverlayColor(
    materialMode,
    tone,
    materialSurfaces,
  );
  const glassTintColor = resolveLiquidGlassTintColor(tone, materialSurfaces);
  const solidColor = fallbackColor ??
    (tone === "strong"
      ? t.surfaceStrong
      : tone === "elevated"
      ? t.surfaceElevated
      : t.surface);
  const materialStyle = [
    StyleSheet.absoluteFill,
    borderRadius === undefined ? null : { borderRadius },
    styles.clip,
    style,
  ];

  useEffect(() => {
    if (platform !== "web" || webBlurReady) return;

    // Expo BlurView relies on backdrop-filter on web. Mounting that filter in
    // the same paint as dynamic transcript content can leave mobile browsers
    // with an uncommitted/black backdrop tile until the first interaction.
    // Give the underlying frame one committed paint before promoting blur.
    let promoteFrame = 0;
    const paintFrame = requestAnimationFrame(() => {
      promoteFrame = requestAnimationFrame(() => setWebBlurReady(true));
    });

    return () => {
      cancelAnimationFrame(paintFrame);
      if (promoteFrame) cancelAnimationFrame(promoteFrame);
    };
  }, [webBlurReady]);

  if (!active) return null;

  if (materialMode === "liquid-glass") {
    // The GlassView is the surface. Never stack a scrim over the native
    // effect; the tone tint is the only readability layer it needs.
    return (
      <NativeGlassView
        testID={testID}
        colorScheme={theme.scheme}
        glassEffectStyle={tone === "strong" ? "regular" : "clear"}
        tintColor={glassTintColor}
        isInteractive={interactive}
        borderRadius={borderRadius}
        style={materialStyle}
      />
    );
  }

  if (materialMode === "blur") {
    return (
      <View testID={testID} style={materialStyle}>
        <BlurView
          intensity={webBlurReady ? resolvedBlurIntensity : 0}
          tint={theme.scheme === "dark"
            ? "systemChromeMaterialDark"
            : "systemChromeMaterialLight"}
          // Android has no UIGlassContainerEffect. Use Material chrome tints
          // here; dimezis backdrop blur stays off unless a BlurTarget is wired.
          blurMethod={platform === "android" ? "none" : undefined}
          style={StyleSheet.absoluteFill}
        />
        {overlayColor === undefined ? null : (
          <View
            style={[
              StyleSheet.absoluteFill,
              styles.ignorePointerEvents,
              { backgroundColor: overlayColor },
            ]}
          />
        )}
      </View>
    );
  }

  return (
    <View
      testID={testID}
      style={[materialStyle, { backgroundColor: solidColor }]}
    />
  );
}

export const AdaptiveMaterial = memo(AdaptiveMaterialComponent);
AdaptiveMaterial.displayName = "AdaptiveMaterial";

const styles = StyleSheet.create({
  clip: {
    overflow: "hidden",
    pointerEvents: "none",
  },
  ignorePointerEvents: {
    pointerEvents: "none",
  },
});
