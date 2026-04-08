# Krusty Subagent Surface Parity

## Purpose

Phase 7 deliverable for the subagent redesign roadmap.

This phase exposes the newer delegated truth contract on user-facing surfaces so the web client is no longer stuck on older, flatter delegated semantics.

## Problem Before

Core and server were carrying richer delegated facts than the PWA rendered:
- `investigation_summary`
- `confidence`
- `coverage_gap_notice`

That left important parent aggregation truth trapped below the surface.

## Implemented Surface Updates

### 1. PWA delegated state now understands parent aggregation fields

Delegated artifacts in the session store now preserve:
- `investigationSummary`
- `confidence`
- `coverageGapNotice`

Implemented in:
- [session.ts](/home/burgess/Work/krusty/apps/pwa/app/src/lib/stores/session.ts)

### 2. Delegated widget now surfaces parent investigation truth

The delegated web widget now shows:
- confidence in the header details
- parent investigation summary
- explicit coverage-gap notice when coverage is partial/degraded

Implemented in:
- [DelegatedToolWidget.svelte](/home/burgess/Work/krusty/apps/pwa/app/src/lib/components/chat/DelegatedToolWidget.svelte)

## What This Achieves

The web surface is now closer to the core/server delegated contract:
- parent aggregation truth is visible
- partial delegated runs are easier to interpret
- the delegated card is less dependent on raw child rows alone

## What Still Remains

This phase still leaves open:
- further parity review against TUI semantics on live delegated runs
- closure validation across reconnect/reload scenarios
