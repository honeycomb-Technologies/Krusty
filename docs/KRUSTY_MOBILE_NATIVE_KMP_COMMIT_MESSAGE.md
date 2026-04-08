# Krusty Mobile Native KMP Commit Message

## Subject
`scaffold native KMP mobile workspace and planning docs`

## Body
```text
Add the first Kotlin Multiplatform mobile scaffold under apps/mobile.

- create shared Compose Multiplatform workspace shape
- add Android entrypoint and iOS handoff placeholder
- add chat-first native shell and Krusty mobile theme tokens
- add shared mobile API contract and route-level server client scaffold
- add roadmap, tracker, Mac handoff, and bring-up checklist docs
- allow docs/ additions to be committed by removing the blanket ignore

This keeps the current PWA and desktop shell intact while preparing a
native mobile client that targets the existing remote Krusty server.
```
