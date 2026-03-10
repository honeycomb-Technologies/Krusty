# AGENTS Guide: /crates/krusty-core/src

## Purpose
Core runtime implementation modules.

## Guardrails
- Keep subsystem contracts explicit between AI, tools, storage, plugins, and protocols.
- Prefer typed boundaries over ad-hoc JSON passing.
- Any cross-cutting change should include targeted tests.
- Keep live compaction separate from pinch/session handoff; do not use pinch as the default overflow path.
- Keep loop budgets and streaming timeouts explicit and shared across callers; do not hide behavioral caps inside transport layers.
- Keep UI-facing tool output separate from model-facing history retention; long raw tool output should not be preserved in conversation history unless the exact payload is still needed for the next turn.
- Keep agent tool approval/retry/result policy centralized in the agent control layer rather than embedding it ad hoc in transport or tool implementation code.
- Keep delegated execution (subagents, MCP, extensions, skills) on explicit inherited governance contracts for permission mode and turn budget; delegated paths must not silently bypass parent policy.
- Resolve delegated turn budgets and permission mode from the shared contract at execution time; do not duplicate drift-prone defaults across subagent or route surfaces.
- Keep plan lifecycle state canonical in core helpers; active-vs-archived plan resolution and effective work mode must not be re-derived independently in UI or server layers.
- Capture runtime observability from the canonical `LoopEvent` boundary; do not add drift-prone provider/tool/UI-specific trace streams for the same execution path.
- Collapse repeated database/session side-effect plumbing behind shared local helpers in hot-path agent code instead of reopening identical boilerplate per helper function.

## Key Local Guides
- `ai/AGENTS.md`
- `storage/AGENTS.md`
- `tools/AGENTS.md`
- `extensions/AGENTS.md`
- `plugins/AGENTS.md`
