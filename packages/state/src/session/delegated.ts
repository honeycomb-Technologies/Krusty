import type {
  DelegatedProgressEvent,
  DelegatedRunResponse,
  DelegatedRunSummaryResponse,
  DelegatedRunStage,
  DelegatedToolKind,
  DelegatedToolStateResponse,
} from '@krusty/api';

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

function delegatedOutcomeForStage(
  stage: DelegatedRunStage | undefined,
): DelegatedArtifactState['outcome'] {
  switch (stage) {
    case 'complete':
      return 'success';
    case 'degraded':
      return 'partial';
    case 'failed':
      return 'failed';
    case 'cancelled':
      return 'cancelled';
    default:
      return undefined;
  }
}

function isTerminalDelegatedStage(
  stage: DelegatedRunStage | undefined,
): stage is Extract<
  DelegatedRunStage,
  'complete' | 'degraded' | 'failed' | 'cancelled'
> {
  return stage === 'complete'
    || stage === 'degraded'
    || stage === 'failed'
    || stage === 'cancelled';
}

function toolCallStatusForDelegatedStage(
  stage: DelegatedRunStage | undefined,
  current: ToolCall['status'],
): ToolCall['status'] {
  switch (stage) {
    case 'created':
    case 'running':
    case 'synthesizing':
      return 'running';
    case 'complete':
      return 'success';
    case 'degraded':
      return 'partial';
    case 'failed':
    case 'cancelled':
      return 'error';
    default:
      return current;
  }
}

function delegatedAgentStatus(record: Record<string, unknown>): DelegatedAgentState['status'] {
  const success = typeof record.success === 'boolean' ? record.success : undefined;
  const usableEvidence =
    typeof record.usable_evidence === 'boolean' ? record.usable_evidence : undefined;
  const termination = typeof record.termination === 'string' ? record.termination : undefined;
  const degradedSuccess = record.degraded_success === true
    || (usableEvidence === true
      && (termination === 'provider_max_tokens' || termination === 'provider_timeout'));

  if (degradedSuccess) return 'degraded';
  if (termination === 'cancelled' || record.cancelled === true) return 'cancelled';
  if (typeof record.error === 'string' || success === false || usableEvidence === false) {
    return 'failed';
  }
  return 'complete';
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
    if (Array.isArray(authoritativeCapabilities)) {
      const capabilities = authoritativeCapabilities.filter(
        (value): value is string => typeof value === 'string',
      );
      if (capabilities.length > 0) {
        return capabilities.includes('write') ? 'build' : 'explore';
      }
      // Migration 51 represents pre-contract rows as an empty capability
      // array. Their persisted role/kind remains authoritative; do not turn
      // every legacy builder into an explorer or trust stale raw call labels.
      if (fallbackKind) return fallbackKind;
    }

    if (Array.isArray(args?.capabilities)) {
      const capabilities = args.capabilities.filter(
        (value): value is string => typeof value === 'string',
      );
      if (capabilities.length > 0) {
        return capabilities.includes('write') ? 'build' : 'explore';
      }
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
  const degradedAgents =
    typeof payload.degraded_agents === 'number' ? payload.degraded_agents : undefined;
  const cancelledAgents =
    typeof payload.cancelled_agents === 'number' ? payload.cancelled_agents : undefined;
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
    degradedAgents !== undefined ? `${degradedAgents} degraded` : undefined,
    cancelledAgents !== undefined ? `${cancelledAgents} cancelled` : undefined,
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
    (agent) => agent.status === 'complete' || agent.status === 'degraded',
  ).length;
  const pendingTargets = Math.max(
    totalTargets
      - activeTargets
      - completedTargets
      - artifact.agents.filter(
        (agent) => agent.status === 'failed' || agent.status === 'cancelled',
      ).length,
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
        payload.status === 'background_started'
          ? 'running'
          : payload.outcome === 'success'
          ? 'complete'
          : payload.outcome === 'partial'
            ? 'degraded'
            : payload.outcome === 'failed'
              ? 'failed'
              : payload.outcome === 'cancelled'
                ? 'cancelled'
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
        || payload.outcome === 'cancelled'
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
      agents:
        payload.status === 'background_started'
          ? buildSeedDelegatedAgents(kind, args)
          : [],
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
      cancelledAgents:
        typeof payload.cancelled_agents === 'number'
          ? payload.cancelled_agents
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

    const preferredAgents = kind === 'build' ? payload.builders : payload.agents;
    const fallbackAgents = kind === 'build' ? payload.agents : payload.builders;
    const agents = Array.isArray(preferredAgents) && preferredAgents.length > 0
      ? preferredAgents
      : fallbackAgents;
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
        const success = typeof record.success === 'boolean' ? record.success : undefined;
        const usableEvidence =
          typeof record.usable_evidence === 'boolean'
            ? record.usable_evidence
            : undefined;
        const termination =
          typeof record.termination === 'string' ? record.termination : undefined;
        const degradedSuccess =
          typeof record.degraded_success === 'boolean'
            ? record.degraded_success
            : (usableEvidence === true
                && (termination === 'provider_max_tokens' || termination === 'provider_timeout'));
        return [
          {
            taskId,
            name: delegatedDisplayName(taskId, defaultAgentName(kind, args)),
            status: delegatedAgentStatus(record),
            success,
            usableEvidence,
            degradedSuccess,
            termination,
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

      const hasPerAgentSuccess = agents.some(
        (entry) => entry && typeof entry === 'object' && 'success' in entry,
      );
      const hasPerAgentUsability = agents.some(
        (entry) => entry && typeof entry === 'object' && 'usable_evidence' in entry,
      );
      const hasPerAgentDegradation = agents.some(
        (entry) => entry && typeof entry === 'object' && 'degraded_success' in entry,
      );
      if (hasPerAgentSuccess) {
        artifact.successfulAgents = artifact.agents.filter(
          (agent) => agent.success === true && agent.status !== 'degraded',
        ).length;
        artifact.failedAgents = artifact.agents.filter(
          (agent) => agent.status === 'failed',
        ).length;
      }
      if (hasPerAgentUsability) {
        artifact.usableAgents = artifact.agents.filter(
          (agent) => agent.usableEvidence === true,
        ).length;
      }
      if (hasPerAgentDegradation || artifact.agents.some((agent) => agent.status === 'degraded')) {
        artifact.degradedAgents = artifact.agents.filter(
          (agent) => agent.status === 'degraded',
        ).length;
      }
      if (artifact.agents.some((agent) => agent.status === 'cancelled')) {
        artifact.cancelledAgents = artifact.agents.filter(
          (agent) => agent.status === 'cancelled',
        ).length;
      }
      artifact.agentCount ??= artifact.agents.length;
    }

    return annotateDelegatedArtifactState(artifact);
  } catch {
    return undefined;
  }
}

export function applyDelegatedProgress(
  toolCall: ToolCall,
  event: DelegatedProgressEvent,
): ToolCall {
  const currentRunId = toolCall.delegated?.delegatedRunId || toolCall.delegatedRunId;
  if (
    isTerminalDelegatedStage(toolCall.delegated?.stage)
    && !isTerminalDelegatedStage(event.stage)
    && (!currentRunId || currentRunId === event.delegated_run_id)
  ) {
    // A late process-local progress frame must never reopen a run whose tool
    // result or durable hydration already established a terminal outcome.
    return toolCall;
  }

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
  const degradedAgents = delegated.agents.filter(
    (entry) => entry.status === 'degraded',
  ).length;
  const failedAgents = delegated.agents.filter(
    (entry) => entry.status === 'failed',
  ).length;
  const cancelledAgents = delegated.agents.filter(
    (entry) => entry.status === 'cancelled',
  ).length;
  delegated.successfulAgents = successfulAgents;
  delegated.usableAgents = successfulAgents + degradedAgents;
  delegated.degradedAgents = degradedAgents;
  delegated.cancelledAgents = cancelledAgents;
  delegated.failedAgents = failedAgents;
  delegated.outcome = delegatedOutcomeForStage(event.stage);
  return {
    ...toolCall,
    delegatedRunId: event.delegated_run_id,
    delegated: annotateDelegatedArtifactState(delegated),
    status: toolCallStatusForDelegatedStage(event.stage, toolCall.status),
  };
}

function parseRecentDelegatedRunArtifact(
  toolCall: ToolCall,
  recentRun: DelegatedRunResponse,
  kind: DelegatedToolKind,
): DelegatedArtifactState | undefined {
  if (!recentRun.artifact) return undefined;

  const artifact = parseDelegatedArtifactState(
    toolCall.name,
    JSON.stringify(recentRun.artifact),
    toolCall.arguments,
    kind,
  );
  if (!artifact) return undefined;

  artifact.kind = kind;
  artifact.delegatedRunId = recentRun.delegated_run_id;
  artifact.stage = recentRun.stage;
  artifact.outcome = delegatedOutcomeForStage(recentRun.stage) ?? artifact.outcome;
  artifact.humanReview = recentRun.human_review || artifact.humanReview;
  artifact.name = recentRun.child_name || artifact.name;
  if (recentRun.capabilities && recentRun.capabilities.length > 0) {
    artifact.capabilities = recentRun.capabilities;
  }
  if (recentRun.child_name && artifact.agents.length === 1) {
    artifact.agents[0].name = recentRun.child_name;
  }

  return annotateDelegatedArtifactState(artifact);
}

export function applyDelegatedSessionState(
  messages: ChatMessage[],
  delegatedTools: DelegatedToolStateResponse[] | null | undefined,
  recentRuns?: DelegatedRunResponse[] | null | undefined,
  runSummaries?: DelegatedRunSummaryResponse[] | null | undefined,
): ChatMessage[] {
  if (
    (!delegatedTools || delegatedTools.length === 0)
    && (!recentRuns || recentRuns.length === 0)
    && (!runSummaries || runSummaries.length === 0)
  ) {
    return messages;
  }

  const delegatedByToolCall = new Map(
    (delegatedTools || []).map((snapshot) => [snapshot.tool_call_id, snapshot]),
  );
  const recentRunByToolCall = new Map<string, DelegatedRunResponse>();
  for (const run of recentRuns || []) {
    const toolCallId = run.parent_tool_call_id;
    if (!toolCallId) continue;
    const existing = recentRunByToolCall.get(toolCallId);
    if (
      !existing
      || run.updated_at > existing.updated_at
      || (
        run.updated_at === existing.updated_at
        && run.delegated_run_id > existing.delegated_run_id
      )
    ) {
      recentRunByToolCall.set(toolCallId, run);
    }
  }
  const summaryByToolCall = new Map<string, DelegatedRunSummaryResponse>();
  for (const summary of runSummaries || []) {
    const existing = summaryByToolCall.get(summary.parent_tool_call_id);
    if (
      !existing
      || summary.updated_at > existing.updated_at
      || (
        summary.updated_at === existing.updated_at
        && summary.delegated_run_id > existing.delegated_run_id
      )
    ) {
      summaryByToolCall.set(summary.parent_tool_call_id, summary);
    }
  }

  return messages.map((message) => ({
    ...message,
    toolCalls: message.toolCalls?.map((toolCall) => {
      const unverifiedSnapshot = delegatedByToolCall.get(toolCall.id);
      const recentRun = recentRunByToolCall.get(toolCall.id);
      // New servers expose a compact, newest-per-tool durable index. It is the
      // lifecycle authority even when the full artifact has aged out of the
      // small recent window. Older servers fall back to the recent full row.
      const durableRun = summaryByToolCall.get(toolCall.id) ?? recentRun;
      const snapshot = unverifiedSnapshot && (
        !durableRun
        || durableRun.delegated_run_id === unverifiedSnapshot.delegated_run_id
      )
        ? unverifiedSnapshot
        : undefined;
      const durableArtifactRun = recentRun?.delegated_run_id === durableRun?.delegated_run_id
        ? recentRun
        : undefined;
      const delegatedKind = resolveDelegatedKind(
        toolCall.name,
        toolCall.arguments,
        snapshot?.kind ?? durableRun?.kind,
        durableRun?.capabilities,
      );
      if (!delegatedKind) return toolCall;

      const currentRunId = toolCall.delegated?.delegatedRunId || toolCall.delegatedRunId;
      const sameCurrentRun = !currentRunId
        || !durableRun
        || currentRunId === durableRun.delegated_run_id;
      const durableStage = durableRun?.stage;
      const currentTerminalStage = sameCurrentRun
        && isTerminalDelegatedStage(toolCall.delegated?.stage)
        ? toolCall.delegated?.stage
        : undefined;

      if (!snapshot) {
        let delegated = toolCall.delegated
          ? { ...toolCall.delegated }
          : createDelegatedArtifactState(delegatedKind, toolCall.arguments);
        const durableArtifact = durableArtifactRun
          ? parseRecentDelegatedRunArtifact(toolCall, durableArtifactRun, delegatedKind)
          : undefined;
        if (durableArtifact) {
          delegated = mergeDelegatedArtifactState(delegated, durableArtifact);
          if (durableArtifact.agents.length > 0) {
            // A durable terminal artifact is authoritative; do not retain
            // optimistic launch rows that never received a matching live ID.
            delegated.agents = durableArtifact.agents;
          }
        }
        delegated.kind = delegatedKind;
        const canonicalStage = isTerminalDelegatedStage(durableStage)
          ? durableStage
          : currentTerminalStage ?? durableStage;
        if (canonicalStage) {
          delegated.stage = canonicalStage;
          delegated.outcome = delegatedOutcomeForStage(canonicalStage);
        }
        if (durableRun?.child_name) {
          delegated.name = durableRun.child_name;
          if (delegated.agents.length === 1) {
            delegated.agents[0].name = durableRun.child_name;
          }
        }
        if (durableRun?.capabilities && durableRun.capabilities.length > 0) {
          delegated.capabilities = durableRun.capabilities;
        }
        return {
          ...toolCall,
          delegatedRunId:
            durableRun?.delegated_run_id || toolCall.delegatedRunId,
          delegated: annotateDelegatedArtifactState(delegated),
          status: toolCallStatusForDelegatedStage(canonicalStage, toolCall.status),
        };
      }

      const canonicalStage = isTerminalDelegatedStage(durableStage)
        ? durableStage
        : currentTerminalStage ?? snapshot.stage;

      const delegated: DelegatedArtifactState = {
        kind: delegatedKind,
        name: durableRun?.child_name || toolCall.delegated?.name,
        capabilities:
          durableRun?.capabilities && durableRun.capabilities.length > 0
            ? durableRun.capabilities
            : toolCall.delegated?.capabilities,
        delegatedRunId:
          snapshot.delegated_run_id
          || durableRun?.delegated_run_id
          || toolCall.delegatedRunId,
        stage: canonicalStage,
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
        usableAgents: snapshot.agents.filter(
          (agent) => agent.status === 'complete' || agent.status === 'degraded',
        ).length,
        degradedAgents: snapshot.agents.filter((agent) => agent.status === 'degraded')
          .length,
        cancelledAgents: snapshot.agents.filter(
          (agent) => agent.status === 'cancelled',
        ).length,
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
        outcome: delegatedOutcomeForStage(canonicalStage),
      };

      let canonicalDelegated = mergeDelegatedArtifactState(toolCall.delegated, delegated);
      const durableArtifact = durableArtifactRun
        ? parseRecentDelegatedRunArtifact(toolCall, durableArtifactRun, delegatedKind)
        : undefined;
      if (durableArtifact && isTerminalDelegatedStage(canonicalStage)) {
        canonicalDelegated = mergeDelegatedArtifactState(
          canonicalDelegated,
          durableArtifact,
        );
        if (durableArtifact.agents.length > 0) {
          canonicalDelegated.agents = durableArtifact.agents;
        }
        canonicalDelegated.stage = canonicalStage;
        canonicalDelegated.outcome = delegatedOutcomeForStage(canonicalStage);
      }

      return {
        ...toolCall,
        delegatedRunId:
          snapshot.delegated_run_id
          || durableRun?.delegated_run_id
          || toolCall.delegatedRunId,
        delegated: annotateDelegatedArtifactState(canonicalDelegated),
        status: toolCallStatusForDelegatedStage(canonicalStage, toolCall.status),
      };
    }),
  }));
}
