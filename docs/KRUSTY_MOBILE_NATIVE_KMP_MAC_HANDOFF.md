# Krusty Mobile Native KMP Mac Handoff

## Purpose
Resume document for continuing the native mobile build on macOS with iPhone-first design focus, after planning and Linux-side setup work in this repository.

## Current Intent
Build a native mobile Krusty client in parallel with the current PWA.

This mobile client is:
- remote-first
- backed by the existing Krusty server
- chat-first
- visually premium on iPhone
- allowed to diverge from the PWA where native UX is better

This mobile client is not:
- a local phone-hosted agent runtime
- a replacement for the PWA before release readiness

## Styling Direction Locked In
- Core accent: existing steel-blue chat-bubble color
- Visual style: selective glassmorphism
- iOS: first-class styling target
- Top and bottom chat chrome: blurred edge treatment so text feathers smoothly into the chrome
- Corners: square-ish rounded
- Elevation: medium shadow/elevation
- Motion: standard, clean, restrained
- Typography direction: distinctive, with `Victor Mono` as a likely accent/code face rather than default body copy

## Product Priority Locked In
Phase 1 mobile focus:
- connect to server
- trusted device bootstrap
- session list
- chat
- settings

Do not start by porting:
- terminal
- IDE
- workspace preview
- full dense PWA nav structure

## Linux vs Mac Workflow

### Linux Can Be Used For
- planning and documentation
- Kotlin Multiplatform shared-module setup
- Compose shared UI work
- Android development and emulator testing
- API contract work
- design token definition

### macOS Is Required For
- iOS simulator runs
- real iPhone device testing
- Xcode project handling
- iOS signing/provisioning
- final iOS build validation
- iOS-specific blur/material/keyboard/safe-area tuning

## Mac Prerequisites
Install and verify:
- Xcode
- latest stable JDK supported by the KMP toolchain in use
- Android Studio
- Kotlin Multiplatform plugin / Compose Multiplatform support in the chosen IDE
- CocoaPods only if selected dependencies require it

## First Mac Session Plan

### 1. Open the repo and review planning docs
Read:
- `docs/KRUSTY_MOBILE_NATIVE_KMP_ROADMAP.md`
- this file

### 2. Create the mobile workspace
Target:
- `apps/mobile/`
- shared KMP module
- `androidApp`
- `iosApp`

### 3. Wire the basic app shell
Implement:
- app bootstrap
- environment/server config
- secure storage abstraction
- server connection test screen

### 4. Build the design token layer
Create:
- color roles
- blur/material roles
- typography roles
- corner/elevation scale
- spacing/motion scale

### 5. Build chat-first screens only
Order:
- server picker / connect flow
- session list
- chat view
- settings

### 6. Run on iPhone simulator early
Validate:
- safe areas
- keyboard avoidance
- blurred chrome
- glass surfaces
- chat readability
- scroll behavior during streaming

## Backend Readiness Work To Confirm
Before deep mobile feature work, confirm the existing backend covers:
- remote token/bootstrap flow suited for native storage
- stable session list/load/create/update API
- stable chat streaming API for mobile clients
- model list/default model API
- push registration story
- reconnect/reload session recovery

If gaps are found:
- patch the backend contract first
- do not pile client-side workarounds into the mobile app

## Design Notes for the First Native Chat Pass

### Chat Surface
- use steel-blue as the signature accent, especially for outgoing/user-owned emphasis and active states
- keep background layers muted so blur and chrome read cleanly
- ensure text contrast remains excellent against translucent surfaces

### Blur Chrome
- top blur should softly protect the nav/header area
- bottom blur should protect the composer and create the feathered text-edge effect
- avoid over-blurring the main scroll body

### Typography
- test `Victor Mono` only as an accent/code voice first
- keep body text optimized for chat reading comfort
- if `Victor Mono` is too stylized at body sizes, keep it for code and metadata only

### Motion
- prioritize send/stream/scroll/composer transitions
- use short, clean transitions
- avoid ornamental animation until the chat loop feels right

## Branch Resume Checklist
When resuming on Mac, complete in order:
- verify docs are current
- create `apps/mobile/`
- get Android + iOS targets compiling
- connect to existing server
- build theme tokens
- ship one polished chat screen
- validate on iPhone simulator

## Push Readiness for the Planning Branch
Before pushing the current planning branch:
- planning docs exist and are readable
- mobile direction is clearly chat-first
- iOS-first styling direction is explicit
- backend readiness checklist is captured
- Linux and Mac roles are separated clearly

## Resume Summary
If reopening this work later, the direction is:

Build a separate native mobile client in Kotlin Multiplatform and Compose Multiplatform under `apps/mobile/`, keep the current PWA and desktop shell alive, make iPhone the styling reference, and prove the native app through a premium chat-first experience before expanding into specialty tabs.
