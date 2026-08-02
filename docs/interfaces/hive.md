# Hive

Hive is Mitsuro's durable background-work mode. Give it an objective, let it
work independently, and return later to review progress or results.

## What Hive does

A Hive run can:

- work inside an approved project directory;
- use the same agent tools and provider setup as interactive sessions;
- break an objective into smaller tasks;
- pause, resume, wait for input, and recover after a restart;
- stream progress to mobile, web, and command-line clients;
- run once or from a schedule.

Hive is designed for work that should not depend on a phone, browser tab, or
terminal remaining open.

## How it works

The Mitsuro server accepts control requests from clients. A separately
supervised Hive service owns scheduling and execution so background work does
not stop when a client disconnects.

Run state is stored durably. Clients can reconnect and continue from the last
recorded event instead of treating the live stream as the only record of what
happened.

Hive does not claim that external side effects happen exactly once. If a
process stops after an action may have run but before the result is confirmed,
the attempt is marked for recovery rather than replayed blindly.

For the full engineering contract, see
[Hive backend architecture](../hive/architecture.md).

## Command-line controls

The primary installed command is `mitsuro`:

```bash
mitsuro hive run "Update the API documentation"
mitsuro hive status
mitsuro hive attach <run-id>
mitsuro hive pause <run-id>
mitsuro hive resume <run-id>
mitsuro hive send <run-id> "Focus on the mobile client first"
mitsuro hive cancel <run-id>
```

Run `mitsuro hive --help` for the current options.

The deprecated `krusty hive` spelling remains a tested alias for existing
automation during the announced transition window. New documentation and
product copy use Mitsuro and Hive.

## Control and safety

Each run has an explicit workspace, model, permission mode, and ownership
context. Approvals and follow-up messages are tied to the exact session and run
rather than whichever conversation a client currently has open.

Schedules record timezone, overlap, missed-run, and retry behavior. The Hive
service uses leases and fencing so a restarted or stale process cannot report
itself as the active owner of newer work.

## Data and privacy

Hive stores compact, structured state needed for recovery and observation. Raw
provider reasoning, tool output, web content, and secrets are not treated as a
permanent event log.

See [Hive engineering documentation](../hive/README.md) for migration and
operational guidance.
