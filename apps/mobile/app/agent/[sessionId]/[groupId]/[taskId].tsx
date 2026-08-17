import { useLocalSearchParams } from "expo-router";

import { AgentConversationScreen } from "../../../../components/chat/AgentConversationScreen";

function param(value: string | string[] | undefined): string {
  return Array.isArray(value) ? value[0] || "" : value || "";
}

export default function AgentActivityRoute() {
  const params = useLocalSearchParams<{
    sessionId: string | string[];
    groupId: string | string[];
    taskId: string | string[];
    name?: string | string[];
    fromParent?: string | string[];
  }>();
  const sessionId = param(params.sessionId);
  const groupId = param(params.groupId);
  const taskId = param(params.taskId);
  const name = param(params.name) || "Hive Worker";
  const openedFromParent = param(params.fromParent) === "1";

  return (
    <AgentConversationScreen
      sessionId={sessionId}
      groupId={groupId}
      taskId={taskId}
      fallbackName={name}
      openedFromParent={openedFromParent}
    />
  );
}
