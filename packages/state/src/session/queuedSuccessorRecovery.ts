import type { MitsuroStorage } from "../storage";
import type {
  Attachment,
  ChatMessage,
  QueuedMessage,
  QueuedSuccessorClaimInput,
  SendMessageOptions,
} from "./types";
import { MAX_QUEUED_MESSAGES } from "./constants";

const STORAGE_VERSION = 1;
const MAX_RECOVERY_SESSIONS = 8;
const MAX_RECORD_BYTES = 384 * 1024;
const MAX_ENVELOPE_BYTES = 3 * 1024 * 1024;

export type QueuedSuccessorPhase =
  | "pending"
  | "claiming"
  | "in_flight"
  | "uncertain"
  | "rejected"
  | "accepted";

export interface QueuedWorkerInputIdentity {
  /** Missing only on legacy v1 records, which are Chat retries. */
  operation?: "chat" | "steer";
  fingerprint: string;
  key: string;
}

function cloneWorkerInput(
  workerInput: QueuedWorkerInputIdentity | undefined,
): QueuedWorkerInputIdentity | undefined {
  return workerInput ? { ...workerInput } : undefined;
}

function cloneQueuedWorkerInput(
  workerInput: QueuedMessage["workerInput"],
): QueuedMessage["workerInput"] {
  return workerInput ? { ...workerInput } : undefined;
}

export interface QueuedSuccessorRecoveryRecord {
  version: 1;
  id: string;
  sessionId: string;
  queuedMessages: QueuedMessage[];
  rows: ChatMessage[];
  phase: QueuedSuccessorPhase;
  /** Exact leading batch owned by the current/last transport attempt. */
  claimedMessageIds?: string[];
  workerInput?: QueuedWorkerInputIdentity;
  updatedAt: number;
}

/** Process-local authority for one exact delivery attempt. Never persisted. */
export interface QueuedSuccessorRecoveryClaim
  extends QueuedSuccessorRecoveryRecord {
  attemptToken: string;
}

interface RecoveryEnvelope {
  version: 1;
  records: QueuedSuccessorRecoveryRecord[];
}

function utf8Bytes(value: string): number {
  return new TextEncoder().encode(value).byteLength;
}

function cloneAttachment(attachment: Attachment): Attachment {
  return {
    name: attachment.name,
    type: attachment.type,
    mimeType: attachment.mimeType,
    uri: attachment.uri,
    base64: attachment.base64,
    text: attachment.text,
  };
}

function cloneSendOptions(
  sendOptions: SendMessageOptions | undefined,
): SendMessageOptions | undefined {
  if (!sendOptions) return undefined;
  return {
    projectDir: sendOptions.projectDir,
    workingDir: sendOptions.workingDir,
    workspaceMode: sendOptions.workspaceMode,
    sessionType: sendOptions.sessionType,
    targetBranch: sendOptions.targetBranch,
    hiveConversationKind: sendOptions.hiveConversationKind,
  };
}

function cloneQueuedMessage(message: QueuedMessage): QueuedMessage {
  return {
    id: message.id,
    orderKey: message.orderKey,
    workerOperation: message.workerOperation,
    workerInput: cloneQueuedWorkerInput(message.workerInput),
    canonicalUserCountBefore: message.canonicalUserCountBefore,
    content: message.content,
    attachments: message.attachments.map(cloneAttachment),
    sendOptions: cloneSendOptions(message.sendOptions),
  };
}

function cloneRecoveryRow(row: ChatMessage): ChatMessage {
  return {
    id: row.id,
    role: row.role,
    content: row.content,
    attachments: row.attachments?.map((attachment) => ({
      type: attachment.type,
      name: attachment.name,
      mimeType: attachment.mimeType,
      uri: attachment.uri,
      base64: attachment.base64,
    })),
    isQueued: true,
  };
}

function cloneRecord(
  record: QueuedSuccessorRecoveryRecord,
): QueuedSuccessorRecoveryRecord {
  return {
    ...record,
    queuedMessages: record.queuedMessages.map(cloneQueuedMessage),
    rows: record.rows.map(cloneRecoveryRow),
    claimedMessageIds: record.claimedMessageIds
      ? [...record.claimedMessageIds]
      : undefined,
    workerInput: record.workerInput ? { ...record.workerInput } : undefined,
  };
}

function isObject(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function parseAttachment(value: unknown): Attachment | null {
  if (!isObject(value)) return null;
  if (
    typeof value.name !== "string" ||
    (value.type !== "image" && value.type !== "file") ||
    typeof value.mimeType !== "string"
  ) {
    return null;
  }
  for (const key of ["uri", "base64", "text"] as const) {
    if (value[key] !== undefined && typeof value[key] !== "string") return null;
  }
  return {
    name: value.name,
    type: value.type,
    mimeType: value.mimeType,
    uri: value.uri as string | undefined,
    base64: value.base64 as string | undefined,
    text: value.text as string | undefined,
  };
}

function parseSendOptions(value: unknown): SendMessageOptions | undefined {
  if (!isObject(value)) return undefined;
  const nullableStrings = ["projectDir", "workingDir", "targetBranch"] as const;
  for (const key of nullableStrings) {
    if (
      value[key] !== undefined && value[key] !== null &&
      typeof value[key] !== "string"
    ) return undefined;
  }
  if (
    value.workspaceMode !== undefined && value.workspaceMode !== "neutral" &&
    value.workspaceMode !== "selected"
  ) return undefined;
  if (
    value.sessionType !== undefined && value.sessionType !== "chat" &&
    value.sessionType !== "code" && value.sessionType !== "hive"
  ) return undefined;
  if (
    value.hiveConversationKind !== undefined &&
    value.hiveConversationKind !== "worker_dm" &&
    value.hiveConversationKind !== "primary_hive"
  ) return undefined;
  return {
    projectDir: value.projectDir as string | null | undefined,
    workingDir: value.workingDir as string | null | undefined,
    workspaceMode: value.workspaceMode as SendMessageOptions["workspaceMode"],
    sessionType: value.sessionType as SendMessageOptions["sessionType"],
    targetBranch: value.targetBranch as string | null | undefined,
    hiveConversationKind: value
      .hiveConversationKind as SendMessageOptions["hiveConversationKind"],
  };
}

function parseQueuedMessage(value: unknown): QueuedMessage | null {
  if (!isObject(value)) return null;
  if (
    typeof value.id !== "string" || value.id.length === 0 ||
    (value.orderKey !== undefined &&
      (typeof value.orderKey !== "string" || value.orderKey.length > 128)) ||
    (value.workerOperation !== undefined &&
      value.workerOperation !== "chat" && value.workerOperation !== "steer") ||
    (value.canonicalUserCountBefore !== undefined &&
      (typeof value.canonicalUserCountBefore !== "number" ||
        !Number.isSafeInteger(value.canonicalUserCountBefore) ||
        value.canonicalUserCountBefore < 0)) ||
    typeof value.content !== "string" || !Array.isArray(value.attachments)
  ) return null;
  const attachments = value.attachments.map(parseAttachment);
  if (attachments.some((attachment) => attachment === null)) return null;
  const workerInput: QueuedMessage["workerInput"] =
    isObject(value.workerInput) &&
      (value.workerInput.operation === "chat" ||
        value.workerInput.operation === "steer") &&
      typeof value.workerInput.fingerprint === "string" &&
      value.workerInput.fingerprint.length > 0 &&
      typeof value.workerInput.key === "string" &&
      value.workerInput.key.length > 0
      ? {
        operation: value.workerInput.operation,
        fingerprint: value.workerInput.fingerprint,
        key: value.workerInput.key,
      }
      : undefined;
  if (value.workerInput !== undefined && !workerInput) return null;
  if (
    workerInput && value.workerOperation !== undefined &&
    workerInput.operation !== value.workerOperation
  ) return null;
  return {
    id: value.id,
    orderKey: value.orderKey as string | undefined,
    workerOperation: value.workerOperation as "chat" | "steer" | undefined,
    workerInput,
    canonicalUserCountBefore: value.canonicalUserCountBefore as
      | number
      | undefined,
    content: value.content,
    attachments: attachments as Attachment[],
    sendOptions: parseSendOptions(value.sendOptions),
  };
}

function parseRecoveryRow(value: unknown): ChatMessage | null {
  if (!isObject(value)) return null;
  if (
    typeof value.id !== "string" || value.id.length === 0 ||
    value.role !== "user" || typeof value.content !== "string"
  ) return null;
  const attachments = Array.isArray(value.attachments)
    ? value.attachments.map((attachment) => {
      if (!isObject(attachment)) return null;
      if (attachment.type !== "image" && attachment.type !== "file") {
        return null;
      }
      return {
        type: attachment.type,
        name: typeof attachment.name === "string" ? attachment.name : undefined,
        mimeType: typeof attachment.mimeType === "string"
          ? attachment.mimeType
          : undefined,
        uri: typeof attachment.uri === "string" ? attachment.uri : undefined,
        base64: typeof attachment.base64 === "string"
          ? attachment.base64
          : undefined,
      };
    })
    : undefined;
  if (attachments?.some((attachment) => attachment === null)) return null;
  return {
    id: value.id,
    role: "user",
    content: value.content,
    attachments: attachments as ChatMessage["attachments"],
    isQueued: true,
  };
}

function parseRecord(value: unknown): QueuedSuccessorRecoveryRecord | null {
  if (!isObject(value)) return null;
  if (
    value.version !== STORAGE_VERSION || typeof value.id !== "string" ||
    typeof value.sessionId !== "string" || value.sessionId.length === 0 ||
    !Array.isArray(value.queuedMessages) || !Array.isArray(value.rows) ||
    typeof value.updatedAt !== "number" || !Number.isFinite(value.updatedAt) ||
    ![
      "pending",
      "claiming",
      "in_flight",
      "uncertain",
      "rejected",
      "accepted",
    ].includes(
      String(value.phase),
    )
  ) return null;
  const queuedMessages = value.queuedMessages.map(parseQueuedMessage);
  const rows = value.rows.map(parseRecoveryRow);
  if (
    queuedMessages.some((message) => message === null) ||
    rows.some((row) => row === null)
  ) return null;
  const queuedIds = new Set(
    (queuedMessages as QueuedMessage[]).map((message) => message.id),
  );
  const phase = value.phase as QueuedSuccessorPhase;
  const isAcceptedTombstone = phase === "accepted";
  if (
    queuedIds.size !== queuedMessages.length ||
    (!isAcceptedTombstone && queuedMessages.length === 0) ||
    (isAcceptedTombstone &&
      (queuedMessages.length !== 0 || rows.length !== 0)) ||
    queuedMessages.length > MAX_QUEUED_MESSAGES ||
    (rows as ChatMessage[]).some((row) => !queuedIds.has(row.id))
  ) return null;
  const workerInput: QueuedWorkerInputIdentity | undefined =
    isObject(value.workerInput) &&
      typeof value.workerInput.fingerprint === "string" &&
      typeof value.workerInput.key === "string" &&
      (value.workerInput.operation === undefined ||
        value.workerInput.operation === "chat" ||
        value.workerInput.operation === "steer")
      ? {
        operation: value.workerInput.operation === "steer" ? "steer" : "chat",
        fingerprint: value.workerInput.fingerprint,
        key: value.workerInput.key,
      }
      : undefined;
  const claimedMessageIds = isAcceptedTombstone
    ? undefined
    : Array.isArray(value.claimedMessageIds) &&
        value.claimedMessageIds.every((id) => typeof id === "string")
    ? value.claimedMessageIds as string[]
    : value.phase === "pending"
    ? undefined
    : (queuedMessages as QueuedMessage[]).map((message) => message.id);
  if (
    claimedMessageIds &&
    (claimedMessageIds.length === 0 ||
      new Set(claimedMessageIds).size !== claimedMessageIds.length ||
      claimedMessageIds.some((id) => !queuedIds.has(id)))
  ) return null;
  const record: QueuedSuccessorRecoveryRecord = {
    version: 1,
    id: value.id,
    sessionId: value.sessionId,
    queuedMessages: queuedMessages as QueuedMessage[],
    rows: rows as ChatMessage[],
    phase,
    claimedMessageIds,
    workerInput: isAcceptedTombstone ? undefined : workerInput,
    updatedAt: value.updatedAt,
  };
  if (utf8Bytes(JSON.stringify(record)) > MAX_RECORD_BYTES) return null;
  return record;
}

function parseEnvelope(raw: string | null): QueuedSuccessorRecoveryRecord[] {
  if (!raw || utf8Bytes(raw) > MAX_ENVELOPE_BYTES) return [];
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!isObject(parsed) || parsed.version !== STORAGE_VERSION) return [];
    if (!Array.isArray(parsed.records)) return [];
    return parsed.records
      .map(parseRecord)
      .filter((record): record is QueuedSuccessorRecoveryRecord =>
        record !== null && record.phase !== "accepted"
      )
      .sort((left, right) => left.updatedAt - right.updatedAt)
      .slice(-MAX_RECOVERY_SESSIONS);
  } catch {
    return [];
  }
}

export function queuedRecoverySerializedBytes(
  queuedMessages: QueuedMessage[],
): number {
  return utf8Bytes(JSON.stringify(queuedMessages.map(cloneQueuedMessage)));
}

export function canPersistQueuedRecovery(
  queuedMessages: QueuedMessage[],
): boolean {
  // Reserve room for optimistic rows, exact Worker identity, and envelope
  // metadata rather than accepting a payload that can only fail at rollover.
  return queuedRecoverySerializedBytes(queuedMessages) <=
    MAX_RECORD_BYTES - 64 * 1024;
}

interface RecoveryCoordinator {
  tail: Promise<void>;
  activeAttempts: Map<
    string,
    { owner: { isDisposed(): boolean }; token: string }
  >;
  deletionAdmissions: Set<string>;
  deletionSnapshots: Map<
    string,
    QueuedSuccessorRecoveryRecord | null
  >;
  failedDeletionRollbacks: Set<string>;
}

const recoveryCoordinatorsByNamespace = new Map<
  string,
  Map<string, RecoveryCoordinator>
>();
const recoveryCoordinatorsByStorage = new WeakMap<
  MitsuroStorage,
  Map<string, RecoveryCoordinator>
>();

function createRecoveryCoordinator(): RecoveryCoordinator {
  return {
    tail: Promise.resolve(),
    activeAttempts: new Map(),
    deletionAdmissions: new Set(),
    deletionSnapshots: new Map(),
    failedDeletionRollbacks: new Set(),
  };
}

function coordinatorFor(
  storage: MitsuroStorage,
  storageKey: string,
): RecoveryCoordinator {
  const durableRecoveryNamespace = storage.durableRecoveryNamespace;
  let coordinators: Map<string, RecoveryCoordinator> | undefined;
  if (durableRecoveryNamespace) {
    coordinators = recoveryCoordinatorsByNamespace.get(
      durableRecoveryNamespace,
    );
    if (!coordinators) {
      coordinators = new Map();
      recoveryCoordinatorsByNamespace.set(
        durableRecoveryNamespace,
        coordinators,
      );
    }
  } else {
    coordinators = recoveryCoordinatorsByStorage.get(storage);
    if (!coordinators) {
      coordinators = new Map();
      recoveryCoordinatorsByStorage.set(storage, coordinators);
    }
  }
  let coordinator = coordinators.get(storageKey);
  if (!coordinator) {
    coordinator = createRecoveryCoordinator();
    coordinators.set(storageKey, coordinator);
  }
  return coordinator;
}

function createRecoveryId(prefix: string): string {
  const randomUuid = globalThis.crypto?.randomUUID?.();
  if (randomUuid) return `${prefix}-${randomUuid}`;
  return `${prefix}-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

function attemptKey(sessionId: string, claimId: string): string {
  return `${sessionId}\u0000${claimId}`;
}

function claimedIds(record: QueuedSuccessorRecoveryRecord): Set<string> {
  return new Set(
    record.claimedMessageIds ??
      (record.phase === "pending"
        ? []
        : record.queuedMessages.map((message) => message.id)),
  );
}

function claimView(
  record: QueuedSuccessorRecoveryRecord,
  attemptToken: string,
): QueuedSuccessorRecoveryClaim {
  const activeIds = claimedIds(record);
  return {
    ...cloneRecord(record),
    queuedMessages: record.queuedMessages
      .filter((message) => activeIds.has(message.id))
      .map(cloneQueuedMessage),
    rows: record.rows
      .filter((row) => activeIds.has(row.id))
      .map(cloneRecoveryRow),
    attemptToken,
  };
}

function recoverInterruptedRecord(
  record: QueuedSuccessorRecoveryRecord,
  coordinator: RecoveryCoordinator,
): QueuedSuccessorRecoveryRecord {
  const key = attemptKey(record.sessionId, record.id);
  const activeAttempt = coordinator.activeAttempts.get(key);
  if (activeAttempt && !activeAttempt.owner.isDisposed()) return record;
  if (activeAttempt) coordinator.activeAttempts.delete(key);
  if (record.phase === "claiming") {
    return {
      ...record,
      phase: "pending",
      claimedMessageIds: undefined,
      workerInput: undefined,
    };
  }
  if (record.phase === "in_flight") {
    return { ...record, phase: "uncertain" };
  }
  return record;
}

export class QueuedSuccessorRecovery {
  private readonly records = new Map<string, QueuedSuccessorRecoveryRecord>();
  private readonly storageKey: string;
  private readonly hydration: Promise<void>;
  private readonly coordinator: RecoveryCoordinator;
  private readonly locallyAcceptedMessages = new Set<string>();
  private hydrated = false;
  private disposed = false;

  constructor(
    private readonly storage: MitsuroStorage,
    scope: string,
  ) {
    this.storageKey = `mitsuro-queued-successor-recovery-v1:${scope}`;
    this.coordinator = coordinatorFor(storage, this.storageKey);
    const synchronousRaw = storage.getDurableSync
      ? storage.getDurableSync(this.storageKey)
      : !storage.getDurable
      ? storage.get(this.storageKey)
      : undefined;
    if (synchronousRaw !== undefined) {
      for (const record of parseEnvelope(synchronousRaw)) {
        this.records.set(
          record.sessionId,
          recoverInterruptedRecord(record, this.coordinator),
        );
      }
      this.hydrated = true;
    }
    this.hydration = this.runExclusive(async () => {
      await this.reload(true);
      this.hydrated = true;
    }).catch(() => {
      // Corrupt or unavailable local recovery storage must not block chat.
    });
  }

  ready(): Promise<void> {
    return this.hydration.then(async () => {
      if (this.hydrated) return;
      await this.runExclusive(async () => {
        await this.reload(true);
        this.hydrated = true;
      });
    });
  }

  isReady(): boolean {
    return this.hydrated;
  }

  dispose(): void {
    this.disposed = true;
  }

  isDisposed(): boolean {
    return this.disposed;
  }

  get(sessionId: string): QueuedSuccessorRecoveryRecord | null {
    const record = this.recordFor(sessionId);
    return record ? cloneRecord(record) : null;
  }

  pendingIds(sessionId: string): Set<string> {
    const record = this.recordFor(sessionId);
    if (!record) return new Set();
    if (record.phase === "pending") {
      return new Set(record.queuedMessages.map((message) => message.id));
    }
    const activeIds = claimedIds(record);
    return new Set(
      record.queuedMessages
        .filter((message) => !activeIds.has(message.id))
        .map((message) => message.id),
    );
  }

  /**
   * Messages that may be sent next without reordering an uncertain ordinary
   * Chat request. Pending input was never transported; rejected/keyed
   * uncertain input can reuse its exact Worker idempotency identity.
   */
  claimable(sessionId: string): QueuedMessage[] {
    const record = this.recordFor(sessionId);
    if (
      !record || record.phase === "claiming" ||
      record.phase === "in_flight" || record.phase === "accepted"
    ) {
      return [];
    }
    if (record.phase === "uncertain" && !record.workerInput) return [];
    if (record.phase === "pending") {
      const first = record.queuedMessages[0];
      if (!first) return [];
      if (first.workerInput) return [cloneQueuedMessage(first)];
      const ordinaryPrefix: QueuedMessage[] = [];
      for (const message of record.queuedMessages) {
        if (message.workerInput) break;
        ordinaryPrefix.push(cloneQueuedMessage(message));
      }
      return ordinaryPrefix;
    }
    const activeIds = claimedIds(record);
    return record.queuedMessages
      .filter((message) => activeIds.has(message.id))
      .map(cloneQueuedMessage);
  }

  /** Tail that is not already owned by the current/last transport attempt. */
  tail(sessionId: string): QueuedMessage[] {
    const record = this.recordFor(sessionId);
    if (!record) return [];
    if (record.phase === "pending") {
      return record.queuedMessages.map(cloneQueuedMessage);
    }
    const activeIds = claimedIds(record);
    return record.queuedMessages
      .filter((message) => !activeIds.has(message.id))
      .map(cloneQueuedMessage);
  }

  isOrdinaryUncertain(sessionId: string): boolean {
    const record = this.recordFor(sessionId);
    return record?.phase === "uncertain" && !record.workerInput;
  }

  isDelivering(sessionId: string): boolean {
    const phase = this.recordFor(sessionId)?.phase;
    return phase === "claiming" || phase === "in_flight";
  }

  async appendPending(
    sessionId: string,
    message: QueuedMessage,
    row: ChatMessage,
  ): Promise<QueuedSuccessorRecoveryRecord> {
    if (this.coordinator.deletionAdmissions.has(sessionId)) {
      throw new Error(
        "This conversation is being deleted; input was not sent.",
      );
    }
    return await this.mutate(async () => {
      if (this.coordinator.deletionAdmissions.has(sessionId)) {
        throw new Error(
          "This conversation is being deleted; input was not sent.",
        );
      }
      const stored = this.records.get(sessionId);
      const existing = stored?.phase === "accepted" ? undefined : stored;
      if (!existing && !stored && this.records.size >= MAX_RECOVERY_SESSIONS) {
        throw new Error(
          "Queued recovery is full. Reopen or discard an earlier queued conversation first.",
        );
      }
      const duplicate = existing?.queuedMessages.find((queued) =>
        queued.id === message.id
      );
      if (duplicate) {
        if (
          JSON.stringify(cloneQueuedMessage(duplicate)) !==
            JSON.stringify(cloneQueuedMessage(message))
        ) {
          throw new Error(
            "A queued input identity was reused for different content.",
          );
        }
        return cloneRecord(existing!);
      }
      const next: QueuedSuccessorRecoveryRecord = existing
        ? {
          ...existing,
          queuedMessages: this.orderQueuedMessages([
            ...existing.queuedMessages,
            cloneQueuedMessage(message),
          ]),
          rows: existing.rows.some((existingRow) => existingRow.id === row.id)
            ? existing.rows
            : [...existing.rows, cloneRecoveryRow(row)],
          updatedAt: Date.now(),
        }
        : {
          version: 1,
          id: createRecoveryId("queued-pending"),
          sessionId,
          queuedMessages: [cloneQueuedMessage(message)],
          rows: [cloneRecoveryRow(row)],
          phase: "pending",
          updatedAt: Date.now(),
        };
      const uniqueIds = new Set(
        next.queuedMessages.map((queued) => queued.id),
      );
      const rowOrder = new Map(
        next.queuedMessages.map((queued, index) => [queued.id, index]),
      );
      next.rows.sort((left, right) =>
        (rowOrder.get(left.id) ?? Number.MAX_SAFE_INTEGER) -
        (rowOrder.get(right.id) ?? Number.MAX_SAFE_INTEGER)
      );
      if (
        uniqueIds.size !== next.queuedMessages.length ||
        next.queuedMessages.length > MAX_QUEUED_MESSAGES
      ) {
        throw new Error(
          "Message queue is full. Please wait for the current response to finish.",
        );
      }
      this.assertRecordSize(next);
      this.records.delete(sessionId);
      this.records.set(sessionId, next);
      return cloneRecord(next);
    });
  }

  async claim(
    input: QueuedSuccessorClaimInput,
    _rows: ChatMessage[],
  ): Promise<QueuedSuccessorRecoveryClaim> {
    if (
      this.coordinator.deletionAdmissions.has(input.sessionId) ||
      (input.sourceSessionId &&
        this.coordinator.deletionAdmissions.has(input.sourceSessionId))
    ) {
      throw new Error(
        "This conversation is being deleted; input was not sent.",
      );
    }
    return await this.mutate(async () => {
      if (
        this.coordinator.deletionAdmissions.has(input.sessionId) ||
        (input.sourceSessionId &&
          this.coordinator.deletionAdmissions.has(input.sourceSessionId))
      ) {
        throw new Error(
          "This conversation is being deleted; input was not sent.",
        );
      }
      const sourceSessionId = input.sourceSessionId ?? input.sessionId;
      if (sourceSessionId !== input.sessionId) {
        const source = this.records.get(sourceSessionId);
        const target = this.records.get(input.sessionId);
        if (!source || source.phase === "accepted") {
          throw new Error(
            "The durable queued input is no longer available at its pinch source.",
          );
        }
        if (target && target.phase !== "accepted") {
          throw new Error(
            "The pinched conversation already owns another durable queue.",
          );
        }
        if (source.phase === "claiming" || source.phase === "in_flight") {
          throw new Error(
            "The queued input cannot move while another delivery owns it.",
          );
        }
        this.records.delete(sourceSessionId);
        this.records.delete(input.sessionId);
        this.records.set(input.sessionId, {
          ...source,
          sessionId: input.sessionId,
          updatedAt: Date.now(),
        });
      }
      const stored = this.records.get(input.sessionId);
      const existing = stored?.phase === "accepted" ? undefined : stored;
      const inputIds = new Set(
        input.queuedMessages.map((message) => message.id),
      );
      if (
        input.queuedMessages.length === 0 ||
        input.queuedMessages.length > MAX_QUEUED_MESSAGES ||
        inputIds.size !== input.queuedMessages.length
      ) {
        throw new Error(
          "Queued recovery payload is invalid or exceeds its bound.",
        );
      }
      if (existing?.phase === "claiming" || existing?.phase === "in_flight") {
        throw new Error("The queued successor is already being delivered.");
      }
      if (existing?.phase === "uncertain" && !existing.workerInput) {
        throw new Error(
          "A prior queued Chat may already have been delivered. Review that conversation before resending it.",
        );
      }
      if (!existing) {
        throw new Error(
          "The durable queued input is no longer available to claim.",
        );
      }
      const pendingPrefixIds: string[] = [];
      if (existing.phase === "pending") {
        const first = existing.queuedMessages[0];
        if (first?.workerInput) {
          pendingPrefixIds.push(first.id);
        } else {
          for (const message of existing.queuedMessages) {
            if (message.workerInput) break;
            pendingPrefixIds.push(message.id);
          }
        }
      }
      const activeIds = existing.phase === "pending"
        ? pendingPrefixIds
        : [...claimedIds(existing)];
      const expectedPrefix = existing.queuedMessages
        .slice(0, activeIds.length)
        .map((message) => message.id);
      if (
        activeIds.some((id) => !inputIds.has(id)) ||
        activeIds.some((id, index) => id !== expectedPrefix[index])
      ) {
        throw new Error(
          "Queued delivery must claim the oldest durable input first.",
        );
      }
      const next: QueuedSuccessorRecoveryRecord = {
        ...existing,
        phase: "claiming",
        claimedMessageIds: activeIds,
        updatedAt: Date.now(),
      };
      this.assertRecordSize(next);
      this.records.delete(input.sessionId);
      this.records.set(input.sessionId, next);
      const attemptToken = createRecoveryId("queued-attempt");
      this.coordinator.activeAttempts.set(
        attemptKey(input.sessionId, next.id),
        { owner: this, token: attemptToken },
      );
      return claimView(next, attemptToken);
    });
  }

  async markInFlight(
    sessionId: string,
    claimId: string,
    attemptToken: string,
    workerInput?: QueuedWorkerInputIdentity,
  ): Promise<boolean> {
    return await this.mutate(async () => {
      const current = this.records.get(sessionId);
      if (
        !this.ownsAttempt(sessionId, claimId, attemptToken) ||
        !current || current.id !== claimId || current.phase !== "claiming"
      ) return false;
      const next: QueuedSuccessorRecoveryRecord = {
        ...current,
        phase: "in_flight",
        workerInput,
        updatedAt: Date.now(),
      };
      this.assertRecordSize(next);
      this.records.set(sessionId, next);
      return true;
    });
  }

  /** Compatibility name for focused tests and callers being migrated. */
  async setWorkerIdentity(
    sessionId: string,
    claimId: string,
    attemptToken: string,
    workerInput: QueuedWorkerInputIdentity,
  ): Promise<boolean> {
    return await this.markInFlight(
      sessionId,
      claimId,
      attemptToken,
      workerInput,
    );
  }

  async reject(
    sessionId: string,
    claimId: string,
    attemptToken: string,
  ): Promise<QueuedSuccessorRecoveryRecord | null> {
    return await this.mutate(async () => {
      const current = this.records.get(sessionId);
      if (
        !this.ownsAttempt(sessionId, claimId, attemptToken) ||
        !current || current.id !== claimId || current.phase === "accepted"
      ) {
        return null;
      }
      const next: QueuedSuccessorRecoveryRecord = {
        ...current,
        // Worker retries retain an exact server idempotency identity. Ordinary
        // Chat has no such proof, so a transport failure remains visible but
        // must not be replayed automatically after an uncertain outcome.
        phase: current.workerInput ? "rejected" : "uncertain",
        updatedAt: Date.now(),
      };
      this.records.set(sessionId, next);
      this.clearAttempt(sessionId, claimId, attemptToken);
      return cloneRecord(next);
    });
  }

  /** The claim was durably prepared but no transport was started. */
  async releaseUndispatched(
    sessionId: string,
    claimId: string,
    attemptToken: string,
  ): Promise<QueuedSuccessorRecoveryRecord | null> {
    return await this.mutate(async () => {
      const current = this.records.get(sessionId);
      if (
        !this.ownsAttempt(sessionId, claimId, attemptToken) ||
        !current || current.id !== claimId ||
        (current.phase !== "claiming" && current.phase !== "in_flight")
      ) {
        return null;
      }
      const next: QueuedSuccessorRecoveryRecord = {
        ...current,
        phase: current.workerInput ? "uncertain" : "pending",
        claimedMessageIds: current.workerInput
          ? current.claimedMessageIds
          : undefined,
        workerInput: current.workerInput,
        updatedAt: Date.now(),
      };
      this.records.set(sessionId, next);
      this.clearAttempt(sessionId, claimId, attemptToken);
      return cloneRecord(next);
    });
  }

  /** A keyed steer was definitely not staged; preserve its slot as Chat. */
  async fallbackToPendingChat(
    sessionId: string,
    claimId: string,
    attemptToken: string,
    chatIdentity?: NonNullable<QueuedMessage["workerInput"]>,
  ): Promise<QueuedSuccessorRecoveryRecord | null> {
    return await this.mutate(async () => {
      const current = this.records.get(sessionId);
      if (
        !this.ownsAttempt(sessionId, claimId, attemptToken) ||
        !current || current.id !== claimId || current.phase === "accepted"
      ) return null;
      const activeIds = claimedIds(current);
      const next: QueuedSuccessorRecoveryRecord = {
        ...current,
        queuedMessages: current.queuedMessages.map((message) =>
          activeIds.has(message.id)
            ? {
              ...message,
              workerOperation: "chat",
              workerInput: chatIdentity
                ? cloneQueuedWorkerInput(chatIdentity)
                : undefined,
            }
            : message
        ),
        phase: "pending",
        claimedMessageIds: undefined,
        workerInput: undefined,
        updatedAt: Date.now(),
      };
      this.records.set(sessionId, next);
      this.clearAttempt(sessionId, claimId, attemptToken);
      return cloneRecord(next);
    });
  }

  /** Explicit user authorization to retry a non-idempotent uncertain batch. */
  async retryOrdinaryUncertain(
    sessionId: string,
  ): Promise<QueuedSuccessorRecoveryRecord | null> {
    return await this.mutate(async () => {
      const current = this.records.get(sessionId);
      if (
        !current || current.phase !== "uncertain" || current.workerInput
      ) return current ? cloneRecord(current) : null;
      const next: QueuedSuccessorRecoveryRecord = {
        ...current,
        phase: "pending",
        claimedMessageIds: undefined,
        workerInput: undefined,
        updatedAt: Date.now(),
      };
      this.records.set(sessionId, next);
      return cloneRecord(next);
    });
  }

  async accept(
    sessionId: string,
    claimId: string,
    attemptToken: string,
  ): Promise<boolean> {
    const accepted = await this.mutate(async () => {
      const current = this.records.get(sessionId);
      if (
        !this.ownsAttempt(sessionId, claimId, attemptToken) ||
        !current || current.id !== claimId
      ) return false;
      const activeIds = claimedIds(current);
      const queuedMessages = current.queuedMessages.filter((message) =>
        !activeIds.has(message.id)
      );
      const rows = current.rows.filter((row) => !activeIds.has(row.id));
      this.records.set(
        sessionId,
        queuedMessages.length > 0
          ? {
            version: 1,
            id: createRecoveryId("queued-pending"),
            sessionId,
            queuedMessages,
            rows,
            phase: "pending",
            updatedAt: Date.now(),
          }
          : {
            version: 1,
            id: claimId,
            sessionId,
            queuedMessages: [],
            rows: [],
            phase: "accepted",
            updatedAt: Date.now(),
          },
      );
      this.clearAttempt(sessionId, claimId, attemptToken);
      return true;
    });
    if (!accepted) return false;
    await this.cleanupTombstoneBestEffort();
    return true;
  }

  /**
   * Remove one exact, not-yet-claimed Worker input after its HTTP staging
   * response. This is used by active steering, which can be durably appended
   * behind the Chat stream that currently owns the record's leading claim.
   */
  async acceptPendingRemote(
    sessionId: string,
    messageId: string,
    workerInput: NonNullable<QueuedMessage["workerInput"]>,
  ): Promise<boolean> {
    try {
      const accepted = await this.mutate(async () => {
        const current = this.records.get(sessionId);
        if (!current || current.phase === "accepted") return false;
        const activeIds = claimedIds(current);
        if (activeIds.has(messageId)) return false;
        const message = current.queuedMessages.find((queued) =>
          queued.id === messageId
        );
        if (
          !message?.workerInput ||
          message.workerInput.operation !== workerInput.operation ||
          message.workerInput.fingerprint !== workerInput.fingerprint ||
          message.workerInput.key !== workerInput.key
        ) return false;
        const queuedMessages = current.queuedMessages.filter((queued) =>
          queued.id !== messageId
        );
        const rows = current.rows.filter((row) => row.id !== messageId);
        this.records.set(
          sessionId,
          queuedMessages.length > 0
            ? { ...current, queuedMessages, rows, updatedAt: Date.now() }
            : {
              version: 1,
              id: current.id,
              sessionId,
              queuedMessages: [],
              rows: [],
              phase: "accepted",
              updatedAt: Date.now(),
            },
        );
        return true;
      });
      if (accepted) await this.cleanupTombstoneBestEffort();
      return accepted;
    } catch (error) {
      const current = this.records.get(sessionId);
      const message = current?.queuedMessages.find((queued) =>
        queued.id === messageId
      );
      if (
        !message?.workerInput ||
        message.workerInput.operation !== workerInput.operation ||
        message.workerInput.fingerprint !== workerInput.fingerprint ||
        message.workerInput.key !== workerInput.key
      ) throw error;
      this.locallyAcceptedMessages.add(attemptKey(sessionId, messageId));
      queueMicrotask(() => {
        void this.cleanupLocallyAcceptedMessages().catch(() => undefined);
      });
      return true;
    }
  }

  /** Convert only one durable steer tail into an exact Chat fallback. */
  async fallbackPendingSteerToChat(
    sessionId: string,
    messageId: string,
    chatIdentity: NonNullable<QueuedMessage["workerInput"]>,
  ): Promise<QueuedSuccessorRecoveryRecord | null> {
    return await this.mutate(async () => {
      const current = this.records.get(sessionId);
      if (!current || current.phase === "accepted") return null;
      const activeIds = claimedIds(current);
      if (activeIds.has(messageId)) return null;
      const index = current.queuedMessages.findIndex((message) =>
        message.id === messageId &&
        (message.workerInput?.operation ?? message.workerOperation) === "steer"
      );
      if (index < 0) return null;
      const queuedMessages = current.queuedMessages.map((message, at) =>
        at === index
          ? {
            ...message,
            workerOperation: "chat" as const,
            workerInput: cloneQueuedWorkerInput(chatIdentity),
          }
          : message
      );
      const next = {
        ...current,
        queuedMessages,
        updatedAt: Date.now(),
      };
      this.assertRecordSize(next);
      this.records.set(sessionId, next);
      return cloneRecord(next);
    });
  }

  /** Definitively rejected pending input; remove only its exact durable row. */
  async discardPending(
    sessionId: string,
    messageId: string,
  ): Promise<boolean> {
    const discarded = await this.mutate(async () => {
      const current = this.records.get(sessionId);
      if (!current || current.phase === "accepted") return false;
      if (claimedIds(current).has(messageId)) return false;
      if (!current.queuedMessages.some((message) => message.id === messageId)) {
        return false;
      }
      const queuedMessages = current.queuedMessages.filter((message) =>
        message.id !== messageId
      );
      const rows = current.rows.filter((row) => row.id !== messageId);
      this.records.set(
        sessionId,
        queuedMessages.length > 0
          ? { ...current, queuedMessages, rows, updatedAt: Date.now() }
          : {
            version: 1,
            id: current.id,
            sessionId,
            queuedMessages: [],
            rows: [],
            phase: "accepted",
            updatedAt: Date.now(),
          },
      );
      return true;
    });
    if (discarded) await this.cleanupTombstoneBestEffort();
    return discarded;
  }

  /**
   * Records an authoritative remote acceptance even when the local tombstone
   * write fails. The exact durable Worker key remains available to a restarted
   * graph, while this live graph relinquishes transport authority and retries
   * the payload-free cleanup without replaying the accepted input.
   */
  async acceptRemote(
    sessionId: string,
    claimId: string,
    attemptToken: string,
  ): Promise<boolean> {
    try {
      return await this.accept(sessionId, claimId, attemptToken);
    } catch (error) {
      if (!this.ownsAttempt(sessionId, claimId, attemptToken)) throw error;
      const current = this.records.get(sessionId);
      if (!current || current.id !== claimId) throw error;
      const acceptedIds = claimedIds(current);
      if (acceptedIds.size === 0) throw error;
      this.coordinator.activeAttempts.delete(attemptKey(sessionId, claimId));
      for (const messageId of acceptedIds) {
        this.locallyAcceptedMessages.add(attemptKey(sessionId, messageId));
      }
      queueMicrotask(() => {
        void this.cleanupLocallyAcceptedMessages().catch(() => undefined);
      });
      return true;
    }
  }

  async delete(
    sessionId: string,
  ): Promise<QueuedSuccessorRecoveryRecord | null> {
    const discarded = await this.mutate(async () => {
      const current = this.records.get(sessionId);
      if (!current) return null;
      this.records.set(sessionId, {
        version: 1,
        id: current.id,
        sessionId,
        queuedMessages: [],
        rows: [],
        phase: "accepted",
        updatedAt: Date.now(),
      });
      for (const key of this.coordinator.activeAttempts.keys()) {
        if (key.startsWith(`${sessionId}\u0000`)) {
          this.coordinator.activeAttempts.delete(key);
        }
      }
      return cloneRecord(current);
    });
    if (!discarded) return null;
    await this.cleanupTombstoneBestEffort();
    return discarded;
  }

  acquireDeletionAdmission(sessionId: string): void {
    if (this.coordinator.deletionAdmissions.has(sessionId)) {
      throw new Error("This conversation is already being deleted.");
    }
    this.coordinator.deletionAdmissions.add(sessionId);
    this.coordinator.deletionSnapshots.delete(sessionId);
    this.coordinator.failedDeletionRollbacks.delete(sessionId);
  }

  isDeletionAdmitted(sessionId: string): boolean {
    return this.coordinator.deletionAdmissions.has(sessionId);
  }

  canRepairFailedDeletionAdmission(sessionId: string): boolean {
    return this.coordinator.deletionAdmissions.has(sessionId) &&
      this.coordinator.deletionSnapshots.has(sessionId) &&
      this.coordinator.failedDeletionRollbacks.has(sessionId);
  }

  /** Atomically wait behind prior mutations and replace the record with a scrub. */
  async scrubForDeletion(
    sessionId: string,
  ): Promise<QueuedSuccessorRecoveryRecord | null> {
    if (!this.coordinator.deletionAdmissions.has(sessionId)) {
      throw new Error("Deletion admission must be held before recovery scrub.");
    }
    const snapshot = await this.mutate(async () => {
      const current = this.records.get(sessionId);
      const snapshot = current && current.phase !== "accepted"
        ? cloneRecord(current)
        : null;
      if (current) {
        this.records.set(sessionId, {
          version: 1,
          id: current.id,
          sessionId,
          queuedMessages: [],
          rows: [],
          phase: "accepted",
          updatedAt: Date.now(),
        });
      }
      for (const key of this.coordinator.activeAttempts.keys()) {
        if (key.startsWith(`${sessionId}\u0000`)) {
          this.coordinator.activeAttempts.delete(key);
        }
      }
      return snapshot;
    });
    this.coordinator.deletionSnapshots.set(
      sessionId,
      snapshot ? cloneRecord(snapshot) : null,
    );
    this.coordinator.failedDeletionRollbacks.delete(sessionId);
    return snapshot;
  }

  async rollbackDeletionAdmission(
    sessionId: string,
    snapshot: QueuedSuccessorRecoveryRecord | null,
    releaseAdmission = true,
  ): Promise<void> {
    if (!this.coordinator.deletionAdmissions.has(sessionId)) return;
    this.coordinator.failedDeletionRollbacks.delete(sessionId);
    try {
      await this.mutate(async () => {
        if (!snapshot) {
          this.records.delete(sessionId);
          return;
        }
        const restored = cloneRecord(snapshot);
        if (restored.phase === "claiming") {
          restored.phase = "pending";
          restored.claimedMessageIds = undefined;
          restored.workerInput = undefined;
        } else if (restored.phase === "in_flight") {
          restored.phase = "uncertain";
        }
        restored.updatedAt = Date.now();
        this.assertRecordSize(restored);
        this.records.set(sessionId, restored);
      });
    } catch (error) {
      this.coordinator.failedDeletionRollbacks.add(sessionId);
      throw error;
    }
    // A failed restore stays admitted. This is deliberately fail-closed: a
    // caller may retry rollback, but no new input can race a missing draft.
    if (releaseAdmission) {
      this.coordinator.deletionAdmissions.delete(sessionId);
      this.coordinator.deletionSnapshots.delete(sessionId);
      this.coordinator.failedDeletionRollbacks.delete(sessionId);
    }
  }

  /** Repair a failed rollback and renew its admission for one fresh DELETE. */
  async renewFailedDeletionAdmission(
    sessionId: string,
  ): Promise<QueuedSuccessorRecoveryRecord | null> {
    if (!this.canRepairFailedDeletionAdmission(sessionId)) {
      throw new Error("This conversation is already being deleted.");
    }
    const snapshot = this.coordinator.deletionSnapshots.get(sessionId) ?? null;
    this.coordinator.failedDeletionRollbacks.delete(sessionId);
    try {
      await this.rollbackDeletionAdmission(sessionId, snapshot, false);
      return await this.scrubForDeletion(sessionId);
    } catch (error) {
      this.coordinator.failedDeletionRollbacks.add(sessionId);
      throw error;
    }
  }

  commitDeletionAdmission(sessionId: string): void {
    this.coordinator.deletionAdmissions.delete(sessionId);
    this.coordinator.deletionSnapshots.delete(sessionId);
    this.coordinator.failedDeletionRollbacks.delete(sessionId);
    queueMicrotask(() => {
      void this.cleanupTombstoneBestEffort();
    });
  }

  releaseDeletionAdmission(sessionId: string): void {
    this.coordinator.deletionAdmissions.delete(sessionId);
    this.coordinator.deletionSnapshots.delete(sessionId);
    this.coordinator.failedDeletionRollbacks.delete(sessionId);
  }

  private async mutate<T>(mutation: () => Promise<T>): Promise<T> {
    return await this.runExclusive(async () => {
      // Reserve the process-wide coordinator position before waiting on this
      // instance's hydration. Otherwise an operation already begun by an old
      // graph can land after its replacement finished hydrating.
      await this.hydration;
      // Stores are rebuilt on reconnect. Refresh under the process-wide
      // storage-key lock so a continuation from an older graph cannot
      // overwrite records written by the current graph.
      // Normalize stale claiming/in-flight phases on every read-modify-write,
      // not only during construction. A freshly rebuilt store must be able to
      // claim the recovered record after its first mutation reloads the disk
      // envelope; live attempts remain unchanged through activeAttempts.
      await this.reload(true);
      const snapshot = new Map(
        [...this.records].map(([sessionId, record]) => [
          sessionId,
          cloneRecord(record),
        ]),
      );
      const activeAttemptSnapshot = new Map(
        this.coordinator.activeAttempts,
      );
      try {
        const value = await mutation();
        await this.persist();
        return value;
      } catch (error) {
        this.records.clear();
        for (const [sessionId, record] of snapshot) {
          this.records.set(sessionId, record);
        }
        this.coordinator.activeAttempts.clear();
        for (const [key, owner] of activeAttemptSnapshot) {
          this.coordinator.activeAttempts.set(key, owner);
        }
        throw error;
      }
    });
  }

  private recordFor(
    sessionId: string,
  ): QueuedSuccessorRecoveryRecord | undefined {
    let current = this.records.get(sessionId);
    if (!current) return undefined;
    current = this.withoutLocallyAcceptedMessages(current);
    if (current.queuedMessages.length === 0) return undefined;
    const recovered = recoverInterruptedRecord(current, this.coordinator);
    if (recovered !== current) this.records.set(sessionId, recovered);
    return recovered;
  }

  private orderQueuedMessages(messages: QueuedMessage[]): QueuedMessage[] {
    return messages.sort((left, right) => {
      if (left.orderKey && right.orderKey) {
        return left.orderKey.localeCompare(right.orderKey);
      }
      if (left.orderKey) return 1;
      if (right.orderKey) return -1;
      return 0;
    });
  }

  private async cleanupTombstoneBestEffort(): Promise<void> {
    try {
      // Accepted/discarded tombstones are filtered during reload. Persisting
      // that empty view physically removes the key, while any concurrently
      // appended newer record is retained.
      await this.mutate(async () => undefined);
    } catch {
      // The first phase already removed all prompt, attachment, and identity
      // material. Physical key deletion is hygiene, not delivery failure.
    }
  }

  private async cleanupLocallyAcceptedMessages(): Promise<void> {
    await this.mutate(async () => undefined);
    for (const key of [...this.locallyAcceptedMessages]) {
      const separator = key.indexOf("\u0000");
      const sessionId = key.slice(0, separator);
      const messageId = key.slice(separator + 1);
      if (
        !this.records.get(sessionId)?.queuedMessages.some((message) =>
          message.id === messageId
        )
      ) {
        this.locallyAcceptedMessages.delete(key);
      }
    }
  }

  private withoutLocallyAcceptedMessages(
    record: QueuedSuccessorRecoveryRecord,
  ): QueuedSuccessorRecoveryRecord {
    const acceptedIds = new Set(
      record.queuedMessages
        .filter((message) =>
          this.locallyAcceptedMessages.has(
            attemptKey(record.sessionId, message.id),
          )
        )
        .map((message) => message.id),
    );
    if (acceptedIds.size === 0) return record;
    const activeIds = claimedIds(record);
    const survivingActiveIds = [...activeIds].filter((id) =>
      !acceptedIds.has(id)
    );
    const queuedMessages = record.queuedMessages.filter((message) =>
      !acceptedIds.has(message.id)
    );
    const visible: QueuedSuccessorRecoveryRecord = {
      ...record,
      queuedMessages,
      rows: record.rows.filter((row) => !acceptedIds.has(row.id)),
      claimedMessageIds: record.claimedMessageIds
        ? survivingActiveIds
        : undefined,
    };
    if (
      [...activeIds].some((id) => acceptedIds.has(id)) &&
      survivingActiveIds.length === 0 && queuedMessages.length > 0
    ) {
      visible.phase = "pending";
      visible.claimedMessageIds = undefined;
      visible.workerInput = undefined;
    }
    return visible;
  }

  private ownsAttempt(
    sessionId: string,
    claimId: string,
    attemptToken: string,
  ): boolean {
    const active = this.coordinator.activeAttempts.get(
      attemptKey(sessionId, claimId),
    );
    return !this.disposed && active?.owner === this &&
      active.token === attemptToken;
  }

  private clearAttempt(
    sessionId: string,
    claimId: string,
    attemptToken: string,
  ): void {
    if (this.ownsAttempt(sessionId, claimId, attemptToken)) {
      this.coordinator.activeAttempts.delete(attemptKey(sessionId, claimId));
    }
  }

  private async runExclusive<T>(operation: () => Promise<T>): Promise<T> {
    const run = this.coordinator.tail.then(operation, operation);
    this.coordinator.tail = run.then(() => undefined, () => undefined);
    return await run;
  }

  private async reload(recoverInterrupted: boolean): Promise<void> {
    const parsed = parseEnvelope(await this.read());
    this.records.clear();
    for (const record of parsed) {
      const visible = this.withoutLocallyAcceptedMessages(record);
      if (visible.queuedMessages.length === 0) continue;
      this.records.set(
        record.sessionId,
        recoverInterrupted
          ? recoverInterruptedRecord(visible, this.coordinator)
          : visible,
      );
    }
  }

  private assertRecordSize(record: QueuedSuccessorRecoveryRecord): void {
    if (utf8Bytes(JSON.stringify(record)) > MAX_RECORD_BYTES) {
      throw new Error(
        "Queued input is too large for crash-safe recovery. Remove an attachment and try again.",
      );
    }
  }

  private async persist(): Promise<void> {
    const envelope: RecoveryEnvelope = {
      version: 1,
      records: [...this.records.values()].map(cloneRecord),
    };
    const raw = JSON.stringify(envelope);
    if (utf8Bytes(raw) > MAX_ENVELOPE_BYTES) {
      throw new Error("Queued recovery storage is full.");
    }
    if (envelope.records.length === 0) {
      if (this.storage.deleteDurable) {
        await this.storage.deleteDurable(this.storageKey);
      } else {
        this.storage.delete(this.storageKey);
      }
      return;
    }
    if (this.storage.setDurable) {
      await this.storage.setDurable(this.storageKey, raw);
    } else {
      this.storage.set(this.storageKey, raw);
    }
  }

  private async read(): Promise<string | null> {
    if (this.storage.getDurable) {
      return await this.storage.getDurable(this.storageKey);
    }
    return this.storage.get(this.storageKey);
  }
}
