import { createLiveActivity } from "expo-widgets";
import { Button, HStack, Image, Spacer, Text, VStack } from "@expo/ui/swift-ui";
import {
  aspectRatio,
  clipped,
  cornerRadius,
  font,
  foregroundStyle,
  frame,
  lineLimit,
  opacity,
  padding,
  resizable,
} from "@expo/ui/swift-ui/modifiers";
import type { LiveActivityComponent } from "expo-widgets";

export interface ChatStreamProps {
  chatTitle: string;
  status: "working" | "needs_input" | "completed";
  elapsedSeconds: number;
  toolCount: number;
  filesAdded: number;
  filesRemoved: number;
  toolApprovalId?: string;
  toolApprovalName?: string;
  toolApprovalSessionId?: string;
}

const ChatStreamActivity: LiveActivityComponent<ChatStreamProps> = (props) => {
  "widget";

  // Expo serializes only this function body into the widget extension. Keep
  // every runtime value used by the layout inside this boundary.
  // A compact copy of apps/mobile/assets/icons/source/krusty-home-master.png.
  // Embedding the official mark makes it available inside the widget extension,
  // where the main React Native asset bundle is not guaranteed to be readable.
  const KRUSTY_LOGO =
  "data:image/png;base64,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";

  function formatElapsed(seconds: number): string {
    const minutes = Math.floor(Math.max(0, seconds) / 60);
    const remainder = Math.max(0, seconds) % 60;
    return `${minutes}:${String(remainder).padStart(2, "0")}`;
  }

  const {
    chatTitle,
    status,
    elapsedSeconds,
    toolCount,
    filesAdded,
    filesRemoved,
    toolApprovalId,
    toolApprovalName,
    toolApprovalSessionId,
  } = props;
  const statusColor =
    status === "needs_input" ? "#ef4444" : status === "completed" ? "#22c55e" : "#ff6b35";
  const statusLabel =
    status === "needs_input" ? "Needs input" : status === "completed" ? "Completed" : "Working";
  const elapsed = formatElapsed(elapsedSeconds);
  const approvalTarget =
    toolApprovalId && toolApprovalSessionId
      ? `${encodeURIComponent(toolApprovalSessionId)}:${encodeURIComponent(toolApprovalId)}`
      : null;

  const logo = (
    <Image
      uiImage={KRUSTY_LOGO}
      modifiers={[
        resizable(),
        aspectRatio({ ratio: 1, contentMode: "fit" }),
        frame({ width: 30, height: 30 }),
        cornerRadius(9),
        clipped(),
      ]}
    />
  );
  const compactLogo = (
    <Image
      uiImage={KRUSTY_LOGO}
      modifiers={[
        resizable(),
        aspectRatio({ ratio: 1, contentMode: "fit" }),
        frame({ width: 18, height: 18 }),
        cornerRadius(5),
        clipped(),
      ]}
    />
  );
  const metrics = (
    <HStack spacing={14}>
      <HStack spacing={5}>
        <Image systemName="wrench" modifiers={[foregroundStyle("#60a5fa"), font({ size: 11 })]} />
        <Text modifiers={[font({ size: 11 }), opacity(0.78)]}>{toolCount} tools</Text>
      </HStack>
      <HStack spacing={5}>
        <Image systemName="doc.text.magnifyingglass" modifiers={[foregroundStyle("#60a5fa"), font({ size: 11 })]} />
        <Text modifiers={[font({ size: 11, design: "monospaced" }), foregroundStyle("#22c55e")]}>+{filesAdded}</Text>
        <Text modifiers={[font({ size: 11, design: "monospaced" }), foregroundStyle("#ef4444")]}>−{filesRemoved}</Text>
      </HStack>
    </HStack>
  );

  return {
    banner: (
      <VStack alignment="leading" spacing={9} modifiers={[padding({ all: 14 })]}>
        <HStack spacing={9}>
          {logo}
          <Text modifiers={[font({ size: 14, weight: "semibold" }), lineLimit(1)]}>
            {chatTitle || "Krusty session"}
          </Text>
          <Spacer />
          <VStack alignment="trailing" spacing={2}>
            <Text modifiers={[font({ size: 17, weight: "medium", design: "monospaced" }), foregroundStyle(statusColor)]}>
              {elapsed}
            </Text>
            <HStack spacing={4}>
              <Image systemName="circle.fill" modifiers={[foregroundStyle(statusColor), font({ size: 6 })]} />
              <Text modifiers={[font({ size: 9, weight: "semibold" }), foregroundStyle(statusColor)]}>{statusLabel}</Text>
            </HStack>
          </VStack>
        </HStack>
        <HStack>
          {metrics}
          <Spacer />
          <Image systemName="chevron.right" modifiers={[foregroundStyle("#60a5fa"), font({ size: 11 })]} />
        </HStack>
        {status === "needs_input" && approvalTarget && (
          <VStack spacing={6}>
            <Text modifiers={[font({ size: 11 }), opacity(0.7), lineLimit(1)]}>
              {toolApprovalName || "Tool"} needs permission
            </Text>
            <HStack spacing={10}>
              <Button target={`deny:${approvalTarget}`} role="destructive" label="Deny" modifiers={[frame({ maxWidth: 999 })]} />
              <Button target={`approve:${approvalTarget}`} label="Approve" modifiers={[frame({ maxWidth: 999 })]} />
            </HStack>
          </VStack>
        )}
      </VStack>
    ),
    compactLeading: (
      <HStack spacing={6}>
        {compactLogo}
        <HStack spacing={3}>
          <Text modifiers={[font({ size: 9, design: "monospaced" }), foregroundStyle("#22c55e")]}>+{filesAdded}</Text>
          <Text modifiers={[font({ size: 9, design: "monospaced" }), foregroundStyle("#ef4444")]}>−{filesRemoved}</Text>
        </HStack>
      </HStack>
    ),
    compactTrailing: (
      <Text modifiers={[font({ size: 11, weight: "medium", design: "monospaced" }), foregroundStyle(statusColor)]}>
        {elapsed}
      </Text>
    ),
    minimal: compactLogo,
    expandedLeading: logo,
    expandedCenter: (
      <Text modifiers={[font({ size: 13, weight: "semibold" }), lineLimit(1)]}>
        {chatTitle || "Krusty session"}
      </Text>
    ),
    expandedTrailing: (
      <VStack alignment="trailing" spacing={2}>
        <Text modifiers={[font({ size: 12, weight: "medium", design: "monospaced" }), foregroundStyle(statusColor)]}>
          {elapsed}
        </Text>
        <Text modifiers={[font({ size: 9, weight: "semibold" }), foregroundStyle(statusColor)]}>{statusLabel}</Text>
      </VStack>
    ),
    expandedBottom: (
      <VStack alignment="leading" spacing={8}>
        {metrics}
        {status === "needs_input" && approvalTarget && (
          <HStack spacing={8}>
            <Button target={`deny:${approvalTarget}`} role="destructive" label="Deny" modifiers={[frame({ maxWidth: 999 })]} />
            <Button target={`approve:${approvalTarget}`} label="Approve" modifiers={[frame({ maxWidth: 999 })]} />
          </HStack>
        )}
      </VStack>
    ),
  };
};

export default createLiveActivity("ChatStreamActivity", ChatStreamActivity);
