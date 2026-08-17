# Apply readiness

Apply mode stays closed until every box is true.

- [x] Skill, reject list, reference, examples, ledger, and sector script are in `.cursor/skills/quality-improve/`
- [x] Five agents are in `.cursor/agents/`
- [x] Hive discovery symlink is `.mitsuro/skills/quality-improve`
- [x] `AGENTS.md` points at the loop
- [x] At least one propose-only tick produced a critic-reviewed candidate list
- [x] That tick changed no product code
- [ ] The user explicitly said apply
- [ ] The chosen slice is on the latest propose-only list

Current authority: **propose-only**.

Ready to apply when you say:

```
Apply quality-improve slice plan-1 from ticks/001-plan.md
```

That slice only. Do not reopen plan-2 through plan-6.
