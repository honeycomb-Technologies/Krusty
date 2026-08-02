import type {
  DelegatedProgressEvent,
  DelegatedRunResponse,
  DelegatedToolKind,
  DelegatedToolStateResponse,
} from '@mitsuro/api';

import type {
  ChatMessage,
  DelegatedAgentState,
  DelegatedArtifactState,
  ToolCall,
} from './types';

type DelegatedToolName = DelegatedToolKind | 'agent';
type ParsedToolEnvelope = {
  ok?: boolean;
  data?: Record<string, unknown>;
  error?: { message?: string };
  warnings?: string[];
};

function isDelegatedKind(value: unknown): value is DelegatedToolKind {
  return value === 'explore' || value === 'plan' || value === 'verify' || value === 'build';
}

export function formatToolOutputForDisplay(
  toolName: string,
  output?: string,
  args?: Record<string, unknown>,
): string | undefined {
  const delegatedKind = resolveDelegatedKind(toolName, args);
  if (delegatedKind) {
    return conciseDelegatedOutput(delegatedKind, output, args);
  }

  return conciseStructuredToolOutput(output);
}

function conciseStructuredToolOutput(output?: string): string | undefined {
  if (!output) return output;

  let parsed: unknown;
  try {
    parsed = JSON.parse(output);
  } catch {
    return output;
  }
  if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
    return typeof parsed === 'string' ? parsed : output;
  }

  const envelope = parsed as Record<string, unknown>;
  const summary = typeof envelope.summary === 'string' ? envelope.summary.trim() : '';
  const result = envelope.result;
  if (result && typeof result === 'object' && !Array.isArray(result)) {
    const resultRecord = result as Record<string, unknown>;
    const preview = typeof resultRecord.output_preview === 'string'
      ? resultRecord.output_preview.trim()
      : '';
    const error = typeof resultRecord.error === 'string'
      ? resultRecord.error.trim()
      : '';
    if (preview) return preview;
    if (error) return error;
    if (summary) return summary;
  }
  if (typeof result === 'string' && result.trim()) return result.trim();

  const error = envelope.error;
  if (typeof error === 'string' && error.trim()) return error.trim();
  if (error && typeof error === 'object' && !Array.isArray(error)) {
    const message = (error as Record<string, unknown>).message;
    if (typeof message === 'string' && message.trim()) return message.trim();
  }
  if (summary) return summary;

  return output;
}

export function isDelegatedToolName(name: string): name is DelegatedToolName {
  return name === 'agent' || isDelegatedKind(name);
}

export function resolveDelegatedKind(
  toolName: string,
  args?: Record<string, unknown>,
  fallbackKind?: DelegatedToolKind | null,
  authoritativeCapabilities?: unknown,
): DelegatedToolKind | undefined {
  if (isDelegatedKind(toolName)) {
    return toolName;
  }

  if (toolName === 'agent') {
    const capabilitySource = authoritativeCapabilities ?? args?.capabilities;
    if (Array.isArray(capabilitySource)) {
      const capabilities = capabilitySource.filter(
        (value): value is string => typeof value === 'string',
      );
      return capabilities.includes('write') ? 'build' : 'explore';
    }

    const agentType = args?.agent_type;
    if (isDelegatedKind(agentType)) {
      return agentType;
    }
    const profile = args?.profile;
    if (isDelegatedKind(profile)) {
      return profile;
    }
    const action = typeof args?.action === 'string' ? args.action : 'spawn';
    if (action === 'spawn') return 'explore';
  }

  return fallbackKind ?? undefined;
}

function defaultAgentName(
  kind: DelegatedToolKind,
  args?: Record<string, unknown>,
): string {
  if (typeof args?.name === 'string' && args.name.trim()) {
    return args.name.trim();
  }
  switch (kind) {
    case 'build':
      return 'builder';
    case 'plan':
      return 'planner';
    case 'verify':
      return 'verifier';
    case 'explore':
      return 'agent';
  }
}

function delegatedDisplayName(path: string, fallback: string): string {
  const parts = path.split('/').filter(Boolean);
  return (parts[parts.length - 1] || fallback).slice(0, 24);
}

function buildDelegatedTargets(
  kind: DelegatedToolKind,
  args?: Record<string, unknown>,
): string[] {
  if (!args) return [];

  const components = Array.isArray(args.components) ? args.components : [];
  const directories = Array.isArray(args.directories) ? args.directories : [];
  const files = Array.isArray(args.files) ? args.files : [];
  const scope = typeof args.scope === 'string' ? [args.scope] : [];

  const sources = kind === 'build'
    ? components
    : kind === 'explore'
      ? [...scope, ...directories, ...files]
      : [];

  return sources.filter(
    (value): value is string => typeof value === 'string' && value.trim().length > 0,
  );
}

function buildSeedDelegatedAgents(
  kind: DelegatedToolKind,
  args?: Record<string, unknown>,
): DelegatedAgentState[] {
  const sources = buildDelegatedTargets(kind, args);
  const fallbackName = defaultAgentName(kind, args);

  if (sources.length === 0) {
    return [
      {
        taskId: 'main',
        name: fallbackName,
        status: 'pending',
        toolCount: 0,
        tokens: 0,
        linesAdded: 0,
        linesRemoved: 0,
      },
    ];
  }

  return sources.map((source, index) => ({
    taskId: kind === 'explore' ? `dir-${index}` : `${kind}-${index}`,
    name:
      typeof args?.name === 'string' && args.name.trim()
        ? `${args.name.trim()} / ${delegatedDisplayName(source, fallbackName)}`
        : delegatedDisplayName(source, fallbackName),
    status: 'pending' as const,
    toolCount: 0,
    tokens: 0,
    currentAction:
      typeof args?.instructions === 'string'
        ? args.instructions.slice(0, 80)
        : typeof args?.prompt === 'string'
          ? args.prompt.slice(0, 80)
          : undefined,
    linesAdded: 0,
    linesRemoved: 0,
  }));
}

function parseToolEnvelope(output: string): ParsedToolEnvelope | undefined {
  try {
    const parsed = JSON.parse(output) as Record<string, unknown>;
    return {
      ok: typeof parsed.ok === 'boolean' ? parsed.ok : undefined,
      data:
        parsed.data && typeof parsed.data === 'object'
          ? (parsed.data as Record<string, unknown>)
          : parsed,
      error:
        parsed.error && typeof parsed.error === 'object'
          ? (parsed.error as { message?: string })
          : undefined,
      warnings: Array.isArray(parsed.warnings)
        ? parsed.warnings.filter((warning): warning is string => typeof warning === 'string')
        : [],
    };
  } catch {
    return undefined;
  }
}

function conciseDelegatedOutput(
  kind: DelegatedToolKind,
  output?: string,
  args?: Record<string, unknown>,
): string | undefined {
  if (!output) return output;
  const envelope = parseToolEnvelope(output);
  const payload = envelope?.data;
  if (!payload) return output;

  const lines: string[] = [];
  const outcome =
    typeof payload.outcome === 'string'
      ? payload.outcome
      : typeof payload.success === 'boolean'
        ? payload.success
          ? 'success'
          : 'failed'
        : undefined;
  const delegatedRunId =
    typeof payload.delegated_run_id === 'string'
      ? payload.delegated_run_id
      : undefined;
  const headline =
    typeof payload.message === 'string'
      ? payload.message
      : typeof payload.investigation_summary === 'string'
        ? payload.investigation_summary
        : typeof payload.findings === 'string'
          ? payload.findings
          : undefined;

  lines.push(`${kind} ${outcome ?? (envelope?.ok === false ? 'failed' : 'completed')}`);
  if (delegatedRunId) lines.push(`run: ${delegatedRunId}`);
  if (headline) lines.push(headline.slice(0, 1200));

  const agentCount =
    typeof payload.agent_count === 'number'
      ? payload.agent_count
      : typeof payload.builder_count === 'number'
        ? payload.builder_count
        : undefined;
  const failedAgents =
    typeof payload.failed_agents === 'number' ? payload.failed_agents : undefined;
  const turns =
    typeof payload.total_turns === 'number'
      ? payload.total_turns
      : typeof payload.turns_used === 'number'
        ? payload.turns_used
        : undefined;
  const files =
    typeof payload.paths_examined_count === 'number'
      ? payload.paths_examined_count
      : typeof payload.files_examined_count === 'number'
        ? payload.files_examined_count
        : undefined;
  const stats = [
    agentCount !== undefined ? `${agentCount} agent${agentCount === 1 ? '' : 's'}` : undefined,
    failedAgents !== undefined ? `${failedAgents} failed` : undefined,
    turns !== undefined ? `${turns} turns` : undefined,
    files !== undefined ? `${files} paths` : undefined,
  ].filter(Boolean);
  if (stats.length > 0) lines.push(stats.join(' · '));

  const targets = buildDelegatedTargets(kind, args).slice(0, 3);
  if (targets.length > 0) lines.push(`targets: ${targets.join(', ')}`);

  const warnings = envelope?.warnings?.slice(0, 2) ?? [];
  for (const warning of warnings) lines.push(`warning: ${warning}`);
  if (envelope?.error?.message) lines.push(`error: ${envelope.error.message}`);

  return lines.filter((line) => line.trim().length > 0).join('\n');
}

export function createDelegatedArtifactState(
  kind: DelegatedToolKind,
  args?: Record<string, unknown>,
): DelegatedArtifactState {
  return {
    kind,
    name:
      typeof args?.name === 'string' && args.name.trim()
        ? args.name.trim()
        : undefined,
    capabilities: Array.isArray(args?.capabilities)
      ? args.capabilities.filter(
          (value): value is 'read' | 'write' | 'execute' =>
            value === 'read' || value === 'write' || value === 'execute',
        )
      : undefined,
    delegatedRunId: undefined,
    stage: 'created',
    thinking: undefined,
    agents: buildSeedDelegatedAgents(kind, args),
    filesExamined: [],
    errors: [],
    totalTargets: buildSeedDelegatedAgents(kind, args).length,
  };
}

function mergeDelegatedAgents(
  current: DelegatedAgentState[] | undefined,
  next: DelegatedAgentState[],
): DelegatedAgentState[] {
  if (!current || current.length === 0) return next;
  if (next.length === 0) return current;

  const merged = [...current];
  for (const nextAgent of next) {
    const index = merged.findIndex(
      (agent) => agent.taskId === nextAgent.taskId || agent.name === nextAgent.name,
    );
    if (index >= 0) {
      merged[index] = { ...merged[index], ...nextAgent };
    } else {
      merged.push(nextAgent);
    }
  }
  return merged;
}

export function annotateDelegatedArtifactState(
  artifact: DelegatedArtifactState,
): DelegatedArtifactState {
  const totalTargets =
    artifact.totalTargets ?? artifact.agentCount ?? artifact.agents.length;
  const activeTargets = artifact.agents.filter(
    (agent) => agent.status === 'running',
  ).length;
  const completedTargets = artifact.agents.filter(
    (agent) => agent.status === 'complete',
  ).length;
  const pendingTargets = Math.max(
    totalTargets
      - activeTargets
      - completedTargets
      - artifact.agents.filter((agent) => agent.status === 'failed').length,
    artifact.agents.filter((agent) => agent.status === 'pending').length,
  );
  return {
    ...artifact,
    totalTargets,
    activeTargets,
    completedTargets,
    pendingTargets,
  };
}

export function mergeDelegatedArtifactState(
  current: DelegatedArtifactState | undefined,
  next: DelegatedArtifactState,
): DelegatedArtifactState {
  return annotateDelegatedArtifactState({
    ...current,
    ...next,
    agents: mergeDelegatedAgents(current?.agents, next.agents),
    filesExamined:
      next.filesExamined.length > 0
        ? next.filesExamined
        : current?.filesExamined || [],
    errors: next.errors.length > 0 ? next.errors : current?.errors || [],
  });
}

export function parseDelegatedArtifactState(
  toolName: string,
  output?: string,
  args?: Record<string, unknown>,
  fallbackKind?: DelegatedToolKind | null,
): DelegatedArtifactState | undefined {
  if (!output) return undefined;

  const envelope = parseToolEnvelope(output);
  const payload = envelope?.data;
  if (!payload) return undefined;

  try {
    const kind = resolveDelegatedKind(toolName, args, fallbackKind);
    if (!kind) return undefined;

    const listKey = kind === 'build' ? 'builders' : 'agents';
    const artifact: DelegatedArtifactState = {
      kind,
      name:
        typeof args?.name === 'string' && args.name.trim()
          ? args.name.trim()
          : undefined,
      capabilities: Array.isArray(args?.capabilities)
        ? args.capabilities.filter(
            (value): value is 'read' | 'write' | 'execute' =>
              value === 'read' || value === 'write' || value === 'execute',
          )
        : undefined,
      delegatedRunId:
        typeof payload.delegated_run_id === 'string'
          ? payload.delegated_run_id
          : undefined,
      stage:
        payload.outcome === 'success'
          ? 'complete'
          : payload.outcome === 'partial'
            ? 'degraded'
            : payload.outcome === 'failed'
              ? 'failed'
              : undefined,
      message:
        typeof payload.message === 'string' ? payload.message : undefined,
      investigationSummary:
        typeof payload.investigation_summary === 'string'
          ? payload.investigation_summary
          : typeof payload.findings === 'string'
            ? payload.findings
            : undefined,
      humanReview:
        typeof payload.human_review === 'string'
          ? payload.human_review
          : undefined,
      outcome:
        payload.outcome === 'success'
        || payload.outcome === 'partial'
        || payload.outcome === 'failed'
          ? payload.outcome
          : typeof payload.success === 'boolean'
            ? payload.success
              ? 'success'
              : 'failed'
            : undefined,
      confidence:
        payload.confidence === 'high'
        || payload.confidence === 'medium'
        || payload.confidence === 'low'
          ? payload.confidence
          : undefined,
      structuralCoverage:
        payload.structural_coverage === 'high'
        || payload.structural_coverage === 'medium'
        || payload.structural_coverage === 'low'
          ? payload.structural_coverage
          : undefined,
      semanticCoverage:
        payload.semantic_coverage === 'high'
        || payload.semantic_coverage === 'medium'
        || payload.semantic_coverage === 'low'
          ? payload.semantic_coverage
          : undefined,
      agents: [],
      filesExamined: Array.isArray(payload.paths_examined)
        ? payload.paths_examined.filter(
            (value): value is string => typeof value === 'string',
          )
        : Array.isArray(payload.files_examined)
          ? payload.files_examined.filter(
              (value): value is string => typeof value === 'string',
            )
          : [],
      errors: Array.isArray(payload.errors)
        ? payload.errors.filter((value): value is string => typeof value === 'string')
        : envelope.error?.message
          ? [envelope.error.message]
          : [],
      agentCount:
        typeof payload.agent_count === 'number'
          ? payload.agent_count
          : typeof payload.builder_count === 'number'
            ? payload.builder_count
            : undefined,
      usableAgents:
        typeof payload.usable_agents === 'number'
          ? payload.usable_agents
          : undefined,
      degradedAgents:
        typeof payload.degraded_agents === 'number'
          ? payload.degraded_agents
          : undefined,
      successfulAgents:
        typeof payload.successful_agents === 'number'
          ? payload.successful_agents
          : undefined,
      failedAgents:
        typeof payload.failed_agents === 'number'
          ? payload.failed_agents
          : undefined,
      filesExaminedCount:
        typeof payload.paths_examined_count === 'number'
          ? payload.paths_examined_count
          : typeof payload.files_examined_count === 'number'
            ? payload.files_examined_count
            : undefined,
      outcomeReason:
        typeof payload.outcome_reason === 'string'
          ? payload.outcome_reason
          : undefined,
      totalTurns:
        typeof payload.total_turns === 'number'
          ? payload.total_turns
          : typeof payload.turns_used === 'number'
            ? payload.turns_used
            : undefined,
      totalDurationMs:
        typeof payload.total_duration_ms === 'number'
          ? payload.total_duration_ms
          : typeof payload.duration_ms === 'number'
            ? payload.duration_ms
            : undefined,
      linesAdded:
        typeof payload.lines_added === 'number'
          ? payload.lines_added
          : undefined,
      linesRemoved:
        typeof payload.lines_removed === 'number'
          ? payload.lines_removed
          : undefined,
      filesModified:
        typeof payload.files_modified === 'number'
          ? payload.files_modified
          : undefined,
      lockContentions:
        typeof payload.lock_contentions === 'number'
          ? payload.lock_contentions
          : undefined,
      totalLockWaitMs:
        typeof payload.total_lock_wait_ms === 'number'
          ? payload.total_lock_wait_ms
          : undefined,
      coverageGapNotice:
        typeof payload.coverage_gap_notice === 'string'
          ? payload.coverage_gap_notice
          : undefined,
    };

    const agents = payload[listKey];
    if (Array.isArray(agents)) {
      artifact.agents = agents.flatMap((entry) => {
        if (!entry || typeof entry !== 'object') return [];
        const record = entry as Record<string, unknown>;
        const taskId =
          typeof record.agent === 'string'
            ? record.agent
            : typeof record.task_id === 'string'
              ? record.task_id
              : kind;
        const summary =
          typeof record.summary === 'string'
            ? record.summary
            : typeof record.output === 'string'
              ? record.output
              : undefined;
        const error =
          typeof record.error === 'string' ? record.error : undefined;
        return [
          {
            taskId,
            name: delegatedDisplayName(taskId, defaultAgentName(kind, args)),
            status: error ? ('failed' as const) : ('complete' as const),
            outcomeReason:
              typeof record.outcome_reason === 'string'
                ? record.outcome_reason
                : undefined,
            toolCount:
              typeof record.tool_calls === 'number'
                ? record.tool_calls
                : typeof record.turns_used === 'number'
                  ? record.turns_used
                  : 0,
            tokens: 0,
            currentAction: summary?.slice(0, 120),
            completionSummary: summary,
            linesAdded:
              typeof record.lines_added === 'number' ? record.lines_added : 0,
            linesRemoved:
              typeof record.lines_removed === 'number'
                ? record.lines_removed
                : 0,
            completedPlanTask:
              typeof record.completed_plan_task === 'string'
                ? record.completed_plan_task
                : undefined,
          },
        ];
      });
    }

    return artifact;
  } catch {
    return undefined;
  }
}

export function applyDelegatedProgress(
  toolCall: ToolCall,
  event: DelegatedProgressEvent,
): ToolCall {
  const delegatedKind = resolveDelegatedKind(
    toolCall.name,
    toolCall.arguments,
    event.kind,
  ) ?? event.kind;
  const delegated = toolCall.delegated
    ? { ...toolCall.delegated, agents: [...toolCall.delegated.agents] }
    : createDelegatedArtifactState(delegatedKind, toolCall.arguments);
  delegated.delegatedRunId = event.delegated_run_id;
  delegated.stage = event.stage;
  delegated.kind = delegatedKind;
  const index = delegated.agents.findIndex((agent) => agent.taskId === event.task_id);
  const agent: DelegatedAgentState = {
    taskId: event.task_id,
    name: event.agent_name,
    status: event.status as DelegatedAgentState['status'],
    outcomeReason: undefined,
    toolCount: event.tool_count,
    tokens: event.tokens,
    currentAction: event.current_action || undefined,
    completionSummary: event.completion_summary || undefined,
    linesAdded: event.lines_added,
    linesRemoved: event.lines_removed,
    completedPlanTask: event.completed_plan_task || undefined,
  };

  if (index >= 0) {
    delegated.agents[index] = agent;
  } else {
    delegated.agents.push(agent);
  }

  delegated.agentCount = Math.max(
    delegated.agentCount || 0,
    delegated.agents.length,
  );
  const successfulAgents = delegated.agents.filter(
    (entry) => entry.status === 'complete',
  ).length;
  const failedAgents = delegated.agents.filter(
    (entry) => entry.status === 'failed',
  ).length;
  delegated.successfulAgents = successfulAgents;
  delegated.failedAgents = failedAgents;
  delegated.outcome =
    failedAgents === 0
      ? 'success'
      : successfulAgents === 0
        ? 'failed'
        : 'partial';
  return {
    ...toolCall,
    delegatedRunId: event.delegated_run_id,
    delegated: annotateDelegatedArtifactState(delegated),
  };
}

export function applyDelegatedSessionState(
  messages: ChatMessage[],
  delegatedTools: DelegatedToolStateResponse[] | null | undefined,
  recentRuns?: DelegatedRunResponse[] | null | undefined,
): ChatMessage[] {
  if (
    (!delegatedTools || delegatedTools.length === 0)
    && (!recentRuns || recentRuns.length === 0)
  ) {
    return messages;
  }

  const delegatedByToolCall = new Map(
    (delegatedTools || []).map((snapshot) => [snapshot.tool_call_id, snapshot]),
  );
  const recentRunByToolCall = new Map(
    (recentRuns || [])
      .filter(
        (run) =>
          typeof run.parent_tool_call_id === 'string'
          && run.parent_tool_call_id.length > 0,
      )
      .map((run) => [run.parent_tool_call_id as string, run]),
  );

  return messages.map((message) => ({
    ...message,
    toolCalls: message.toolCalls?.map((toolCall) => {
      const snapshot = delegatedByToolCall.get(toolCall.id);
      const recentRun = recentRunByToolCall.get(toolCall.id);
      const delegatedKind = resolveDelegatedKind(
        toolCall.name,
        toolCall.arguments,
        snapshot?.kind ?? recentRun?.kind,
        recentRun?.capabilities,
      );
      if (!delegatedKind) return toolCall;

      if (!snapshot) {
        const delegated = toolCall.delegated
          ? { ...toolCall.delegated }
          : createDelegatedArtifactState(delegatedKind, toolCall.arguments);
        delegated.kind = delegatedKind;
        if (recentRun?.stage) {
          delegated.stage = recentRun.stage;
        }
        if (recentRun?.child_name) {
          delegated.name = recentRun.child_name;
          if (delegated.agents[0]) delegated.agents[0].name = recentRun.child_name;
        }
        if (recentRun?.capabilities) {
          delegated.capabilities = recentRun.capabilities;
        }
        return {
          ...toolCall,
          delegatedRunId:
            recentRun?.delegated_run_id || toolCall.delegatedRunId,
          delegated,
        };
      }

      const delegated: DelegatedArtifactState = {
        kind: delegatedKind,
        name: recentRun?.child_name || toolCall.delegated?.name,
        capabilities:
          recentRun?.capabilities || toolCall.delegated?.capabilities,
        delegatedRunId:
          snapshot.delegated_run_id
          || recentRun?.delegated_run_id
          || toolCall.delegatedRunId,
        stage: snapshot.stage,
        agents: snapshot.agents.map((agent) => ({
          taskId: agent.task_id,
          name: agent.agent_name,
          status: agent.status as DelegatedAgentState['status'],
          outcomeReason: undefined,
          toolCount: agent.tool_count,
          tokens: agent.tokens,
          currentAction: agent.current_action || undefined,
          completionSummary: agent.completion_summary || undefined,
          linesAdded: agent.lines_added,
          linesRemoved: agent.lines_removed,
          completedPlanTask: agent.completed_plan_task || undefined,
        })),
        thinking: toolCall.delegated?.thinking,
        filesExamined: toolCall.delegated?.filesExamined || [],
        errors: toolCall.delegated?.errors || [],
        agentCount: Math.max(
          toolCall.delegated?.agentCount || 0,
          snapshot.agents.length,
        ),
        usableAgents: snapshot.agents.filter((agent) => agent.status === 'complete')
          .length,
        degradedAgents: 0,
        successfulAgents: snapshot.agents.filter((agent) => agent.status === 'complete')
          .length,
        failedAgents: snapshot.agents.filter((agent) => agent.status === 'failed')
          .length,
        filesExaminedCount:
          toolCall.delegated?.filesExaminedCount
          || toolCall.delegated?.filesExamined?.length
          || 0,
        totalTargets:
          toolCall.delegated?.totalTargets
          || toolCall.delegated?.agentCount
          || snapshot.agents.length,
        outcome: snapshot.agents.some((agent) => agent.status === 'failed')
          ? snapshot.agents.some((agent) => agent.status === 'complete')
            ? 'partial'
            : 'failed'
          : 'success',
      };

      return {
        ...toolCall,
        delegatedRunId:
          snapshot.delegated_run_id
          || recentRun?.delegated_run_id
          || toolCall.delegatedRunId,
        delegated: mergeDelegatedArtifactState(toolCall.delegated, delegated),
      };
    }),
  }));
}
