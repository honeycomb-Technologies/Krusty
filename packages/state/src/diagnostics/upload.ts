import type {
  DiagnosticBatch,
  DiagnosticEvent,
  DiagnosticNativePayload,
  DiagnosticUploadBatch,
  DiagnosticUploadMetadata,
} from './types';

const MAX_NATIVE_PAYLOADS_PER_BATCH = 16;
const MAX_NATIVE_SUMMARY_BYTES = 16 * 1024;
const MAX_UPLOAD_BATCH_BYTES = 480 * 1024;
const MAX_METRICKIT_DIAGNOSTICS = 8;
const MAX_METRICKIT_STACKS = 8;
const MAX_METRICKIT_FRAMES_PER_STACK = 32;
const MAX_METRICKIT_FRAMES = 256;
const MAX_METRICKIT_SAMPLE_COUNT = 1_000_000;
const MAX_U64_DECIMAL = '18446744073709551615';

export function buildDiagnosticUploadBatch(
  batch: DiagnosticBatch,
  metadata: DiagnosticUploadMetadata,
  nativePayloads: readonly DiagnosticNativePayload[] = [],
): DiagnosticUploadBatch {
  if (nativePayloads.length > MAX_NATIVE_PAYLOADS_PER_BATCH) {
    throw new Error('Mobile diagnostic upload exceeds the native payload count limit');
  }
  const upload: DiagnosticUploadBatch = {
    run: {
      id: batch.runId,
      installation_id: batch.installationId,
      app_version: safeMetadata(metadata.appVersion, 'unknown'),
      build_number: safeMetadata(metadata.buildNumber, 'unknown'),
      platform: metadata.platform,
      os_version: safeMetadata(metadata.osVersion, 'unknown'),
      device_class: safeMetadata(metadata.deviceClass, 'mobile'),
      capture_level: metadata.captureLevel,
      started_at_ms: batch.runStartedAtMs,
      ended_at_ms: metadata.completed
        ? Math.max(batch.runStartedAtMs, metadata.endedAtMs ?? batch.createdAtMs)
        : null,
      dropped_event_count: 0,
    },
    events: batch.events.map((event) => ({
      sequence: event.sequence,
      occurred_at_ms: event.atMs,
      monotonic_ms: Math.max(0, event.atMs - batch.runStartedAtMs),
      category: eventCategory(event),
      name: eventName(event),
      duration_ms: event.fields.durationMs ?? null,
      severity: eventSeverity(event),
      attributes: eventAttributes(event),
    })),
    native_payloads: nativePayloads.map((payload) => ({
      payload_id: tokenLabel(payload.id),
      kind: payload.kind,
      received_at_ms: Math.max(0, Math.floor(payload.receivedAtMs)),
      payload_json: serializeNativeSummary(payload),
    })),
    completed: Boolean(metadata.completed),
  };

  if (utf8ByteLength(JSON.stringify(upload)) > MAX_UPLOAD_BATCH_BYTES) {
    throw new Error('Mobile diagnostic upload exceeds the safe request budget');
  }
  return upload;
}

function serializeNativeSummary(payload: DiagnosticNativePayload): string {
  const summary = payload.summarySchemaVersion === 1
    ? serializeNativeV1Summary(payload)
    : serializeNativeV2Summary(payload);
  const encoded = JSON.stringify(summary);
  if (utf8ByteLength(encoded) > MAX_NATIVE_SUMMARY_BYTES) {
    throw new Error('MetricKit summary exceeds the safe payload budget');
  }
  return encoded;
}

function serializeNativeV1Summary(payload: Extract<DiagnosticNativePayload, { summarySchemaVersion: 1 }>) {
  return {
    schema_version: 1,
    source_payload_bytes: boundedInteger(payload.sourcePayloadBytes, 0, 2 * 1024 * 1024),
    has_application_launch_metrics: Boolean(payload.hasApplicationLaunchMetrics),
    has_application_responsiveness_metrics: Boolean(payload.hasApplicationResponsivenessMetrics),
    has_memory_metrics: Boolean(payload.hasMemoryMetrics),
    has_cpu_metrics: Boolean(payload.hasCpuMetrics),
    has_disk_io_metrics: Boolean(payload.hasDiskIoMetrics),
    has_display_metrics: Boolean(payload.hasDisplayMetrics),
    has_network_transfer_metrics: Boolean(payload.hasNetworkTransferMetrics),
    has_application_exit_metrics: Boolean(payload.hasApplicationExitMetrics),
    has_cellular_condition_metrics: Boolean(payload.hasCellularConditionMetrics),
    has_location_activity_metrics: Boolean(payload.hasLocationActivityMetrics),
    has_animation_metrics: Boolean(payload.hasAnimationMetrics),
    crash_diagnostic_count: boundedInteger(payload.crashDiagnosticCount, 0, 1_000),
    hang_diagnostic_count: boundedInteger(payload.hangDiagnosticCount, 0, 1_000),
    cpu_exception_diagnostic_count: boundedInteger(payload.cpuExceptionDiagnosticCount, 0, 1_000),
    disk_write_exception_diagnostic_count: boundedInteger(
      payload.diskWriteExceptionDiagnosticCount,
      0,
      1_000,
    ),
  };
}

function serializeNativeV2Summary(payload: Extract<DiagnosticNativePayload, { summarySchemaVersion: 2 }>) {
  const periodStartMs = requiredInteger(payload.periodStartMs, 1, Number.MAX_SAFE_INTEGER, 'period start');
  const periodEndMs = requiredInteger(payload.periodEndMs, periodStartMs, Number.MAX_SAFE_INTEGER, 'period end');
  if (payload.diagnostics.length === 0 || payload.diagnostics.length > MAX_METRICKIT_DIAGNOSTICS) {
    throw new Error('MetricKit diagnostic count is out of bounds');
  }

  let totalFrames = 0;
  const diagnostics = payload.diagnostics.map((diagnostic) => {
    if (!['crash', 'hang', 'cpu_exception', 'disk_write_exception'].includes(diagnostic.type)) {
      throw new Error('MetricKit diagnostic type is invalid');
    }
    if (diagnostic.stacks.length === 0 || diagnostic.stacks.length > MAX_METRICKIT_STACKS) {
      throw new Error('MetricKit stack count is out of bounds');
    }
    return {
      type: diagnostic.type,
      app_version: nativeLabel(diagnostic.appVersion, 32, 'app version'),
      build_version: nativeLabel(diagnostic.buildVersion, 32, 'build version'),
      architecture: nativeLabel(diagnostic.architecture, 16, 'architecture'),
      stacks: diagnostic.stacks.map((stack) => {
        if (!/^[0-9a-f]{64}$/.test(stack.fingerprintSha256)) {
          throw new Error('MetricKit stack fingerprint is invalid');
        }
        if (stack.frames.length === 0 || stack.frames.length > MAX_METRICKIT_FRAMES_PER_STACK) {
          throw new Error('MetricKit frame count is out of bounds');
        }
        totalFrames += stack.frames.length;
        if (totalFrames > MAX_METRICKIT_FRAMES) {
          throw new Error('MetricKit payload exceeds the aggregate frame limit');
        }
        return {
          fingerprint_sha256: stack.fingerprintSha256,
          thread_attributed: Boolean(stack.threadAttributed),
          frames: stack.frames.map((frame) => ({
            binary_uuid: canonicalUuid(frame.binaryUuid),
            binary_name: nativeBasename(frame.binaryName),
            offset: decimalU64(frame.offset),
            sample_count: requiredInteger(
              frame.sampleCount,
              0,
              MAX_METRICKIT_SAMPLE_COUNT,
              'sample count',
            ),
          })),
        };
      }),
    };
  });

  return {
    schema_version: 2,
    period_start_ms: periodStartMs,
    period_end_ms: periodEndMs,
    diagnostics,
  };
}

function requiredInteger(value: number, minimum: number, maximum: number, field: string): number {
  if (!Number.isSafeInteger(value) || value < minimum || value > maximum) {
    throw new Error(`MetricKit ${field} is invalid`);
  }
  return value;
}

function nativeLabel(value: string, maximumBytes: number, field: string): string {
  if (!/^[a-zA-Z0-9._-]+$/.test(value) || utf8ByteLength(value) > maximumBytes) {
    throw new Error(`MetricKit ${field} is invalid`);
  }
  return value;
}

function nativeBasename(value: string): string {
  if (
    !value ||
    utf8ByteLength(value) > 96 ||
    /[\\/\u0000-\u001f\u007f]/.test(value) ||
    value.includes('://') ||
    value.includes('?')
  ) {
    throw new Error('MetricKit binary name is invalid');
  }
  return value;
}

function canonicalUuid(value: string): string {
  if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(value)) {
    throw new Error('MetricKit binary UUID is invalid');
  }
  return value;
}

function decimalU64(value: string): string {
  if (!/^(0|[1-9][0-9]*)$/.test(value)) {
    throw new Error('MetricKit frame offset is invalid');
  }
  if (value.length > MAX_U64_DECIMAL.length ||
      (value.length === MAX_U64_DECIMAL.length && value > MAX_U64_DECIMAL)) {
    throw new Error('MetricKit frame offset is invalid');
  }
  return value;
}

function boundedInteger(value: number, minimum: number, maximum: number): number {
  if (!Number.isFinite(value)) return minimum;
  return Math.min(maximum, Math.max(minimum, Math.floor(value)));
}

function utf8ByteLength(value: string): number {
  let bytes = 0;
  for (let index = 0; index < value.length; index += 1) {
    const codeUnit = value.charCodeAt(index);
    if (codeUnit < 0x80) {
      bytes += 1;
    } else if (codeUnit < 0x800) {
      bytes += 2;
    } else if (codeUnit >= 0xd800 && codeUnit <= 0xdbff && index + 1 < value.length) {
      const next = value.charCodeAt(index + 1);
      if (next >= 0xdc00 && next <= 0xdfff) {
        bytes += 4;
        index += 1;
      } else {
        bytes += 3;
      }
    } else {
      bytes += 3;
    }
  }
  return bytes;
}

function eventCategory(event: DiagnosticEvent): string {
  switch (event.type) {
    case 'mode':
    case 'navigation':
      return 'navigation';
    case 'request':
      return 'network';
    case 'heartbeat':
    case 'longtask':
    case 'event_timing':
    case 'performance':
      return 'runtime';
    case 'resource':
      return 'resource';
    case 'webview':
      return 'webview';
    case 'live_activity':
      return 'live_activity';
    default:
      return 'app';
  }
}

function eventName(event: DiagnosticEvent): string {
  if (event.type === 'longtask') return 'long_task';
  if (event.type === 'heartbeat') return 'heartbeat_stall';
  if (event.type === 'event_timing') return 'event_timing';
  if (event.type === 'navigation') return 'route_change';
  if (event.type === 'mode') return 'mode_switch';
  if (event.type === 'webview' && event.fields.outcome === 'terminate') return 'terminated';
  return tokenLabel(event.fields.name ?? `${event.type}.${event.fields.outcome ?? 'event'}`);
}

function eventSeverity(event: DiagnosticEvent): 'info' | 'warning' | 'error' {
  if (event.fields.outcome === 'error') return 'error';
  if (event.type === 'longtask' || event.type === 'heartbeat' || event.fields.outcome === 'terminate') {
    return 'warning';
  }
  return 'info';
}

function eventAttributes(event: DiagnosticEvent): Record<string, string | number | boolean> {
  const attributes: Record<string, string | number | boolean> = {};
  const { fields } = event;
  if (event.type === 'navigation' && fields.name) attributes.route = fields.name;
  if (event.type === 'mode') attributes.mode = fields.name ?? fields.state ?? 'mode.change';
  if (event.type === 'performance' && fields.name) attributes.phase = fields.name;
  if (event.type === 'resource' && fields.name) attributes.resource = fields.name;
  if (event.type === 'request' && fields.name) attributes.source = fields.name;
  if (fields.surface) attributes.surface = fields.surface;
  if (fields.state) attributes.state = fields.state;
  if (fields.outcome) attributes.outcome = fields.outcome;
  if (fields.code) attributes.code = fields.code;
  if (fields.count !== undefined) attributes.count = fields.count;
  return attributes;
}

function tokenLabel(value: string): string {
  const token = value.replace(/[^a-zA-Z0-9_.:-]/g, '_').slice(0, 64);
  return token || 'diagnostic.event';
}

function safeMetadata(value: string, fallback: string): string {
  const clean = value.trim().slice(0, 64);
  if (!clean || clean.includes('://') || clean.includes('?') || clean.includes('\\')) {
    return fallback;
  }
  return clean;
}
