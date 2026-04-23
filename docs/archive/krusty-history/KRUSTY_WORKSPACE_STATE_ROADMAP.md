# Krusty Workspace State Roadmap

> Archived historical planning/audit document.
>
> This file is preserved for project history. It may reference the former `apps/pwa/app`, Svelte-era client files, or old validation commands when describing historical implementation state.


## Purpose
Make Krusty workspace semantics explicit, neutral by default, and promotable into project mode only when the user selects or creates a project.

## Product Goal
Krusty should behave correctly in all of these modes:
- neutral chat with no active project
- system/config/admin work outside a repo
- explicit project work in a chosen directory
- new project creation starting from neutral mode

It must not infer project identity from server launch cwd, process cwd, or other hidden fallbacks.

## Architectural Goal
Separate these concepts cleanly:
- `server_root`: filesystem safety boundary only
- `execution_dir`: concrete cwd used for tools/processes
- `project_dir`: explicit project context for prompt injection, local skills, repo-aware behavior
- `workspace_mode`: semantic session state

## State Model
Krusty should support these session states:

### 1. Neutral
- no explicit project selected
- no project instructions
- no project-local skills
- no repo-aware framing
- tools run from a neutral execution cwd

### 2. ProjectSelected
- user explicitly chose a directory
- project instructions active
- project-local skills active
- repo-aware behavior allowed

### 3. ProjectCreated
- session started neutral
- Krusty created a new directory/project during the task
- session pivots into that directory
- subsequent turns behave like project mode

## Phase 1: Canonical Workspace Contract
Define and persist first-class workspace semantics.

Scope:
- add explicit `workspace_mode`
- add explicit `project_dir`
- keep `execution_dir` explicit or derive it canonically from workspace mode + policy
- make server, core, and PWA share one contract

Why:
- `working_dir` alone is ambiguous
- prompt behavior and tool behavior must stop inferring semantics from fallback cwd

Exit gate:
- session/runtime contract can represent neutral vs selected vs created without heuristics

## Phase 2: Prompt and Context Discipline
Make project context activation explicit.

Scope:
- inject project instructions only when `project_dir` exists
- inject project-local skills only when `project_dir` exists
- add explicit neutral-mode prompt guidance

Why:
- absence of repo context should be a deliberate state, not accidental behavior
- this is the core fix for repo identity leakage

Exit gate:
- neutral mode never frames itself as a repo assistant unless explicitly promoted

## Phase 3: Neutral Execution Policy
Define how tools behave when no project is active.

Scope:
- choose canonical neutral execution cwd policy
- separate neutral execution from project semantics
- ensure safe defaults for reads, inspection, and system tasks

Recommended policy:
- prefer user home for personal/local environments when available
- otherwise use configured server root
- never treat the neutral execution cwd as project context by itself

Exit gate:
- tools behave deterministically in neutral mode without turning neutral sessions into repo sessions

## Phase 4: Promotion Rules
Define when neutral mode becomes project mode.

Promotion triggers:
- explicit user directory selection
- explicit "work in this folder" direction
- new project creation by Krusty

Non-triggers:
- merely being launched from a repo
- one-off reads/writes to arbitrary machine paths
- having a concrete execution cwd

Exit gate:
- promotion happens only from explicit user/project actions and is persisted cleanly

## Phase 5: New Project Creation Flow
Make "build something from nowhere" first-class.

Scope:
- when user requests a new project in neutral mode, create/select a target directory
- persist that directory to the session
- promote workspace mode to `ProjectCreated`
- continue work in that directory automatically

Why:
- users should not need to pre-create a project just to start building

Exit gate:
- neutral sessions can smoothly become project sessions during real work

## Phase 6: System and Config Workflows
Protect non-project work from accidental promotion.

Scope:
- shell config edits
- machine admin tasks
- system inspection
- brainstorming
- compare/evaluate/explain tasks

Rules:
- remain neutral unless user explicitly selects or creates a project
- writes alone must not imply project identity

Exit gate:
- system/config work stays neutral and does not pick up repo framing

## Phase 7: Tool Governance by Workspace Mode
Align tools with workspace semantics.

Scope:
- repo-oriented tools should detect lack of project context and respond clearly
- project scaffolding flows should promote sessions when appropriate
- generic file/system tools remain usable in neutral mode

Why:
- prompt correctness alone is not enough if tools still assume repo context

Exit gate:
- tool behavior matches workspace mode, not hidden cwd assumptions

## Phase 8: Surface UX and Transparency
Make workspace state visible and controllable.

Scope:
- PWA/TUI/Desktop/server surfaces show neutral vs project state clearly
- expose current project directory only when explicit
- provide `Choose folder` / `Use this folder` / `Create project here` style actions
- show promotion transitions clearly

Exit gate:
- users can always tell whether Krusty is neutral or project-aware

## Phase 9: API and Persistence Finalization
Make the server API expose canonical workspace semantics.

Scope:
- session APIs expose `workspace_mode`, `project_dir`, and related state
- reload/recovery preserves workspace semantics
- no surface re-derives meaning from `working_dir` alone

Exit gate:
- UI and clients consume canonical workspace state directly

## Phase 10: Validation and Competitive Audit
Backcheck real scenarios and compare behavior to top coding agents.

Required scenarios:
- neutral brainstorming
- neutral system config edit
- new project created from neutral mode
- explicit project selection
- reload of neutral session
- reload of project session
- tools in neutral mode
- tools in project mode

Audit goals:
- no project prompt leakage in neutral mode
- promotion works and persists
- no surface drift
- behavior is elegant and unsurprising compared to professional tools

Exit gate:
- no unresolved high-severity drift in workspace semantics across core/server/PWA/TUI

## Non-Negotiable Principles
- `krusty-core` remains the single behavior brain
- `server_root` is a safety boundary, not project meaning
- project context is explicit, never accidental
- neutral mode is first-class, not a missing-data corner case
- promotion into project mode should feel seamless, not manual or brittle

## Professional End State
At completion:
- Krusty opens cleanly in neutral mode
- it handles shell-like and machine-level work naturally
- it does not assume repo identity from launch context
- it can create and adopt a new project during the session
- project-aware behavior activates only when it should
