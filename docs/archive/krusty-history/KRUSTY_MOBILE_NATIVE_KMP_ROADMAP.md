# Krusty Mobile Native KMP Roadmap

> Archived historical planning/audit document.
>
> This file is preserved for project history. It may reference the former `apps/pwa/app`, Svelte-era client files, or old validation commands when describing historical implementation state.


## Purpose
Define the build plan for a first-class native mobile Krusty client using Kotlin Multiplatform and Compose Multiplatform, while preserving the current PWA and desktop shell.

## Product Goal
Ship a chat-first native mobile app for iPhone and Android that connects to the existing Krusty server runtime running on user machines or remote hosts.

The mobile app must feel intentionally native and visually premium, especially on iPhone, while remaining API-compatible with the current server and leaving the PWA intact as an actively shippable surface.

## Non-Goals
- Replacing `krusty-server`, `krusty-core`, or the remote-first runtime model
- Abandoning the current PWA before native mobile is proven
- Achieving full terminal/IDE/workspace parity in the first mobile release
- Forcing Android to mimic iOS 1:1 when platform-specific polish is warranted

## Architecture Decision
Krusty mobile will be:
- a separate native client under `apps/mobile/`
- powered by Kotlin Multiplatform + Compose Multiplatform
- chat-first and remote-first
- backed by the existing HTTP/SSE/WebSocket server contracts

Krusty mobile will not:
- run the agent runtime locally on the phone
- replace the PWA as the only shipping client during initial development

## Product Shape
The initial native app should prioritize:
- server connection and trusted-device flow
- session list and recent sessions
- first-class chat and streaming responses
- model/session controls relevant to chat
- notifications, reconnect, and session resume
- settings and server management

Defer until after chat quality is strong:
- terminal
- IDE
- workspace preview
- advanced file browsing/edit flows
- dense power-user menus copied directly from the PWA

## Styling Direction
The mobile design system should preserve Krusty identity while making iPhone the aesthetic reference point.

### Visual Principles
- Core accent: the existing steel blue chat-bubble color
- Surface language: glassmorphism in selected chrome and overlays, not everywhere
- Blur treatment: top and bottom edge blur regions for chat so content feathers cleanly into the chrome
- Corners: square-ish rounded corners
- Elevation: medium shadow/elevation, never heavy or muddy
- Motion: restrained and clean, with crisp state transitions rather than flashy motion
- Priority: chat readability and comfort over decorative effects

### Platform Strategy
- iOS is the reference visual direction
- Android keeps the same brand tokens but uses platform-tuned surfaces and motion where needed
- Shared identity remains consistent across both platforms
- Platform divergence is allowed for blur, nav feel, sheet behavior, and edge chrome

### Typography Strategy
Primary recommendation:
- UI/body candidate: `Space Grotesk` or a similarly distinctive sans optimized for chat readability
- Accent/code candidate: `Victor Mono`

Guidance:
- do not use `Victor Mono` for long-form chat body text unless readability testing proves it works
- use the mono face for code, metadata, model/status chips, and brand accents
- keep chat reading comfort above typography novelty

## Phase 0: Backend Contract Readiness
Freeze the mobile MVP contract before major client work.

Scope:
- identify the exact server endpoints required for mobile MVP
- document chat/session/model/server-access contracts
- confirm streaming transport choice for chat on mobile
- define token/bootstrap flow for trusted device access
- identify gaps between current PWA needs and native mobile needs

Required mobile MVP backend surface:
- server discovery / saved server config
- remote access bootstrap
- sessions list/create/load/update
- chat send/stream/reload
- models/default model
- push registration lifecycle
- server status / active sessions

Exit gate:
- mobile MVP can be built without inventing new backend semantics midstream

## Phase 1: KMP Project Bootstrap
Create the new mobile workspace without disturbing the PWA.

Target structure:
- `apps/mobile/`
- `apps/mobile/shared/` or equivalent shared module for common code
- `apps/mobile/androidApp/`
- `apps/mobile/iosApp/`

Scope:
- Kotlin Multiplatform setup
- Compose Multiplatform setup
- baseline CI/build notes
- environment config for local servers and remote hosts

Exit gate:
- Android runs from Linux
- iOS target opens cleanly on macOS
- shared module compiles for both targets

## Phase 2: Shared Mobile Architecture
Create the common app architecture before screen implementation.

Scope:
- shared network client layer
- shared DTOs mapped from server contracts
- shared state/store patterns
- navigation architecture
- auth/session bootstrap state machine
- platform capability abstraction for notifications, secure storage, links, and haptics

Rules:
- backend contracts remain the source of truth
- no business logic is copied from the PWA unless it is explicitly being ported
- PWA behavior should inform mobile parity, but the implementation must be native-mobile shaped

Exit gate:
- auth, session loading, and chat streaming work from shared mobile code

## Phase 3: Design System and Theme Tokens
Build the native visual system before feature breadth.

Tokens to define:
- steel-blue accent scale
- glass surface opacity and blur levels
- surface/background layers
- corner radius scale
- border/outline treatments
- elevation/shadow scale
- motion durations and easing
- typography roles
- spacing scale

Components to define first:
- app background treatment
- top blur chrome
- bottom blur composer region
- chat bubbles
- input/composer
- sheet/modal
- segmented controls
- tab bar / primary nav
- cards and settings rows

Exit gate:
- one stable `KrustyTheme` exists
- iOS and Android theme variants are implemented deliberately, not ad hoc

## Phase 4: Chat-First MVP
Make chat excellent before adding specialty surfaces.

Screens:
- splash / bootstrap
- connect to server
- auth/trusted device flow
- session list
- chat view
- chat settings/model picker
- settings

Critical UX behavior:
- top and bottom blur with smooth text edge fade
- fast streaming updates
- strong keyboard handling
- reliable scroll anchoring and jump-to-bottom behavior
- safe-area correctness
- reconnect and resume behavior after app suspension

Exit gate:
- native chat experience is materially better than the current PWA on iPhone

## Phase 5: Native-Only Integrations
Add the capabilities that justify the native app.

Scope:
- secure token storage
- deep linking into server/session state
- push notifications
- notification tap routing into a session
- biometrics for app unlock or sensitive settings
- haptics for send/complete/error states
- share sheet support where useful

Exit gate:
- native app provides clear value over the PWA even before feature parity

## Phase 6: Special Tabs and Extended Surfaces
Add only after chat quality and native integrations are solid.

Candidate tabs:
- sessions/history
- preview/workspace status
- lightweight file/repo status
- approvals/plan/task controls
- server health / running session observer view

Guidance:
- terminal and IDE should be reconsidered for mobile ergonomics, not copied blindly from the PWA
- each added surface must prove that it belongs on a phone

Exit gate:
- tab set reflects actual mobile value, not feature-box parity

## Phase 7: Validation and Release Discipline
Make the mobile app production-safe without destabilizing the current system.

Required validation:
- server connection against local and remote Krusty instances
- trusted device bootstrap
- long chat streaming sessions
- reconnect after app background/foreground
- push delivery and tap routing
- iPhone safe-area and keyboard validation
- Android adaptive layout validation

Release principle:
- the PWA remains available until the native client is clearly better for its intended mobile job

## Backend Readiness Checklist
Before declaring backend ready for native mobile:
- document mobile MVP endpoints and payloads
- confirm chat streaming contract for mobile clients
- confirm remote token/bootstrap semantics for native secure storage
- identify any PWA-specific assumptions that should move server-side
- define push registration and notification event shape
- confirm session reload/recovery semantics are stable outside the browser

## Git Push Readiness
Before pushing the initial mobile planning branch:
- roadmap committed
- Mac handoff/resume doc committed
- target repo structure agreed
- backend readiness checklist captured
- mobile MVP scope fixed to chat-first
- design system direction fixed to steel-blue + selective glassmorphism

## Professional End State
At completion:
- Krusty has a premium native mobile client without sacrificing the PWA
- chat is the hero experience on phone
- backend contracts are stable enough to support multiple clients cleanly
- iPhone feels first-class
- Android is branded and polished without being an awkward iOS imitation
