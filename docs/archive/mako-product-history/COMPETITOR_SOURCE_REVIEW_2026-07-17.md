# Mako Backend Source Review: OpenClaw and Hermes Agent (2026-07-17)

## Scope and Method

This review concerns backend behavior, not frontend design. The repositories
were refreshed on Honey on July 17, 2026 and inspected at these exact revisions:

- [OpenClaw](https://github.com/openclaw/openclaw) `28c2f4c07543786ffa8a086c1ebaee0c19e22f63`
- [Hermes Agent](https://github.com/NousResearch/hermes-agent) `d59b79fadd1e9edd7afc5c679cc3b143838e7c01`

The corresponding [Hermes documentation](https://hermes-agent.nousresearch.com/docs/)
was used to cross-check intended behavior. A pinned source review is important:
both projects move quickly, and a product decision should not rest on a vague
memory of their current marketing pages.

Pinned implementation landmarks used for the comparison:

- OpenClaw's [systemd gateway owner](https://github.com/openclaw/openclaw/blob/28c2f4c07543786ffa8a086c1ebaee0c19e22f63/src/daemon/systemd.ts)
  and [workspace bootstrap contract](https://github.com/openclaw/openclaw/blob/28c2f4c07543786ffa8a086c1ebaee0c19e22f63/docs/start/bootstrapping.md);
- Hermes' [stable/context/volatile prompt tiers](https://github.com/NousResearch/hermes-agent/blob/d59b79fadd1e9edd7afc5c679cc3b143838e7c01/agent/system_prompt.py),
  [post-turn finalizer](https://github.com/NousResearch/hermes-agent/blob/d59b79fadd1e9edd7afc5c679cc3b143838e7c01/agent/turn_finalizer.py),
  [profile isolation contract](https://github.com/NousResearch/hermes-agent/blob/d59b79fadd1e9edd7afc5c679cc3b143838e7c01/docs/profile-routing.md),
  and [durable cron execution ledger](https://github.com/NousResearch/hermes-agent/blob/d59b79fadd1e9edd7afc5c679cc3b143838e7c01/cron/executions.py).

## Executive Judgment

Hermes feels more alive because continuity is structural. Its identity is loaded
early, its home is profile-specific, and memory review happens after ordinary
conversation. OpenClaw feels more operationally complete because it treats its
Gateway as installed infrastructure and has mature scheduling, channel, and
service-management machinery.

Mako should combine those strengths without copying their sharpest edges:

- OpenClaw-grade process ownership and operational visibility;
- Hermes-grade stable identity and continuity;
- stricter learning governance than Hermes;
- explicit normalized scheduling and process-fencing semantics alongside
  OpenClaw's already mature durable scheduler;
- Krusty's typed tool governance, project isolation, and orchestrator reuse.

## Comparative Matrix

| Concern | OpenClaw | Hermes Agent | Mako direction |
| --- | --- | --- | --- |
| Long-lived owner | Gateway daemon and installed service | Gateway plus cron loop | Dedicated `krusty-mako` daemon; HTTP is only a client |
| Personality | Workspace bootstrap files including soul/identity/user | `SOUL.md` first, profile-scoped home and memory | Revisioned DB profile, stable prompt tier, guarded import |
| Memory | Workspace/user memory conventions | Memory/user files plus post-turn review | Canonical provenance store plus episodes and governed candidates |
| Scheduling | Shared SQLite state/run history, queued manual runs, idempotency, reconciliation, generation guards | Cron gateway with a durable execution ledger and periodic tick | Transactional DB claims, normalized occurrences/runs/attempts, leases, fencing |
| Recovery | Mature startup/timeout diagnostics and durable task reconciliation | SQLite-backed audit history and richer session/shutdown recovery | Explicit `recovery_required` uncertain-work state; never blind replay |
| Multi-tenancy | Primarily personal gateway assumptions | Profile isolation | Exact DB ownership check for every daemon command |
| Events | Rich gateway/channel event model | Conversation and gateway events | Monotonic durable controller log plus bounded live bridge |
| Execution isolation | Gateway-mediated workers/tools | Gateway jobs; some process-global workdir handling | Per-worker workdir/environment; no process-global mutation |

## What OpenClaw Gets Right

### The process is a product

OpenClaw does not treat background execution as an HTTP request that happens to
last a long time. Its Gateway has install, start, stop, diagnostics, and service
supervision paths. That distinction matters: if a UI server owns autonomous
loops, a deploy or client outage becomes an execution outage.

Mako adopts the same core lesson with a separately supervised daemon and a
private control socket. It goes further by making production server startup
fail closed instead of quietly becoming a second runtime owner.

### One observable control plane

OpenClaw's Gateway centralizes sessions, channels, tools, and events. The useful
idea is not that everything must share one giant module; it is that there must
be one authoritative owner and one place to inspect what it believes.

Mako retains modular Rust boundaries while exposing one daemon protocol and one
durable controller event sequence.

### Bootstrap files make behavior inspectable

Files such as `AGENTS.md`, `SOUL.md`, `IDENTITY.md`, `USER.md`, `HEARTBEAT.md`,
and `MEMORY.md` give users a vocabulary for behavior. OpenClaw also demonstrates
that reading these files needs size limits, missing-file handling, and bootstrap
attestation rather than naïve concatenation.

Mako keeps the vocabulary but uses a revisioned database as authority. A legacy
file importer is bounded and explicit; a random repository cannot impersonate
the user's profile.

### Where Mako should not copy OpenClaw

OpenClaw's breadth carries control-plane complexity, but its current scheduler
is itself mature: it shares SQLite state and run history, queues manual runs,
uses idempotency and active-marker generation guards, reconciles durable tasks,
and exposes extensive startup and timeout diagnostics. Mako's differentiation is
narrower: an explicit controller/schedule/occurrence/run/attempt schema,
process-generation fencing in the database, exact actor ownership, and a
first-class `recovery_required` outcome for uncertain side effects.

## What Hermes Gets Right

### Identity precedes task behavior

Hermes gives `SOUL.md` first-class position and scopes home/state by profile.
That makes voice stable across channels and sessions. The feeling of personality
does not primarily come from whimsical prose; it comes from recognizing which
parts of the agent should remain stable while the task changes.

Mako formalizes that into separate base, identity, project, and session prompt
tiers. Stable identity can benefit from provider prompt caching while current
task state remains dynamic.

### Continuity is active, not archival

Hermes uses `MEMORY.md` and `USER.md` as live context and runs background review
after turns. This closes a common product gap: a system may store transcripts
forever yet still feel as if every conversation starts from zero.

Mako adds bounded episode retrieval and post-turn candidate extraction so past
conversation can affect the next one without replaying entire transcripts.

### Warm prompt caching is an architectural concern

Hermes distinguishes stable, contextual, and volatile prompt material. That is
both a latency/cost optimization and a behavioral stability tool. Mako mirrors
the separation in its provider request builders rather than allowing each
transport to assemble prompts differently.

### Where Mako should not copy Hermes

Hermes' background reviewer can be very proactive about writing memory or
skills. That produces continuity, but it grants a secondary model broad power to
turn inference into durable behavior. Mako's reviewer has no tools. It proposes
evidence-linked candidates; deterministic policy decides whether a narrow class
of explicit, safe corrections/preferences may be promoted.

Hermes now maintains a durable cron execution ledger and has materially richer
SQLite/session shutdown recovery than its earlier JSON-job and file-lock design.
That ledger is intentionally an audit history rather than a retry queue;
interrupted attempts become unknown after their exact owner dies. Mako still
needs a stricter multi-process contract: lease generations and fencing tokens,
a normalized occurrence/run/attempt model, exact actor ownership on every state
change, and explicit recovery review for uncertain work. Workdir-oriented jobs
that depend on changing process-global state also serialize unrelated jobs and
make isolation harder. Mako binds workspace and environment to each worker
request.

## Original Design Conclusions

### Personality is four systems, not one prompt

A durable personality requires all four of these:

1. a stable identity and voice;
2. explicit user preferences;
3. selective episodic recall;
4. a governed way to learn or forget.

Removing any one produces a familiar failure: style without recognition,
recognition without accuracy, memory without consent, or learning that drifts
into invented familiarity.

### A daemon boundary is also a correctness boundary

Making Mako its own executable is not enough. If the HTTP server can still run
the same loops when the daemon is absent, the product has two authorities and
will eventually duplicate work. The important invariant is single ownership,
enforced by fail-closed clients, durable claims, and fencing.

### A scheduler is a reliability subsystem

Recurrence syntax is the easy part. A serious scheduler must specify timezone
folds/gaps, downtime misfires, overlaps, retries, idempotency, claims, fencing,
and uncertain side effects. Without those answers, "cron" is only a timer UI.

### Background learning should be asymmetric

It is cheap for a reviewer to propose knowledge and expensive for the product
to unlearn a false belief. The correct default is therefore permissive proposal
and conservative promotion. Evidence, provenance, tombstones, revisions, and a
review queue turn personality into something users can trust.

### Live streaming cannot be the audit log

Broadcast channels optimize immediacy; they do not guarantee history. Durable
ordered events must be authoritative, with live delivery treated as a bounded
tail. A slow or disconnected client should see an explicit gap and resume from
a sequence number.

## Resulting Mako Backend Priorities

The implementation order follows dependency rather than visual prominence:

1. establish the authenticated daemon protocol and single production owner;
2. normalize controllers, schedules, occurrences, runs, attempts, and events;
3. add transactional claims, idempotency, leases, fencing, and reconciliation;
4. proxy every server control path through the daemon and preserve ownership;
5. add revisioned identity, episodes, canonical memory, and governed learning;
6. prove restart, outage, replay, lag, and cross-user behavior in tests;
7. only then expose richer frontend controls.

This produces a Mako that can feel as continuous as Hermes while being easier
to operate, audit, and recover than a collection of in-memory agent loops.
