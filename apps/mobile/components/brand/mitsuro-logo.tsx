import { View, type StyleProp, type ViewStyle } from "react-native";
import Animated, { FadeIn } from "react-native-reanimated";

import { MitsuroTraceMark } from "./mitsuro-mark";
import { MitsuroWordmark } from "./mitsuro-wordmark";

interface MitsuroLogoProps {
  color?: string;
  markFill?: string;
  markSize?: number;
  wordmarkWidth?: number;
  playKey?: string | number;
  style?: StyleProp<ViewStyle>;
}

export function MitsuroLogo({
  color = "#e8e5ea",
  markFill = "rgba(232, 229, 234, 0.055)",
  markSize = 84,
  wordmarkWidth = 216,
  playKey,
  style,
}: MitsuroLogoProps) {
  return (
    <View
      style={[
        { alignItems: "center", justifyContent: "center", gap: 14 },
        style,
      ]}
      accessibilityRole="image"
      accessibilityLabel="Mitsuro"
    >
      <MitsuroTraceMark
        size={markSize}
        color={color}
        fill={markFill}
        playKey={playKey}
      />
      <Animated.View entering={FadeIn.duration(260).delay(180)}>
        <MitsuroWordmark width={wordmarkWidth} color={color} />
      </Animated.View>
    </View>
  );
}
