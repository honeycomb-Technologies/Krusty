import type { MitsuroClient, ModelKey } from "@mitsuro/api";
import { createSessionStore } from "../src/session/store.ts";

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
};

function assertEquals(actual: unknown, expected: unknown, message: string) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(
      `${message}\nexpected: ${JSON.stringify(expected)}\nactual: ${
        JSON.stringify(actual)
      }`,
    );
  }
}

const exactGrokKey: ModelKey = {
  provider: "grok",
  model_id: "grok-4.6",
  auth_scope: "oauth",
  api_format: "open_ai_responses",
};

Deno.test(
  "Hive-owned store blocks generic metadata PATCH before session type hydrates",
  async () => {
    const sessionUpdates: Array<{ id: string; data: unknown }> = [];
    const currentModelUpdates: Array<{
      model: string | null;
      key: ModelKey | null | undefined;
    }> = [];
    let planVisible = false;
    let sessionListReloads = 0;
    const client = {
      updateSession: async (id: string, data: unknown) => {
        sessionUpdates.push({ id, data });
        return {};
      },
      setCurrentModel: async (
        model: string | null,
        key?: ModelKey | null,
      ) => {
        currentModelUpdates.push({ model, key });
        return { ok: true };
      },
      streamChat: async (
        _request: unknown,
        callbacks: { onModeChange(mode: "plan" | "build"): void },
      ) => {
        callbacks.onModeChange("plan");
      },
    } as unknown as MitsuroClient;
    const storage = {
      get: () => null,
      set: () => {},
      delete: () => {},
    };
    const workspace = {
      getState: () => ({ clear: () => {} }),
    };
    const sessionsStore = {
      getState: () => ({
        loadSessions: () => {
          sessionListReloads += 1;
        },
      }),
    };
    const planStore = {
      getState: () => ({
        setWorkflow: () => {},
        setVisible: (visible: boolean) => {
          planVisible = visible;
        },
      }),
    };
    const store = createSessionStore(
      client,
      storage,
      workspace as never,
      sessionsStore as never,
      planStore as never,
      "hive",
    );

    // Worker DMs are not guaranteed to appear in the generic session list.
    // Their optimistic shell can therefore have identity but no hydrated type.
    store.getState().initSession("seed-worker-dm", "Atlas");
    assertEquals(
      store.getState().sessionType,
      null,
      "the regression requires the pre-hydration null session type",
    );

    store.getState().setModel(
      exactGrokKey.model_id,
      exactGrokKey.provider,
      null,
      exactGrokKey,
    );
    await Promise.resolve();

    assertEquals(
      sessionUpdates,
      [],
      "catalog reconciliation must not PATCH runtime-owned Worker metadata",
    );
    assertEquals(
      sessionListReloads,
      0,
      "a skipped Worker PATCH must not refresh the generic session list",
    );
    assertEquals(
      currentModelUpdates,
      [],
      "opening a Worker must not replace the generic current-model preference",
    );

    assertEquals(
      store.getState().permissionMode,
      "autonomous",
      "the local permission mode starts from its stored default",
    );
    store.getState().togglePermissionMode();
    await Promise.resolve();

    assertEquals(
      store.getState().permissionMode,
      "supervised",
      "the Hive permission toggle must still update local UI state",
    );
    assertEquals(
      sessionUpdates,
      [],
      "permission toggles must not PATCH runtime-owned Hive metadata",
    );
    assertEquals(
      sessionListReloads,
      0,
      "a skipped Hive permission PATCH must not refresh the session list",
    );

    await store.getState().sendMessage("exercise stream mode callback");
    await Promise.resolve();

    assertEquals(
      store.getState().sessionType,
      null,
      "the stream regression must retain pre-hydration null session type",
    );
    assertEquals(
      store.getState().mode,
      "plan",
      "the stream callback must still apply local mode state",
    );
    assertEquals(
      planVisible,
      true,
      "the stream callback must still reveal local plan state",
    );
    assertEquals(
      sessionUpdates,
      [],
      "stream mode changes must not PATCH runtime-owned Hive metadata",
    );
    assertEquals(
      sessionListReloads,
      0,
      "a skipped Hive mode PATCH must not refresh the session list",
    );

    store.getState().cleanup();
  },
);
