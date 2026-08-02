import { MitsuroClient } from './client';

/**
 * Compatibility boundary for clients and servers that predate the Mitsuro/Hive
 * identity migration. New code must use the canonical exports from `client.ts`
 * and `types.ts` directly.
 */
/** @deprecated Import `MitsuroApiError` from `@mitsuro/api`. */
export { MitsuroApiError as KrustyApiError } from './client';
/** @deprecated Import the canonical Mitsuro client types from `@mitsuro/api`. */
export type {
	MitsuroClientConfig as KrustyClientConfig,
	MitsuroRequestDiagnostic as KrustyRequestDiagnostic,
	MitsuroRequestDiagnosticOutcome as KrustyRequestDiagnosticOutcome,
} from './client';

/** @deprecated Use the corresponding `Hive*` type. */
export type {
	HiveAttentionItem as MakoAttentionItem,
	HiveAttentionItemKind as MakoAttentionItemKind,
	HiveAttentionResponse as MakoAttentionResponse,
	HiveAttentionSection as MakoAttentionSection,
	HiveBootstrapResponse as MakoBootstrapResponse,
	HiveCadenceSummary as MakoCadenceSummary,
	HiveChannelItem as MakoChannelItem,
	HiveChannelKind as MakoChannelKind,
	HiveChannelStatus as MakoChannelStatus,
	HiveChannelsResponse as MakoChannelsResponse,
	HiveCrewDocumentKind as MakoCrewDocumentKind,
	HiveCrewMember as MakoCrewMember,
	HiveCrewResponse as MakoCrewResponse,
	HiveCrewRuntimeMember as MakoCrewRuntimeMember,
	HiveCrewRuntimeStatus as MakoCrewRuntimeStatus,
	HiveCurrentResponse as MakoCurrentResponse,
	HiveCurrentRunSummary as MakoCurrentRunSummary,
	HiveDaemonSummary as MakoDaemonSummary,
	HiveDiagnosticSeverity as MakoDiagnosticSeverity,
	HiveDiagnosticsSummary as MakoDiagnosticsSummary,
	HiveDispatchOptions as MakoDispatchOptions,
	HiveDispatchResponse as MakoDispatchResponse,
	HiveDstPolicy as MakoDstPolicy,
	HiveGlobalSchedule as MakoGlobalSchedule,
	HiveHealthState as MakoHealthState,
	HiveHomeDocument as MakoHomeDocument,
	HiveHomeDocumentKind as MakoHomeDocumentKind,
	HiveHomeResponse as MakoHomeResponse,
	HiveHomeStatus as MakoHomeStatus,
	HiveKnowledgeHealthSummary as MakoKnowledgeHealthSummary,
	HiveMainResponse as MakoMainResponse,
	HiveMisfireConfig as MakoMisfireConfig,
	HiveMonthlyDayPolicy as MakoMonthlyDayPolicy,
	HivePendingApproval as MakoPendingApproval,
	HiveQueuePressure as MakoQueuePressure,
	HiveRecoverDaemonResponse as MakoRecoverDaemonResponse,
	HiveRecurrenceV1 as MakoRecurrenceV1,
	HiveRetryPolicy as MakoRetryPolicy,
	HiveRunDiagnostic as MakoRunDiagnostic,
	HiveRunDiagnosticKind as MakoRunDiagnosticKind,
	HiveRunPriority as MakoRunPriority,
	HiveRunWakeEvent as MakoRunWakeEvent,
	HiveRuntimeState as MakoRuntimeState,
	HiveRuntimeStatus as MakoRuntimeStatus,
	HiveSchedule as MakoSchedule,
	HiveScheduleMutationResponse as MakoScheduleMutationResponse,
	HiveScheduleOverlapPolicy as MakoScheduleOverlapPolicy,
	HiveScheduleStatus as MakoScheduleStatus,
	HiveScheduleWeekday as MakoScheduleWeekday,
	HiveScheduleWriteRequest as MakoScheduleWriteRequest,
	HiveSessionStatus as MakoSessionStatus,
	HiveSessionSummary as MakoSessionSummary,
	HiveStatusSummary as MakoStatusSummary,
} from './types';

const legacyHiveClientMethods = {
	dispatchMako: 'dispatchHive',
	getMakoMain: 'getHiveMain',
	ensureMakoMain: 'ensureHiveMain',
	listMakoSchedules: 'listHiveSchedules',
	listMakoSessionSchedules: 'listHiveSessionSchedules',
	createMakoSchedule: 'createHiveSchedule',
	pauseMakoSchedule: 'pauseHiveSchedule',
	resumeMakoSchedule: 'resumeHiveSchedule',
	getMakoCurrent: 'getHiveCurrent',
	getMakoAttention: 'getHiveAttention',
	setMakoAttentionRead: 'setHiveAttentionRead',
	setMakoAttentionCleared: 'setHiveAttentionCleared',
	getMakoHome: 'getHiveHome',
	bootstrapMakoHome: 'bootstrapHiveHome',
	updateMakoHomeDocument: 'updateHiveHomeDocument',
	updateMakoCrewDocument: 'updateHiveCrewDocument',
	getMakoCrew: 'getHiveCrew',
	getMakoChannels: 'getHiveChannels',
	recoverMakoDaemon: 'recoverHiveDaemon',
	listMakoSessions: 'listHiveSessions',
	getMakoSessionStatus: 'getHiveSessionStatus',
	sendMakoMessage: 'sendHiveMessage',
	scheduleMakoSession: 'scheduleHiveSession',
	setMakoSessionPriority: 'setHiveSessionPriority',
	setMakoSessionCrew: 'setHiveSessionCrew',
	pauseMakoSession: 'pauseHiveSession',
	resumeMakoSession: 'resumeHiveSession',
	cancelMakoSession: 'cancelHiveSession',
	observeMakoSession: 'observeHiveSession',
} as const;

type LegacyHiveClientMethods = {
	[LegacyMethod in keyof typeof legacyHiveClientMethods]:
		MitsuroClient[(typeof legacyHiveClientMethods)[LegacyMethod]];
};

/** @deprecated Construct `MitsuroClient`; this bridge is transition-only. */
export interface KrustyClient extends LegacyHiveClientMethods {}
/** @deprecated Construct `MitsuroClient`; this bridge is transition-only. */
export class KrustyClient extends MitsuroClient {}

for (const [legacyMethod, canonicalMethod] of Object.entries(
	legacyHiveClientMethods,
) as Array<[
	keyof typeof legacyHiveClientMethods,
	(typeof legacyHiveClientMethods)[keyof typeof legacyHiveClientMethods],
]>) {
	Object.defineProperty(KrustyClient.prototype, legacyMethod, {
		configurable: true,
		value(this: MitsuroClient, ...args: unknown[]) {
			const method = this[canonicalMethod] as unknown as (
				...values: unknown[]
			) => unknown;
			return method.apply(this, args);
		},
		writable: true,
	});
}
