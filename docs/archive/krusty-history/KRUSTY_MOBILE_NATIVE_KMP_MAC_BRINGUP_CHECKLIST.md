# Krusty Mobile Native KMP Mac Bring-Up Checklist

> Archived historical planning/audit document.
>
> This file is preserved for project history. It may reference the former `apps/pwa/app`, Svelte-era client files, or old validation commands when describing historical implementation state.


## Purpose
Precise checklist for resuming this branch on the MacBook and getting the native mobile workspace from scaffold to first validated runs.

## Before Opening the Project
- pull the branch with the new `apps/mobile/` workspace
- confirm `docs/archive/krusty-history/KRUSTY_MOBILE_NATIVE_KMP_ROADMAP.md` is present
- confirm `docs/archive/krusty-history/KRUSTY_MOBILE_NATIVE_KMP_MAC_HANDOFF.md` is present
- confirm `docs/archive/krusty-history/KRUSTY_MOBILE_NATIVE_KMP_TRACKER.md` is present

## Mac Toolchain
- install Xcode
- install Android Studio
- install or select JDK 17 or 21 for Gradle/Kotlin work
- install Kotlin Multiplatform / Compose Multiplatform support in the IDE

## First Workspace Steps
1. Open `/home/burgess/Work/krusty/apps/mobile`
2. Generate the Gradle wrapper if still missing
3. Sync the Gradle project
4. Validate the shared module resolves
5. Validate Android target configuration
6. Create or wire the iOS wrapper project under `apps/mobile/iosApp`

## First Commands
Run from `/home/burgess/Work/krusty/apps/mobile`:

```bash
gradle wrapper
./gradlew tasks
./gradlew :composeApp:assembleDebug
```

After Android is stable:

```bash
./gradlew :composeApp:embedAndSignAppleFrameworkForXcode
```

## First iPhone Validation Pass
- open the iOS wrapper in Xcode
- run on iPhone simulator
- validate safe areas
- validate keyboard behavior
- validate blurred top/bottom chrome
- validate bubble readability and spacing
- validate chat scroll behavior

## First Backend Validation Pass
- point the mobile app at a local Krusty server
- validate `/health`
- validate `/remote-auth/status`
- validate `/remote-auth/bootstrap`
- validate `/api/server/access`
- validate `/api/models`
- validate `/api/sessions`
- validate `/api/chat` request construction for SSE work

## First Code Targets
- add a real transport implementation for `MobileApiTransport`
- replace connect screen stub with real saved-server flow
- load real sessions into the chat shell
- add secure token storage for trusted device flow
- wire the first SSE chat stream consumer
