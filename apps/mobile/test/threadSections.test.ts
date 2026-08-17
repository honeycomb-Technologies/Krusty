// @ts-ignore Deno requires an explicit extension; the Expo compiler resolves the same source through its extensionless application imports.
import { applySessionListOverrides, archivedSessions, chronologicalSessions, chronologicalThreadDayGroups, codeDirectoryToAutoExpand, codeProjectThreadGroups, formatThreadMetric, sessionModelLabel, sessionProjectDirectory, sessionProviderKey, sessionProviderLabel, sessionStateLabel } from "../components/navigation/threadSections.ts";
import type { SessionResponse } from "@mitsuro/api";

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
};

function assertEquals<T>(actual: T, expected: T, message: string): void {
  if (!Object.is(actual, expected)) {
    throw new Error(`${message}: expected ${String(expected)}, got ${String(actual)}`);
  }
}

function session(
  id: string,
  updatedAt: string,
  overrides: Partial<SessionResponse> = {},
): SessionResponse {
  return {
    id,
    title: id,
    working_dir: "/work/mitsuro",
    project_dir: "/work/mitsuro",
    workspace_mode: "selected",
    session_type: "code",
    parent_session_id: null,
    mode: "build",
    updated_at: updatedAt,
    target_branch: null,
    permission_mode: "autonomous",
    ...overrides,
  };
}

Deno.test("code projects and tasks sort by latest activity", () => {
  const groups = codeProjectThreadGroups([
    session("older", "2026-08-14T10:00:00Z"),
    session("newer", "2026-08-16T10:00:00Z"),
    session("other", "2026-08-15T10:00:00Z", {
      project_dir: "/work/other",
      working_dir: "/work/other",
    }),
  ]);

  assertEquals(groups[0]?.directory, "/work/mitsuro", "latest project sorts first");
  assertEquals(groups[0]?.sessions[0]?.id, "newer", "latest task sorts first");
  assertEquals(groups[0]?.sessions.length, 2, "project retains both tasks");
});

Deno.test("thread metadata labels stay compact and honest", () => {
  assertEquals(sessionModelLabel("openai/gpt-5.6"), "gpt-5.6", "provider is trimmed");
  assertEquals(sessionStateLabel("tool_executing"), "Working", "active tool state is working");
  assertEquals(sessionStateLabel("awaiting_input"), "Needs input", "input state is actionable");
  assertEquals(sessionStateLabel(undefined), null, "missing state stays absent");
  assertEquals(formatThreadMetric(999), "999", "small metrics stay exact");
  assertEquals(formatThreadMetric(1_450), "1.4k", "compact metrics use one decimal");
  assertEquals(formatThreadMetric(140_839), "141k", "large metrics stay scannable");
});

Deno.test("conversation avatars prefer durable provider identity and recognize Grok", () => {
  const grok = session("grok", "2026-08-16T10:00:00Z", {
    model: "grok-4.6",
    model_key: {
      provider: "xai",
      model_id: "grok-4.6",
      api_format: "openai_responses",
    },
  });
  const legacyClaude = session("claude", "2026-08-16T10:00:00Z", {
    model: "anthropic/claude-opus-4.1",
  });

  assertEquals(sessionProviderKey(grok), "xai", "exact model key wins");
  assertEquals(sessionProviderLabel(grok), "Grok", "xAI is presented as Grok");
  assertEquals(
    sessionProviderKey(legacyClaude),
    "anthropic",
    "legacy model strings remain identifiable",
  );
});

Deno.test("project directory falls back to the working directory and General", () => {
  assertEquals(
    sessionProjectDirectory(session("working", "2026-08-16T10:00:00Z", {
      project_dir: null,
      working_dir: "/work/fallback",
    })),
    "/work/fallback",
    "working directory is the fallback",
  );
  assertEquals(
    sessionProjectDirectory(session("neutral", "2026-08-16T10:00:00Z", {
      project_dir: null,
      working_dir: null,
    })),
    "Neutral",
    "missing directories remain General",
  );
});

Deno.test("neutral Code conversations do not auto-expand a fake project folder", () => {
  const neutral = session("neutral", "2026-08-16T10:00:00Z", {
    project_dir: null,
    working_dir: null,
  });
  assertEquals(
    codeDirectoryToAutoExpand([neutral], neutral.id, null),
    null,
    "General stays a flat conversation list",
  );
});

Deno.test("active lists put pinned conversations first and omit archived rows", () => {
  const ordinary = session("ordinary", "2026-08-16T12:00:00Z");
  const pinned = session("pinned", "2026-08-16T10:00:00Z", {
    pinned_at: "2026-08-16T11:00:00Z",
  });
  const archived = session("archived", "2026-08-16T13:00:00Z", {
    archived_at: "2026-08-16T13:30:00Z",
  });

  const active = chronologicalSessions([ordinary, archived, pinned], "code");
  assertEquals(active.length, 2, "archive is absent from active list");
  assertEquals(active[0]?.id, "pinned", "pinned thread sorts first");
  assertEquals(
    archivedSessions([ordinary, archived, pinned], "code")[0]?.id,
    "archived",
    "archived management list remains recoverable",
  );
});

Deno.test("chronological Code tasks group into newest-first local days", () => {
  const now = new Date(2026, 7, 16, 12);
  const groups = chronologicalThreadDayGroups(
    [
      session("today", new Date(2026, 7, 16, 9).toISOString()),
      session("yesterday-new", new Date(2026, 7, 15, 14).toISOString()),
      session("yesterday-old", new Date(2026, 7, 15, 8).toISOString()),
      session("older", new Date(2026, 7, 11, 12).toISOString()),
    ],
    "code",
    now,
  );

  assertEquals(groups.length, 3, "one section is created per local day");
  assertEquals(groups[0]?.label, "Today", "current day gets a relative label");
  assertEquals(groups[1]?.label, "Yesterday", "previous day gets a relative label");
  assertEquals(groups[1]?.sessions[0]?.id, "yesterday-new", "tasks stay newest first within a day");
  assertEquals(groups[2]?.label, "Tuesday, Aug 11", "older days stay recognizable");
});

Deno.test("pinned Code tasks stay in their calendar day", () => {
  const now = new Date(2026, 7, 16, 12);
  const groups = chronologicalThreadDayGroups(
    [
      session("today", new Date(2026, 7, 16, 9).toISOString()),
      session("older-pinned", new Date(2026, 7, 15, 9).toISOString(), {
        pinned_at: new Date(2026, 7, 16, 10).toISOString(),
      }),
    ],
    "code",
    now,
  );

  assertEquals(groups[0]?.label, "Today", "day order remains chronological");
  assertEquals(groups[1]?.sessions[0]?.id, "older-pinned", "pinning does not move a task into Today");
});

Deno.test("local overrides hide deleted and archived rows immediately", () => {
  const active = session("active", "2026-08-16T12:00:00Z");
  const doomed = session("doomed", "2026-08-16T12:00:00Z");
  const parked = session("parked", "2026-08-16T12:00:00Z");
  const restored = session("restored", "2026-08-16T11:00:00Z", {
    archived_at: "2026-08-16T11:30:00Z",
  });

  const next = applySessionListOverrides(
    [active, doomed, parked],
    {
      doomed: { type: "remove" },
      parked: { type: "archive", archived_at: "2026-08-16T12:05:00Z" },
      restored: { type: "archive", archived_at: null },
    },
    [restored],
  );

  assertEquals(
    chronologicalSessions(next, "code").map((item) => item.id).join(","),
    "active,restored",
    "delete and archive must leave the active list in the same pass",
  );
  assertEquals(
    next.find((item) => item.id === "parked")?.archived_at,
    "2026-08-16T12:05:00Z",
    "archive override stamps archived_at before the parent store updates",
  );
});

Deno.test("a pinned conversation promotes its project folder", () => {
  const recent = session("recent", "2026-08-16T14:00:00Z", {
    project_dir: "/work/recent",
    working_dir: "/work/recent",
  });
  const pinned = session("pinned-project", "2026-08-16T10:00:00Z", {
    project_dir: "/work/pinned",
    working_dir: "/work/pinned",
    pinned_at: "2026-08-16T13:00:00Z",
  });
  const groups = codeProjectThreadGroups([recent, pinned]);
  assertEquals(groups[0]?.directory, "/work/pinned", "pinned project sorts first");
});
