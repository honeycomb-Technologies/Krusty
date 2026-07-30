import { createWidget } from "expo-widgets";
import { Text, VStack, HStack, Spacer, Image } from "@expo/ui/swift-ui";
import {
  font,
  foregroundStyle,
  padding,
  background,
  opacity,
  lineLimit,
  widgetURL,
} from "@expo/ui/swift-ui/modifiers";
import type { WidgetEnvironment } from "expo-widgets";
import { AccessoryBackground } from "./AccessoryBackground";

interface ChatProps {
  hasActiveSession: boolean;
  sessionTitle: string;
  lastMessage: string;
  model: string;
  isStreaming: boolean;
  tokenCount: number;
  serverConnected: boolean;
}

function ChatWidget(props: ChatProps, env: WidgetEnvironment) {
  "widget";

  const {
    hasActiveSession,
    sessionTitle,
    lastMessage,
    model,
    isStreaming,
    tokenCount,
    serverConnected,
  } = props;
  const family = env.widgetFamily;

  if (family === "accessoryCircular") {
    return (
      <AccessoryBackground modifiers={[widgetURL("mitsuro://chat")]}>
        <VStack alignment="center">
          <Image
            systemName={
              isStreaming ? "ellipsis.bubble.fill" : "bubble.left.fill"
            }
            modifiers={[font({ size: 16 })]}
          />
          <Text modifiers={[font({ size: 8 })]}>
            {isStreaming ? "Live" : "Chat"}
          </Text>
        </VStack>
      </AccessoryBackground>
    );
  }

  if (family === "accessoryRectangular") {
    return (
      <AccessoryBackground modifiers={[widgetURL("mitsuro://chat")]}>
        <VStack alignment="leading">
          <HStack>
            <Image
              systemName="bubble.left.fill"
              modifiers={[font({ size: 10 })]}
            />
            <Text modifiers={[font({ size: 12, weight: "semibold" })]}>
              {hasActiveSession ? sessionTitle : "Mitsuro"}
            </Text>
          </HStack>
          <Text modifiers={[font({ size: 11 }), lineLimit(2), opacity(0.7)]}>
            {hasActiveSession
              ? isStreaming
                ? "Streaming response..."
                : lastMessage || "No messages yet"
              : "Tap to start a new chat"}
          </Text>
        </VStack>
      </AccessoryBackground>
    );
  }

  if (family === "systemSmall") {
    return (
      <VStack
        alignment="leading"
        modifiers={[
          padding({ all: 14 }),
          background("#0e0e11"),
          foregroundStyle("#e8e5ea"),
          widgetURL("mitsuro://chat"),
        ]}
      >
        <HStack>
          <Image
            systemName={serverConnected ? "circle.fill" : "circle"}
            modifiers={[
              foregroundStyle(serverConnected ? "#7f9a86" : "#b06f73"),
              font({ size: 8 }),
            ]}
          />
          <Text modifiers={[font({ size: 11 }), opacity(0.5)]}>Mitsuro</Text>
          <Spacer />
          {isStreaming && (
            <Image
              systemName="ellipsis.bubble.fill"
              modifiers={[foregroundStyle("#b89a61"), font({ size: 12 })]}
            />
          )}
        </HStack>

        <Spacer />

        {hasActiveSession ? (
          <VStack alignment="leading" spacing={4}>
            <Text
              modifiers={[font({ size: 13, weight: "semibold" }), lineLimit(1)]}
            >
              {sessionTitle}
            </Text>
            <Text modifiers={[font({ size: 11 }), lineLimit(3), opacity(0.7)]}>
              {isStreaming
                ? "Generating response..."
                : lastMessage || "No messages"}
            </Text>
          </VStack>
        ) : (
          <VStack alignment="center" spacing={6}>
            <Image
              systemName="plus.bubble.fill"
              modifiers={[foregroundStyle("#b89a61"), font({ size: 24 })]}
            />
            <Text modifiers={[font({ size: 12 }), opacity(0.6)]}>New Chat</Text>
          </VStack>
        )}

        <Spacer />

        {hasActiveSession && (
          <HStack>
            <Text modifiers={[font({ size: 9 }), opacity(0.4)]}>{model}</Text>
            <Spacer />
            <Text modifiers={[font({ size: 9 }), opacity(0.4)]}>
              {tokenCount.toLocaleString()} tokens
            </Text>
          </HStack>
        )}
      </VStack>
    );
  }

  return (
    <VStack
      alignment="leading"
      modifiers={[
        padding({ all: 14 }),
        background("#0e0e11"),
        foregroundStyle("#e8e5ea"),
        widgetURL("mitsuro://chat"),
      ]}
    >
      <HStack>
        <Image
          systemName={serverConnected ? "circle.fill" : "circle"}
          modifiers={[
            foregroundStyle(serverConnected ? "#7f9a86" : "#b06f73"),
            font({ size: 8 }),
          ]}
        />
        <Text modifiers={[font({ size: 13, weight: "bold" })]}>
          Mitsuro Chat
        </Text>
        <Spacer />
        {isStreaming && (
          <HStack spacing={4}>
            <Image
              systemName="ellipsis.bubble.fill"
              modifiers={[foregroundStyle("#b89a61"), font({ size: 12 })]}
            />
            <Text
              modifiers={[font({ size: 11 }), foregroundStyle("#b89a61")]}
            >
              Streaming
            </Text>
          </HStack>
        )}
      </HStack>

      <Spacer />

      {hasActiveSession ? (
        <VStack alignment="leading" spacing={4}>
          <Text
            modifiers={[font({ size: 14, weight: "semibold" }), lineLimit(1)]}
          >
            {sessionTitle}
          </Text>
          <Text modifiers={[font({ size: 12 }), lineLimit(2), opacity(0.7)]}>
            {isStreaming
              ? "Generating response..."
              : lastMessage || "No messages yet"}
          </Text>
        </VStack>
      ) : (
        <HStack alignment="center" spacing={8}>
          <Image
            systemName="plus.bubble.fill"
            modifiers={[foregroundStyle("#b89a61"), font({ size: 28 })]}
          />
          <VStack alignment="leading">
            <Text modifiers={[font({ size: 14, weight: "medium" })]}>
              Start a New Chat
            </Text>
            <Text modifiers={[font({ size: 11 }), opacity(0.5)]}>
              Tap to open Mitsuro
            </Text>
          </VStack>
        </HStack>
      )}

      <Spacer />

      <HStack>
        <Text modifiers={[font({ size: 10 }), opacity(0.4)]}>
          {hasActiveSession
            ? model
            : serverConnected
              ? "Connected"
              : "Disconnected"}
        </Text>
        <Spacer />
        {hasActiveSession && (
          <Text modifiers={[font({ size: 10 }), opacity(0.4)]}>
            {tokenCount.toLocaleString()} tokens
          </Text>
        )}
      </HStack>
    </VStack>
  );
}

export default createWidget("ChatWidget", ChatWidget);
