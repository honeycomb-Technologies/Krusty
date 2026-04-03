import type { ComponentProps, ComponentType, PropsWithChildren } from "react";
import { AccessoryWidgetBackground } from "@expo/ui/swift-ui";

type AccessoryBackgroundProps = ComponentProps<
  typeof AccessoryWidgetBackground
>;

const AccessoryBackgroundComponent =
  AccessoryWidgetBackground as unknown as ComponentType<
    PropsWithChildren<AccessoryBackgroundProps>
  >;

export function AccessoryBackground(
  props: PropsWithChildren<AccessoryBackgroundProps>,
) {
  return <AccessoryBackgroundComponent {...props} />;
}
