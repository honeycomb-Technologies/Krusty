import { resolveModelPreferencePolicy } from "../components/chat-screen/modelPreferencePolicy.ts";
import type { ModelInfo, ModelKey } from "@mitsuro/api";
import { resolveUsableModel } from "../../../packages/state/src/session/modelSelection.ts";

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
  readTextFile(path: URL): Promise<string>;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function source(path: string): Promise<string> {
  return Deno.readTextFile(new URL(path, import.meta.url));
}

function model(id: string, provider: string, key?: ModelKey): ModelInfo {
  return {
    id,
    provider,
    key,
    display_name: id,
    context_window: 128_000,
    max_output: 8_192,
    supports_thinking: true,
    supports_tools: true,
    supports_vision: false,
  };
}

Deno.test("model preference policy follows the exact Hive session binding", () => {
  assert(
    resolveModelPreferencePolicy(true, "worker-x") === "store-only",
    "a bound Worker or primary Hive session must remain store-only",
  );
  assert(
    resolveModelPreferencePolicy(true, null) === "default-only",
    "an empty Hive shell must use the exact server default without mutating generic preferences",
  );
  assert(
    resolveModelPreferencePolicy(false, null) === "shared",
    "Chat and Code stores must retain the existing shared preference behavior",
  );
});

Deno.test("empty Hive ignores preserved Worker and ambiguous generic identity", () => {
  const workerKey: ModelKey = {
    provider: "grok",
    model_id: "worker-x",
    auth_scope: "oauth",
    api_format: "open_ai_responses",
  };
  const sharedDefaultKey: ModelKey = {
    provider: "openai",
    model_id: "shared-y",
    auth_scope: "api_key",
    api_format: "open_ai_responses",
  };
  const duplicateKey: ModelKey = {
    provider: "grok",
    model_id: "shared-y",
    auth_scope: "oauth",
    api_format: "open_ai_responses",
  };
  const catalog = [
    model("worker-x", "Grok", workerKey),
    model("shared-y", "Grok", duplicateKey),
    model("shared-y", "OpenAI", sharedDefaultKey),
  ];
  const policy = resolveModelPreferencePolicy(true, null);
  const usesStoreModel = policy !== "default-only";
  const selected = resolveUsableModel(
    usesStoreModel ? "worker-x" : null,
    "shared-y",
    catalog,
    [],
    usesStoreModel ? workerKey : null,
    sharedDefaultKey,
  );

  assert(
    selected?.key?.provider === "openai",
    "the exact server default must beat both the preserved Worker key and the first duplicate slug",
  );
});

Deno.test("an exact shared selection stays authoritative for empty Hive", () => {
  const serverDefaultKey: ModelKey = {
    provider: "grok",
    model_id: "shared-y",
    auth_scope: "oauth",
    api_format: "open_ai_responses",
  };
  const selectedKey: ModelKey = {
    provider: "openai",
    model_id: "shared-y",
    auth_scope: "api_key",
    api_format: "open_ai_responses",
  };
  const catalog = [
    model("shared-y", "Grok", serverDefaultKey),
    model("shared-y", "OpenAI", selectedKey),
  ];
  const selected = resolveUsableModel(
    "shared-y",
    "shared-y",
    catalog,
    [],
    selectedKey,
    serverDefaultKey,
  );
  assert(
    selected?.key?.provider === "openai",
    "a synchronous Chat/Code selection must beat a stale server default with the same slug",
  );
});

Deno.test("model readiness fences every selectedModel identity operation from Hive", async () => {
  const controller = await source(
    "../components/chat-screen/useSessionController.ts",
  );
  const readiness = controller.slice(
    controller.indexOf("const ensureModelReady"),
    controller.indexOf("// Connect warmup"),
  );

  assert(
    readiness.includes("const targetsHiveStore = targetStore ===") &&
      readiness.includes("const hiveSessionBinding = targetsHiveStore") &&
      readiness.includes(
        "targetStore.getState().sessionId === hiveSessionBinding",
      ),
    "model readiness must capture and retain exact Hive session ownership across awaits",
  );
  assert(
    readiness.includes(
      "readsSharedModelPreference &&\n      persistedModelCandidateRef.current === undefined",
    ) &&
      readiness.includes(
        "mutatesSharedModelPreference &&\n        persistedResolvedModelRef.current !== selectedModel.id",
      ) &&
      readiness.includes(
        "mutatesSharedModelPreference && persistedResolvedModelRef.current !== null",
      ),
    "identity reads and mutations must use distinct policy capabilities",
  );
  assert(
    readiness.includes(
      "(usesStoreModel ? targetStore.getState().model : null)",
    ) || readiness.includes("exactSharedSelection?.id ?? null"),
    "empty Hive must resolve from either its bound store or an exact shared selection",
  );
  assert(
    readiness.includes(
      "const existingModelKey = usesStoreModel",
    ) &&
      readiness.includes("exactSharedSelection?.key ?? null") &&
      readiness.includes("existingModelKey,\n      fallbackDefaultKey") &&
      readiness.includes("state.setModel(selectedModel.id"),
    "empty Hive must carry the exact shared key through local enrichment",
  );
  assert(
    readiness.includes("sharedExactSelectionRef.current = selection") &&
      controller.includes("responseDefaultMatchesLocal") &&
      controller.includes(
        "localSelection === null || responseDefaultMatchesLocal",
      ),
    "a local exact selection must synchronously outrank an older catalog response",
  );
});

Deno.test("model picker preserves exact row identity for duplicate slugs", async () => {
  const [popover, chatBar, actions] = await Promise.all([
    source("../components/chat/ChatBarModelPopover.tsx"),
    source("../components/chat/ChatBar.tsx"),
    source("../components/chat-screen/useSessionActions.ts"),
  ]);
  assert(
    popover.includes("keyExtractor={modelRowKey}") &&
      popover.includes("onSelectModel(item)") &&
      popover.includes("modelKeysEqual(item.key"),
    "each visible catalog row must retain and emit its exact provider/auth identity",
  );
  assert(
    chatBar.includes("onModelSelect: (model: ModelInfo) => void") &&
      chatBar.includes("onModelSelectRef.current(modelInfo)") &&
      chatBar.includes("modelKeysEqual(candidate.key ?? null, modelKey)"),
    "the composer must carry and highlight an exact ModelInfo rather than a bare slug",
  );
  const handler = actions.slice(
    actions.indexOf("const handleModelSelect"),
    actions.indexOf("const handleFastModeToggle"),
  );
  assert(
    handler.includes("(modelInfo: ModelInfo)") &&
      handler.includes("const modelId = modelInfo.id") &&
      !handler.includes("models.find"),
    "the action boundary must not re-resolve an exact row through first-slug matching",
  );
});

Deno.test("model readiness retains exact default and selection keys", async () => {
  const controller = await source(
    "../components/chat-screen/useSessionController.ts",
  );
  const readiness = controller.slice(
    controller.indexOf("const ensureModelReady"),
    controller.indexOf("// Connect warmup"),
  );
  assert(
    readiness.includes(
      "readsSharedModelPreference ? persistedModelCandidateRef.current : null",
    ),
    "Chat and Code retain the compatibility slug path while Hive uses exact keys",
  );
});

Deno.test("Hive model controls are read-only at UI and action boundaries", async () => {
  const [chatBar, controls, actions] = await Promise.all([
    source("../components/chat/ChatBar.tsx"),
    source("../components/chat/AccordionControls.tsx"),
    source("../components/chat-screen/useSessionActions.ts"),
  ]);
  assert(
    chatBar.includes("if (isHive) return;") &&
      chatBar.includes("!isHive && modelPickerOpen"),
    "Hive must close and suppress the generic model picker",
  );
  assert(
    controls.includes("disabled={modelManagedByHive}") &&
      controls.includes("Hive-managed model"),
    "Hive model controls must expose an honest disabled state",
  );
  assert(
    actions.includes("sessionStore === modeStores.hive.session") &&
      actions.includes("if (sessionStore === modeStores.hive.session) return;"),
    "stale model callbacks must be fenced before local or persisted mutation",
  );
});
