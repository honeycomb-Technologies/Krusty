# Krusty Mobile Native KMP Tracker

> Archived historical planning/audit document.
>
> This file is preserved for project history. It may reference the former `apps/pwa/app`, Svelte-era client files, or old validation commands when describing historical implementation state.


## Program
Mobile Native KMP Roadmap

## Objective
Build a chat-first native mobile Krusty client in parallel with the existing PWA and desktop shell, using Kotlin Multiplatform and Compose Multiplatform.

## Current Baseline
- Roadmap captured for the native mobile track
- Mac handoff/resume doc captured for iPhone-first continuation
- `apps/mobile/` scaffold added with KMP/Compose-oriented project structure
- Shared theme tokens created for steel-blue accent, selective glass surfaces, and chat-first chrome
- Android entrypoint and common UI shell stubbed
- Shared mobile API contract and route-level client scaffolded against current server paths
- iOS wrapper intentionally deferred to macOS/Xcode setup

## Remaining Program Status

| Phase | Name | Status | Notes |
| --- | --- | --- | --- |
| 0 | Backend Contract Readiness | In Progress | Shared route-level mobile contract exists, but backend semantics still need live validation. |
| 1 | KMP Project Bootstrap | In Progress | Workspace scaffold exists, but Gradle wrapper and IDE/Xcode validation are still outstanding. |
| 2 | Shared Mobile Architecture | In Progress | Shared contracts, navigation shell, and route-level client exist, but concrete transport/state wiring is still pending. |
| 3 | Design System and Theme Tokens | In Progress | Initial theme tokens and frosted surfaces exist; typography assets and platform tuning remain. |
| 4 | Chat-First MVP | Pending | Connect, sessions, and chat flows still need server-backed implementation. |
| 5 | Native-Only Integrations | Pending | Secure storage, push, deep links, biometrics, and haptics not started. |
| 6 | Special Tabs and Extended Surfaces | Pending | Deferred until chat quality is strong. |
| 7 | Validation and Release Discipline | Pending | Requires Android and iOS validation on provisioned machines. |

## Completed Deliveries
- Added `apps/mobile/AGENTS.md`
- Added KMP mobile workspace skeleton under `apps/mobile/`
- Added shared design/theme scaffold for the new mobile client
- Added route-level mobile API contract and transport abstraction for existing server paths
- Added roadmap, tracker, and Mac handoff docs under `docs/`
- Added precise Mac bring-up checklist and suggested commit message
- Unblocked future doc commits by removing the blanket `docs/` ignore rule

## Known Gaps
- No Gradle wrapper checked in yet
- No local mobile build validation in this Linux environment
- No iOS app wrapper project yet
- No concrete HTTP/SSE transport implementation yet
- No bundled font assets yet

## Resume Checklist
- install mobile toolchain on Linux and macOS
- generate Gradle wrapper
- validate Android compile
- generate and validate iOS wrapper on macOS
- implement mobile API client against current server contracts
- replace placeholder screens with real connect/session/chat flows
