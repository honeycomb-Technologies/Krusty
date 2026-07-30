#!/usr/bin/env node

import { createServer } from "node:http";

const port = Number(process.env.PORT ?? 3456);
const now = new Date().toISOString();
const session = {
  id: "fixture-heavy-session",
  title: "Tool-heavy performance fixture",
  token_count: 48_000,
  working_dir: null,
  project_dir: null,
  workspace_mode: "neutral",
  session_type: "chat",
  parent_session_id: null,
  mode: "build",
  updated_at: now,
  model: "fixture-model",
  model_key: null,
  model_catalog_revision: null,
  target_branch: null,
  permission_mode: "supervised",
};

const toolUses = Array.from({ length: 53 }, (_, index) => ({
  type: "tool_use",
  id: `fixture-tool-${index}`,
  name: index % 3 === 0 ? "read" : index % 3 === 1 ? "grep" : "bash",
  input: { fixture_index: index },
}));
const toolResults = toolUses.map((tool) => ({
  type: "tool_result",
  tool_use_id: tool.id,
  content: `Synthetic fixture result ${tool.id}`,
}));
const messages = [
  {
    role: "user",
    content: [{ type: "text", text: "Render the bounded performance fixture." }],
  },
  ...Array.from({ length: 35 }, (_, index) => ({
    role: "assistant",
    content: [
      {
        type: "text",
        text: `Completed fixture step ${index + 1}. This is intentionally plain committed prose.`,
      },
      ...toolUses.slice(
        Math.floor(index * toolUses.length / 35),
        Math.floor((index + 1) * toolUses.length / 35),
      ),
    ],
  })),
  {
    role: "assistant",
    content: [
      ...toolResults,
      {
        type: "text",
        text: "Fixture complete. The newest turn contains 37 messages and 53 tool calls.",
      },
    ],
  },
];

const previewSettings = {
  enabled: true,
  auto_refresh_secs: 5,
  show_only_http_like: true,
  probe_timeout_ms: 1_000,
  allow_force_open_non_http: false,
  pinned_ports: [],
  hidden_ports: [],
  blocked_ports: [],
};
const diagnosticSummary = {
  batches: 0,
  completed_batches: 0,
  events: 0,
  native_payloads: 0,
  event_types: {},
};

const server = createServer(async (request, response) => {
  const url = new URL(request.url ?? "/", `http://${request.headers.host}`);
  response.setHeader("content-type", "application/json");

  if (url.pathname === "/health") {
    return json(response, { status: "ok", version: "fixture" });
  }
  if (url.pathname === "/api/sessions" && request.method === "GET") {
    return json(response, [session]);
  }
  if (url.pathname === `/api/sessions/${session.id}`) {
    await delay(120);
    return json(response, { session, messages });
  }
  if (url.pathname === `/api/sessions/${session.id}/state`) {
    return json(response, {
      id: session.id,
      agent_state: "idle",
      started_at: null,
      last_event_at: null,
      mode: "build",
      permission_mode: "supervised",
      workflow: null,
      recovery: null,
      pending_interactions: [],
      live_partial_assistant: null,
      delegated_tools: [],
      recent_delegated_runs: [],
      last_event_sequence: 1,
    });
  }
  if (url.pathname === `/api/sessions/${session.id}/presence`) {
    return json(response, {
      session_id: session.id,
      active_viewers: 1,
      active_controllers: 1,
      stale_clients: 0,
      clients: [],
    });
  }
  if (url.pathname === "/api/models") {
    return json(response, {
      models: [{
        id: "fixture-model",
        display_name: "Fixture",
        provider: "fixture",
        context_window: 128_000,
        max_output: 8_192,
        supports_thinking: false,
        supports_tools: true,
        supports_vision: false,
      }],
      default_model: "fixture-model",
      default_model_key: null,
    });
  }
  if (url.pathname === "/api/credentials") {
    return json(response, []);
  }
  if (url.pathname === "/api/mcp") {
    return json(response, []);
  }
  if (url.pathname === "/api/skills") {
    return json(response, Array.from({ length: 24 }, (_, index) => ({
      name: `fixture-skill-${index + 1}`,
      description: "Synthetic bounded settings row",
      tags: ["fixture"],
      source: "global",
    })));
  }
  if (url.pathname === "/api/ports") {
    return json(response, {
      ports: [],
      settings: previewSettings,
      discovery_error: null,
    });
  }
  if (url.pathname === "/api/reports") {
    return json(response, { reports: [] });
  }
  if (url.pathname === "/api/apns/status") {
    return json(response, { configured: false });
  }
  if (url.pathname === "/api/mobile-diagnostics/batches") {
    const body = await readBody(request);
    const events = Array.isArray(body?.events) ? body.events : [];
    const nativePayloads = Array.isArray(body?.native_payloads)
      ? body.native_payloads
      : [];
    diagnosticSummary.batches += 1;
    diagnosticSummary.completed_batches += body?.completed === true ? 1 : 0;
    diagnosticSummary.events += events.length;
    diagnosticSummary.native_payloads += nativePayloads.length;
    for (const event of events) {
      const type =
        typeof event?.category === "string" ? event.category : "unknown";
      diagnosticSummary.event_types[type] =
        (diagnosticSummary.event_types[type] ?? 0) + 1;
    }
    return json(response, {
      run_id: body?.run?.id ?? "fixture-run",
      accepted_events: events.length,
      accepted_native_payloads: nativePayloads.length,
      dropped_attributes: 0,
    });
  }
  if (url.pathname === "/fixture/diagnostics" && request.method === "GET") {
    return json(response, diagnosticSummary);
  }

  return json(response, {});
});

server.listen(port, "127.0.0.1", () => {
  console.log(`Mobile performance fixture listening on http://127.0.0.1:${port}`);
});

function json(response, value, status = 200) {
  response.statusCode = status;
  response.end(JSON.stringify(value));
}

function delay(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

async function readBody(request) {
  const chunks = [];
  for await (const chunk of request) chunks.push(chunk);
  try {
    return JSON.parse(Buffer.concat(chunks).toString("utf8"));
  } catch {
    return null;
  }
}
