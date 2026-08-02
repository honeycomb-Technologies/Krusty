import type { HiveChatContext } from "./types";
import { HiveThreadSurface } from "./HiveThreadSurface";

interface HiveChatViewProps {
  chat: HiveChatContext;
}

export function HiveChatView({ chat }: HiveChatViewProps) {
  return <HiveThreadSurface chat={chat} />;
}
