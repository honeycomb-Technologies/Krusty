# Krusty Broad Explorer Audit Findings

## Scope

Focused audit of the broad multi-target delegated `explore` path after scoped explorer recovery.

Primary evidence:
- live audit session `d75b8086-111d-41d3-8f72-3f7cb7cd0a90`
- [KRUSTY_LIVE_EXPLORER_AUDIT_2026_03_11.md](/home/burgess/Work/krusty/docs/KRUSTY_LIVE_EXPLORER_AUDIT_2026_03_11.md)

## Findings

### 1. Broad audits are still executed as one large delegated run

File:
- [explore.rs](/home/burgess/Work/krusty/crates/krusty-core/src/tools/implementations/explore.rs)

Issue:
- all requested targets are packed into one `explore` call and one `delegated_run_id`
- provider concurrency for MiniMax is clamped to `1`
- so a six-target audit becomes one long serialized sweep

Impact:
- weak child results drag down the whole audit
- runtime is slow and expensive
- there is no batch-level checkpointing or aggregation boundary

### 2. Forced-summary fallback is still too permissive in broad runs

File:
- [execution.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/subagent/execution.rs)

Issue:
- when stale read-only cycles trigger forced synthesis, the loop still prefers `final_output` if non-empty
- that means repair-phase prose can survive too long before normalization

Impact:
- broad runs still show weak summaries like “Let me try...” or “Let me check...”
- deterministic path-based synthesis does not always become the first-class fallback early enough

### 3. Placeholder rejection is narrower than the broad-run failure language

File:
- [types.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/subagent/types.rs)

Issue:
- `summary_looks_non_substantive()` catches some phrases
- broad runs still produce other low-value completion text that is not clearly architecture-grade

Impact:
- some degraded summaries can still look superficially valid long enough to contaminate larger audits

### 4. Dense targets are under-served by the current assignment model

Files:
- [explore.rs](/home/burgess/Work/krusty/crates/krusty-core/src/tools/implementations/explore.rs)
- [execution.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/subagent/execution.rs)

Issue:
- `agent` and `ai` need stronger architecture-specific exploration than clearer module trees like `tools` or `storage`
- current assignments are still too uniform across targets

Impact:
- `tools`, `storage`, `server`, and `tui` often succeed
- `agent` and `ai` still degrade or become placeholder-heavy

### 5. The progress model still hides batch boundaries

Files:
- [mod.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/subagent/mod.rs)
- [types.rs](/home/burgess/Work/krusty/crates/krusty-server/src/types.rs)

Issue:
- delegated progress is run-level, but there is no first-class batch concept above individual child tasks
- one big audit looks like one long delegated run rather than a sequence of bounded sub-investigations

Impact:
- harder to reason about stability
- harder to isolate weak targets from strong ones
- harder to give the parent a clean aggregation seam

### 6. The current prompt says “parallel” when MiniMax execution is actually serialized

Files:
- [explore.rs](/home/burgess/Work/krusty/crates/krusty-core/src/tools/implementations/explore.rs)
- live audit prompt/trace

Issue:
- user-facing behavior and internal logic still frame large audits as parallel exploration
- MiniMax path is intentionally serialized for reliability

Impact:
- misleading mental model
- longer runtime than the prompt/UX implies

## Conclusion

The broad-audit problem is not one bug.

It is a design gap:
- no batch-level broad exploration model
- forced-summary path still too tolerant of weak intermediate prose
- dense targets need stronger target-specific assignments
- MiniMax broad-run strategy still needs explicit shaping rather than relying on the scoped explorer contract
