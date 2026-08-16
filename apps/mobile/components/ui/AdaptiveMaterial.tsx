import {
  GlassView,
  isGlassEffectAPIAvailable,
  isLiquidGlassAvailable,
} from "expo-glass-effect";
import { memo, useEffect, useState } from "react";
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

export type { AdaptiveMaterialTone } from "./adaptiveMaterialPolicy";

interface AdaptiveMaterialProps {
  style?: StyleProp<ViewStyle>;
  tone?: AdaptiveMaterialTone;
  blurIntensity?: number;
  borderRadius?: number;
  fallbackColor?: string;
  testID?: string;
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
    surfaceOverlaySubtle: t.surfaceOverlaySubtle,
    surfaceOverlay: t.surfaceOverlay,
    surfaceOverlayElevated: t.surfaceOverlayElevated,
  };
  const resolvedBlurIntensity = blurIntensity ??
    resolveAdaptiveMaterialBlurIntensity(
      tone,
      theme.colors.glassBlur,
      theme.colors.glassBlurIntense,
    );
  const overlayColor = resolveAdaptiveMaterialOverlayColor(
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
    platform === "web" ? styles.webBackgroundLayer : null,
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

  if (materialMode === "liquid-glass") {
    return (
      <GlassView
        testID={testID}
        colorScheme={theme.scheme}
        glassEffectStyle={tone === "subtle" ? "clear" : "regular"}
        tintColor={glassTintColor}
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
          style={StyleSheet.absoluteFill}
        />
        <View
          style={[
            StyleSheet.absoluteFill,
            styles.ignorePointerEvents,
            { backgroundColor: overlayColor },
          ]}
        />
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
  // RN Web gives positioned siblings the same default stack level. Keep the
  // backdrop strictly behind icons/text so Chrome cannot include foreground
  // controls in a later backdrop-filter rasterization pass.
  webBackgroundLayer: {
    isolation: "isolate",
    zIndex: -1,
  },
  clip: {
    overflow: "hidden",
    pointerEvents: "none",
  },
  ignorePointerEvents: {
    pointerEvents: "none",
  },
});
