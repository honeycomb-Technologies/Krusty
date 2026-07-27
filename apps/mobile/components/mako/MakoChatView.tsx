import type { MakoChatContext } from "./types";
import { MakoThreadSurface } from "./MakoThreadSurface";

interface MakoChatViewProps {
  chat: MakoChatContext;
}

export function MakoChatView({ chat }: MakoChatViewProps) {
  return <MakoThreadSurface chat={chat} />;
}
