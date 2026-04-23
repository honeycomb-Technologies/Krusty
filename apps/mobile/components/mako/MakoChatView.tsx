import type { MakoChatContext } from "./types";
import { MakoThreadSurface } from "./MakoThreadSurface";

interface MakoChatViewProps {
  chat: MakoChatContext;
}

export function MakoChatView({ chat }: MakoChatViewProps) {
  return (
    <MakoThreadSurface
      chat={chat}
      emptyTitle="Start a Mako chat"
      emptyBody="Send a message to steer Mako, ask for status, or start a new run."
    />
  );
}
