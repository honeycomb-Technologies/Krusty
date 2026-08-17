# Tick 001 — crates/mitsuro-core/src/plan

- Date: 2026-08-17
- Mode: propose-only
- Product code edited: no
- Critic verdict: pass
- Apply allowed: no (user has not said apply)

## Approved for a later apply

### plan-1
- Kind: idiom
- Paths: `crates/mitsuro-core/src/plan/file/markdown.rs:367`, `crates/mitsuro-core/src/plan/file/response.rs:223`
- Change: one private/`pub(super)` `parse_checkbox_line`; both `from_markdown` and `try_parse_from_response` call it. Do not export on `PlanFile` / `PlanTask`.
- Why: the two functions are the same. Unification deepens the parse boundary. Tests stay unmodified.
- Critic: no findings.

## Blocked

- plan-2 (`PlanTask::is_completed` including `to_context`) — `to_context` today uses only `!t.completed`; folding `status` in would hide tasks and change AI context.
- plan-3 (unify task-identity parsers) — markdown and response accept different empty-desc / bold / no-colon cases. Unifying changes parse results.
- plan-4 (share metadata line parser) — response would start storing timestamped `Result [ts]:` lines it ignores today.
- plan-5 (one DB handle in `has_active_workflow_or_plan`) — no named workload; naive share would run plan-dir setup on the Active-goal path that skips it today.
- plan-6 (skip full `to_markdown` in `to_context`) — truncated context is not a prefix of the full document; a size estimate can change what the model sees.

## Fences left in place

`extract_completed_task_ids`, `save_plan`, `list_completed_for_dir`, the two `PlanSummary` types, `has_plan` vs `get_active_plan`, phase indent cap, `migrate_legacy_plans`.
