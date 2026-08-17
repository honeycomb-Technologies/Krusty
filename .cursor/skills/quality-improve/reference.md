# Quality Improve Reference

Read this after the constitution. It is the technique catalog and the Mitsuro
map. Do not load it unless you are scoring or writing a slice.

## Techniques

### Deep modules (Ousterhout)

A module is deep when its interface is small relative to what it does. Unix
`open` / `read` / `write` is deep. A pass-through `addNullValueForAttribute`
is shallow.

**Deletion test:** if deleting the wrapper concentrates complexity in the
caller, keep and deepen it. If complexity vanishes, inline it.

**Not the goal:** many tiny files. That is a thicket of shallow modules.

### Simple ≠ easy (Hickey)

Simple means one fold: not braided. Easy means familiar. Complecting is
braiding state, time, identity, and I/O. Prefer values and explicit data
transforms over objects that hide time.

### Parse, don't validate (King)

A validator returns `bool` and throws the proof away. A parser returns a
refined type. After the boundary, take `Email`, not `String`.

In Rust that is a newtype or enum constructed in one `parse` / `TryFrom`.
In TS that is a zod (or equivalent) parse at the route or form edge, then
branded or inferred types inward.

### Functional core, imperative shell (Bernhardt)

Core: data in, data out, no clock, no disk, no network. Shell: read the
world, call the core, write the world. The shell stays short. If a
conditional on domain data appears in the shell, it belongs in the core.

### Seams that pay rent

Add a seam only for nondeterminism, I/O, or a second implementation that
already ships. Rule of three for the rest. The consumer owns the narrow
capability type. Wire the graph in one boring place.

### Bottom-up operators (Graham)

If three call sites want the same move, name the operator and let the
program shrink. Do not invent a framework. Do not extract a one-call helper
unless the name is the concept.

### Design it twice

When the interface is the slice, sketch two shapes, pick the deeper one,
write only that. Do not land both.

### Simplicity first (Karpathy-style)

No speculative feature, no flexibility that was not asked, no interface
before the third case. If 200 lines could be 50 without losing an invariant,
prefer 50. Line count is not the goal; unearned lines are the smell.

## Mitsuro map

| If you see | Prefer |
|---|---|
| Logic in `mitsuro-cli` or a route handler | Move the decision to `mitsuro-core`, keep the shell thin |
| A new path-validation helper | Use `ToolContext` / registry policy |
| Provider if-ladders at a call site | Shared AI transform / model profile |
| Plan status parsed from prose or re-derived in UI | Canonical plan lifecycle helpers |
| Permission mode or turn budget copied into a child | Inherited governance contract |
| Raw JSON used past a route | Parse once into a typed request |
| Tool result as a loose string | Structured envelope (`ok`, `data`, `error`, `warnings`) |
| Full transcript subscribed from a shell | Active conversation owns the transcript |
| Work during React render | Effects only, after render |
| Migration that rewrites old rows in place | Forward-only migration, compatibility reader at the edge |
| Internal name in UI copy | Translate at the boundary to Mitsuro / Agent / Hive / Pulse |

## Candidate shape

```
### Candidate <id>
- Kind: deepen | unbraid | parse-once | idiom | hot-path
- Path: `file.rs:line`
- Interface now:
- Interface after:
- Why this is deeper / simpler, not smaller:
- Invariants preserved:
- Reject-list check: clean
- Blast radius: files, crates, tests
- Evidence: call sites or measurement
- Score:
```

## Sources

- John Ousterhout, *A Philosophy of Software Design* — deep modules
- Rich Hickey, *Simple Made Easy* — simple vs easy, complecting
- Alexis King, *Parse, don't validate*
- Gary Bernhardt, *Boundaries* — functional core, imperative shell
- Butler Lampson, *Hints and Principles for Computer System Design*
- Paul Graham, *Programming Bottom-Up*
- Anthropic `code-simplifier` — preserve behavior, recently changed code
- Addy Osmani `code-simplification` — Chesterton's Fence, one change, tests unmodified
- Matt Pocock `codebase-design` — module, interface, depth, seam, deletion test
- `coding-effectively` — seams as parameters, core/shell, parse at the edge
