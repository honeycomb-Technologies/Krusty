import type { ModelKey } from "@mitsuro/api";
import { buildEmptyHiveDispatchSelection } from "../components/chat-screen/emptyHiveDispatchSelection.ts";

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
  readTextFile(path: URL): Promise<string>;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

Deno.test("empty Hive dispatch preserves exact identity for duplicate model slugs", () => {
  const grokKey: ModelKey = {
    provider: "grok",
    model_id: "shared-model",
    auth_scope: "oauth",
    api_format: "open_ai_responses",
  };
  const openAiKey: ModelKey = {
    provider: "openai",
    model_id: "shared-model",
    auth_scope: "api_key",
    api_format: "open_ai_responses",
  };

  const grokSelection = buildEmptyHiveDispatchSelection(
    "shared-model",
    grokKey,
  );
  const openAiSelection = buildEmptyHiveDispatchSelection(
    "shared-model",
    openAiKey,
  );

  assert(
    grokSelection.model === openAiSelection.model &&
      grokSelection.modelKey?.provider === "grok" &&
      grokSelection.modelKey?.auth_scope === "oauth" &&
      openAiSelection.modelKey?.provider === "openai" &&
      openAiSelection.modelKey?.auth_scope === "api_key",
    "the shared slug must not erase provider or auth identity",
  );
});

Deno.test("empty Hive reads and forwards the exact key only after readiness", async () => {
  const index = await Deno.readTextFile(
    new URL("../app/(tabs)/index.tsx", import.meta.url),
  );
  const dispatch = index.slice(
    index.indexOf("const handleChatBarSend"),
    index.indexOf("const handleHiveWorkerSend"),
  );
  const readiness = dispatch.indexOf(
    "const resolvedModel = await ensureModelReady()",
  );
  const postReadinessFence = dispatch.indexOf(
    "if (!isCurrentDispatchPhase(null)) return;",
    readiness,
  );
  const exactKeyRead = dispatch.indexOf(
    "hiveStore.getState().modelKey",
    postReadinessFence,
  );
  const preDispatchFence = dispatch.indexOf(
    "if (!isCurrentDispatchPhase(null)) return;",
    exactKeyRead,
  );
  const dispatchCall = dispatch.indexOf(
    "await client.dispatchHive",
    preDispatchFence,
  );
  const selectionForward = dispatch.indexOf(
    "...dispatchModelSelection",
    dispatchCall,
  );

  assert(
    readiness > 0 && postReadinessFence > readiness &&
      exactKeyRead > postReadinessFence && preDispatchFence > exactKeyRead &&
      dispatchCall > preDispatchFence &&
      selectionForward > dispatchCall,
    "the exact empty-Hive key must be read after readiness/fencing and forwarded with dispatch",
  );
});
