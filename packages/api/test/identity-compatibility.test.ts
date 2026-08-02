import { test } from "bun:test";

import { MitsuroApiError, MitsuroClient } from "../src/client.ts";
import { KrustyClient } from "../src/compatibility.ts";

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

test("new client reads an old-route server and normalizes old session values", async () => {
  const calls: string[] = [];
  const wireVersions: Array<string | null> = [];
  const fetchImpl: typeof fetch = async (input, init) => {
    const path = new URL(String(input)).pathname;
    calls.push(path);
    wireVersions.push(new Headers(init?.headers).get("X-Mitsuro-Wire-Version"));
    if (path === "/api/hive/main") return json({ error: "route not found" }, 404);
    if (path === "/api/mako/main") return json({ session_id: "main", created: false });
    if (path === "/api/sessions") {
      return json([{ id: "old", session_type: "mako" }]);
    }
    return json({ error: "unexpected" }, 500);
  };
  const client = new MitsuroClient({ baseUrl: "https://example.test", fetchImpl });

  assert((await client.getHiveMain()).session_id === "main", "canonical method should reach the old route");
  assert(
    calls[0] === "/api/hive/main" && calls[1] === "/api/mako/main",
    "the canonical route must be preferred before the old route",
  );
  assert(
    wireVersions.every((version) => version === "2"),
    "every canonical client request must negotiate typed Hive wire values",
  );
  const sessions = await client.getSessions();
  assert(sessions[0]?.session_type === "hive", "old wire values must not leak into app state");
});

test("auto route detection never replays a Hive mutation", async () => {
  const calls: Array<{ path: string; method: string }> = [];
  const fetchImpl: typeof fetch = async (input, init) => {
    const path = new URL(String(input)).pathname;
    const method = (init as { method?: string } | undefined)?.method ?? "GET";
    calls.push({ path, method });
    if (path.endsWith("/capabilities")) return json({ error: "route not found" }, 404);
    if (path === "/api/hive/sessions") return json({ error: "route not found" }, 404);
    if (path === "/api/mako/sessions") return json([]);
    if (path === "/api/mako/dispatch" && method === "POST") {
      return json({ session_id: "run-1", status: "queued" });
    }
    return json({ error: "unexpected" }, 500);
  };
  const client = new MitsuroClient({ baseUrl: "https://example.test", fetchImpl });

  await client.dispatchHive("test task");
  const mutations = calls.filter((call) => call.method !== "GET");
  assert(mutations.length === 1, "a route probe must happen before one mutation attempt");
  assert(
    mutations[0]?.path === "/api/mako/dispatch",
    "an old server should receive the mutation exactly once on its supported route",
  );
});

test("session creation encodes the old enum only after probing and never replays", async () => {
  const calls: Array<{ path: string; method: string; body?: string }> = [];
  const fetchImpl: typeof fetch = async (input, init) => {
    const path = new URL(String(input)).pathname;
    const requestInit = init as { method?: string; body?: unknown } | undefined;
    const method = requestInit?.method ?? "GET";
    const body = typeof requestInit?.body === "string" ? requestInit.body : undefined;
    calls.push({ path, method, body });
    if (path.endsWith("/capabilities")) return json({ error: "route not found" }, 404);
    if (path === "/api/hive/sessions") return json({ error: "route not found" }, 404);
    if (path === "/api/mako/sessions") return json([]);
    if (path === "/api/sessions" && method === "POST") {
      const payload = JSON.parse(body ?? "{}") as { session_type?: string };
      assert(payload.session_type === "mako", "a proven old server must receive its stored enum");
      return json({ id: "created", session_type: "mako" });
    }
    return json({ error: "unexpected" }, 500);
  };
  const client = new MitsuroClient({ baseUrl: "https://example.test", fetchImpl });

  const created = await client.createSession(undefined, undefined, undefined, undefined, "hive");
  assert(created.session_type === "hive", "the old response enum must normalize back to Hive");
  const mutations = calls.filter((call) => call.method !== "GET");
  assert(mutations.length === 1, "session creation must be sent exactly once after the probe");
  assert(mutations[0]?.path === "/api/sessions", "the non-prefixed session route must stay stable");
  assert(
    calls.slice(0, 4).map((call) => call.path).join(",") ===
      "/api/hive/capabilities,/api/mako/capabilities,/api/hive/sessions,/api/mako/sessions",
    "the generation probe must use capability routes then read-only session lists",
  );
});

test("chat stream encodes the old session enum before its single POST", async () => {
  const calls: Array<{ path: string; method: string; body?: string }> = [];
  const fetchImpl: typeof fetch = async (input, init) => {
    const path = new URL(String(input)).pathname;
    const requestInit = init as { method?: string; body?: unknown } | undefined;
    const method = requestInit?.method ?? "GET";
    const body = typeof requestInit?.body === "string" ? requestInit.body : undefined;
    calls.push({ path, method, body });
    if (path.endsWith("/capabilities")) return json({ error: "route not found" }, 404);
    if (path === "/api/hive/sessions") return json({ error: "route not found" }, 404);
    if (path === "/api/mako/sessions") return json([]);
    if (path === "/api/chat" && method === "POST") {
      const payload = JSON.parse(body ?? "{}") as { session_type?: string };
      assert(payload.session_type === "mako", "old chat transport must receive its session enum");
      return new Response("", { status: 200 });
    }
    return json({ error: "unexpected" }, 500);
  };
  const client = new MitsuroClient({ baseUrl: "https://example.test", fetchImpl });

  await client.streamChat(
    { message: "hello", session_type: "hive" },
    { onError() {}, onFinish() {} } as never,
  );
  const mutations = calls.filter((call) => call.method !== "GET");
  assert(mutations.length === 1, "stream creation must have one POST after its read-only probe");
  assert(mutations[0]?.path === "/api/chat", "the chat mutation must not be replayed elsewhere");
});

test("non-route failures never fall back", async () => {
  const calls: string[] = [];
  const fetchImpl: typeof fetch = async (input) => {
    calls.push(new URL(String(input)).pathname);
    return json({ error: "provider unavailable" }, 500);
  };
  const client = new MitsuroClient({ baseUrl: "https://example.test", fetchImpl });
  let error: unknown;
  try {
    await client.getHiveMain();
  } catch (caught) {
    error = caught;
  }
  assert(error instanceof MitsuroApiError && error.status === 500, "the original failure must surface");
  assert(calls.length === 1 && calls[0] === "/api/hive/main", "500 must not trigger an old-route retry");
});

test("session normalization never rewrites opaque message or tool payloads", async () => {
  const fetchImpl: typeof fetch = async (input) => {
    const path = new URL(String(input)).pathname;
    if (path === "/api/sessions/session-1") {
      return json({
        session: { id: "session-1", session_type: "mako" },
        messages: [
          {
            id: "message-1",
            content: [
              {
                type: "tool_call",
                input: { session_type: "mako", note: "user-owned data" },
              },
            ],
          },
        ],
      });
    }
    return json({ error: "unexpected" }, 500);
  };
  const client = new MitsuroClient({ baseUrl: "https://example.test", fetchImpl });

  const response = await client.getSession("session-1");
  assert(response.session.session_type === "hive", "the typed session field must normalize");
  const opaqueInput = (response.messages[0] as unknown as {
    content: Array<{ input: { session_type: string } }>;
  }).content[0]?.input;
  assert(
    opaqueInput?.session_type === "mako",
    "arbitrary message/tool payloads must remain untouched",
  );
});

test("deprecated client methods stay isolated but functional", async () => {
  const fetchImpl: typeof fetch = async (input) => {
    const path = new URL(String(input)).pathname;
    if (path === "/api/hive/main") return json({ error: "missing" }, 404);
    if (path === "/api/mako/main") return json({ session_id: "main", created: false });
    return json({ error: "unexpected" }, 500);
  };
  const client = new KrustyClient({ baseUrl: "https://example.test", fetchImpl });
  assert((await client.getMakoMain()).session_id === "main", "old method should forward through the safe transport bridge");
});
