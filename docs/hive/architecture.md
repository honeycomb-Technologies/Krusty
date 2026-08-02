# Hive Backend Architecture

## Status

This document is the backend contract for Hive. It supersedes descriptions in
older product documents that place autonomous execution inside the HTTP server.
The frontend may present Hive in many ways, but it must not become the owner of
execution, schedules, identity, or recovery state.

## Outcome

Hive is a long-lived, independently supervised process. It keeps working when a
web client disconnects and can restart independently of the Mitsuro HTTP server.
The HTTP server is a control-plane client, not a second Hive runtime.

```text
CLI / mobile / web
        |
        v
mitsuro-server (authentication, HTTP, SSE)
        |
        | authenticated, versioned Unix-socket protocol
        v
mitsuro-hive (ownership checks, scheduler, recovery, event log)
        |
        v
execution host (currently daemon-owned; child-worker boundary is evolvable)
        |
        v
mitsuro-core (orchestrator, tools, storage, providers)
```

The daemon is the only production process allowed to claim scheduled work or
start a Hive execution. Embedded runtime construction remains available for
focused tests, but production router construction fails closed when the daemon
is unavailable.

## Process Boundary

The local control channel is a private Unix-domain socket. The protocol has:

- length-bounded JSON frames;
- explicit major/minor version negotiation;
- request IDs, deadlines, and idempotency keys;
- HMAC authentication with a private on-disk key;
- nonce-based replay protection during connection setup;
- peer-credential verification where the platform exposes it;
- a bounded connection count and I/O timeout;
- graceful shutdown with a finite drain period.

The `Actor` in a request is a claim, not proof. For every session-scoped
operation, the daemon reloads the canonical session from SQLite and verifies
the exact authenticated user. Local single-user identity is represented by
`None`; it never acts as a wildcard for authenticated records.

No request may silently fall back to an in-process runtime. A daemon outage is
reported as a dependency-unavailable response and must not create a second
scheduler or execution owner.

## Durable Control Model

Hive separates the user's desired work from attempts to perform it:

- **controller**: the durable objective and lifecycle requested by a user;
- **schedule**: recurrence, timezone, misfire, overlap, and retry policy;
- **occurrence**: one logical due time produced by a schedule;
- **run**: one logical execution of a controller or occurrence;
- **attempt**: an immutable try within a run;
- **controller event**: the ordered audit and replay stream;
- **idempotency record**: the durable response to a mutation key;
- **daemon lease**: the current claimant and fencing generation.

This model intentionally does not promise mythical exactly-once execution.
Claims are at-least-once; control-plane mutations and schedule materialization
are made effectively once through durable idempotency keys, immutable attempts,
and fencing checks. A tool side effect interrupted before acknowledgement is
uncertain work and moves to `recovery_required` rather than being called
exactly-once or replayed blindly.

### Durable Run States

Schedules and controllers create queued runs; `scheduled`, `paused`, and
`cancelling` are not hidden run statuses. The canonical persisted run lifecycle
is:

```text
queued -> leased -> running
leased -> queued | recovery_required
running -> sleeping | retry_wait | awaiting_input | recovery_required
        -> succeeded | failed | cancelled | dead_letter
sleeping | retry_wait | awaiting_input -> queued
recovery_required -> queued | failed | cancelled
queued | leased | sleeping | retry_wait | awaiting_input -> cancelled
failed | dead_letter -> queued (explicit retry)
```

Controller pause/disable and cancellation commands are separate control-plane
operations that drive legal run transitions. Transitions are validated in the
storage layer. A worker cannot declare a state by replacing arbitrary JSON.
Terminal history remains queryable.

### Claims and Fencing

Only the current daemon lease holder may claim work. Each successful lease
acquisition or takeover advances a fencing generation. A completion from an
older generation is rejected even if the old process resumes after a pause.

A claim is time-bounded and records its claimant. Heartbeats extend only a
matching claim. Work whose claim expires is reconciled from durable state; it
is not blindly marked successful or simply restarted.

### Idempotency

Every mutating control request carries a bounded idempotency key. The daemon
persists the command fingerprint and response. Repeating the same key and same
command returns the original result. Reusing a key for different input is a
conflict.

Schedule occurrence identity is derived from schedule plus logical due time,
so a restart cannot create a second logical run for the same occurrence.

### Frozen Execution Contract

Each queued run records the exact model, permission mode, workspace, project,
crew, and retry policy that the scheduler authorized. The execution host
revalidates that immutable claim against the current daemon fence immediately
before starting work; it does not silently substitute a daemon default or a
later session setting. Secret credential values are never copied into the run
record or event log. They are resolved from the current credential store when
the frozen model begins execution, so rotation does not rewrite durable work.
The daemon refreshes dynamic catalogs when credentials change or that exact
model is unknown; if it still cannot resolve the frozen model, execution is
refused rather than falling back to a different model.

### Exact Interactive Controls

Tool approvals and user responses are bound to an exact session, run, and tool
call. A session-only approval is rejected when it cannot identify one pending
run. Approval decisions are written to a durable outbox before the control API
acknowledges them, delivered only to the currently fenced execution, and
discarded when that run becomes terminal or requires recovery. The durable
event log distinguishes a queued decision from one consumed by the execution
host.

## Scheduling Semantics

The scheduler is a durable dispatcher, not an in-memory timer collection.
Supported recurrence includes one-shot, daily, weekdays, weekly, and monthly
rules with an IANA timezone.

Each schedule explicitly defines:

- local wall-clock recurrence and timezone;
- daylight-saving gap/fold policy;
- misfire policy for downtime;
- overlap policy when previous work is still active;
- retry limit and backoff;
- next logical due time and revision.

The claim pump reads due schedules in bounded batches and uses transactional
claims. Multiple daemon processes may briefly exist during supervision or an
upgrade, but fencing makes only one generation authoritative.

The daemon never changes process-global current working directory to run a
schedule. Each execution receives an explicit, frozen workspace and its tool
context is scoped there. The current in-daemon execution host still inherits
its supervised service environment, including that environment in shell tools.
Deployments must therefore keep the service-manager environment minimal and
use an `UnsetEnvironment=` drop-in for unrelated ambient credentials or
bootstrap tokens. Provider and push variables that the deployment intentionally
uses remain part of the daemon's trusted service environment. A strict,
allow-listed environment for each run belongs at the future child-worker
boundary; until then the service environment itself is a security boundary.

## Crash Recovery

Startup recovery distinguishes known-safe work from uncertain work:

- queued or due work can be claimed normally;
- a future sleeping wake is restored from its durable deadline;
- an expired claim returns to reconciliation;
- an attempt recorded as running when its worker disappeared becomes
  `recovery_required`;
- a human or a backend-specific reconciler may retry an uncertain attempt with
  a new attempt ID;
- a stale process cannot complete work after its fencing generation changes.

This avoids the dangerous shortcut of replaying a command whose side effects
may already have happened.

## Events and Observation

Every controller has a monotonic durable event sequence. Live delivery is a
convenience layered over that event log:

1. a subscriber asks for events after sequence `N`;
2. the daemon replays durable events after `N` in order;
3. it then bridges to a bounded live channel;
4. a gap or lag is explicit, never silently skipped;
5. the client reconnects using its last durable sequence.

UI-facing summaries remain separate from model-facing conversation history.
Large raw tool output is not retained indefinitely merely because it was once
streamed to a client.

Execution events have an explicit two-part contract. The live payload may be
delivered only to the authenticated bounded subscriber channel; its optional
durable payload is a small allow-listed summary. Hidden reasoning and provider
signatures are suppressed even from live delivery. Text deltas, tool-output
deltas, tool arguments/results, and web bodies are never written to the
controller journal. Live-only events have no durable sequence. Replayable
events are inserted first and then published exactly once at that allocated
sequence.

The controller journal is bounded by both row count and encoded size. Cleanup
deletes only a contiguous old prefix, always retains the high-water event, and
protects unresolved approvals and questions. Canonical open and recovery run
truth remains in the run and control-outbox tables rather than pinning replay
history. At most 32 interactions may remain unresolved. While any are pending,
the journal reserves 32 rows and bounded byte headroom exclusively for exact
approval/response resolutions. Unrelated appends that would consume that
reserve fail with `resource_exhausted`; resolutions can therefore make progress
in any order without exceeding the hard 2,048-row/2-MiB ceiling. A client whose
cursor predates the retained prefix receives an explicit replay-gap event.
Expired idempotency rows and old delivered/discarded control-outbox rows are
removed in bounded batches; pending controls are never age-pruned.

## Identity, Personality, and Continuity

Personality is a durable product subsystem, not a decorative prompt suffix.
Hive uses five revisioned, database-owned profile documents:

- `SOUL`: voice, temperament, and behavioral values;
- `IDENTITY`: name and stable self-description;
- `USER`: user-authored preferences and collaboration context;
- `HEARTBEAT`: standing autonomous-work guidance;
- `CHANNELS`: channel-specific communication behavior.

Profile ownership comes from the authenticated user record. A repository cannot
take ownership of a user's identity by placing a similarly named file in a
workspace. A guarded legacy importer can seed profiles, but imported documents
are recorded and revisioned.

Prompt construction has distinct cache and trust tiers:

1. base Mitsuro behavior and safety;
2. stable Hive identity profile;
3. project instructions and trusted project context;
4. current session state, retrieved episodes, and transient steering.

Stable identity is kept in the provider's reusable prompt prefix when the
provider supports caching. Dynamic work state never leaks into that prefix.

### Episodes

Conversation episodes index bounded user and assistant text only. Tool calls,
tool output, hidden thinking, pending steering, and malformed content are not
episode memories. Retrieval is exact-owner and optional exact-project scoped,
excludes the current session, and injects only a small number of relevant
snippets.

An episode is evidence, not automatically a fact about the user.

### Governed Learning

Post-turn learning is asynchronous and best-effort. The reviewer receives a
bounded text-only transcript, has no tools, and must return strict structured
candidates with evidence. Candidate policy is deterministic:

- explicit, non-sensitive preferences or corrections may be promoted;
- inferred preferences and project knowledge require review;
- sensitive facts are rejected by default;
- a forget request creates a tombstone/supersession rather than merely deleting
  an index row;
- retries are checkpointed and deduplicated by evidence plus canonical key.

The reviewer cannot mutate the main conversation, provider cache, workspace,
or memory store directly. Generated project summaries live in
`knowledge_snapshots`; durable user memories live in the canonical memory
store with provenance and revision history.

## Security and Resource Governance

Autonomous execution inherits the same permission and delegated-turn contract
as the parent controller. A sub-agent, extension, MCP server, or direct tool
route cannot silently gain a broader policy.

Project skills are discovered from the exact frozen project root for each run;
they are not inherited from the daemon's launch directory. Project MCP is
deliberately fail-closed in autonomous daemon runs today. In particular, the
daemon does not expose `.mcp.json` from its own working directory to every run,
and an HTTP-process MCP connection is not treated as authority in another
process. Enabling project MCP requires a canonical project-and-config trust
record plus explicit MCP child/server lifecycle ownership in the daemon.

Production hardening requires:

- canonical workspace-root validation before execution;
- no secrets in process arguments or event payloads;
- per-run working directory plus a minimal daemon environment;
- bounded turns, runtime, output, event queues, and concurrency;
- explicit network and shell policy;
- redacted structured traces;
- systemd/launchd supervision with a private runtime directory;
- worker termination that cannot kill unrelated user processes.

The systemd unit applies restart policy, filesystem protections, resource
limits, and a private runtime directory. A future child-worker backend can add
stronger per-run cgroup, namespace, or container isolation without changing the
control protocol or durable scheduler.

## Operational Contract

A healthy deployment proves all of the following, rather than inferring health
from a successful build:

- the socket and service units are installed and tracked;
- the socket accepts an authenticated `Ping`;
- `Stats` reports the expected daemon instance and queue state;
- the HTTP server is connected to that daemon and has no embedded owner;
- a scheduled test occurrence survives a daemon restart without duplication;
- an interrupted running attempt is reconciled to `recovery_required`;
- event replay resumes from a known sequence without an invisible gap;
- cross-user control requests return not-found and perform no side effect.

## Evolution Rules

Protocol additions are backward-compatible within a major version. Durable
schema changes are forward-only. State transition and ownership policy live in
shared typed helpers, not independently in HTTP handlers, daemon handlers, and
clients. Any new execution backend implements the same claim, fencing,
idempotency, recovery, and event contracts.
