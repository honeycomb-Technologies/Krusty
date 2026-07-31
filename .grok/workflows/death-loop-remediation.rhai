let meta = #{
    name: "death-loop-remediation",
    description: "Verify death-loop remediation goals G1-G6 against the krusty tree",
    phases: [
        #{ title: "Inventory", detail: "Confirm plan docs and target files exist" },
        #{ title: "Core tests", detail: "Progress, shell policy, process wake unit coverage" },
        #{ title: "Mobile", detail: "Composer soft-wrap unit tests" },
        #{ title: "Review", detail: "Adversarial check that goals are solid" },
    ],
    when_to_use: "After implementing Mitsuro death-loop fixes; run to verify G1-G6.",
};

let inventory_schema = #{
    "type": "object",
    "required": ["files_ok", "missing", "notes"],
    "properties": #{
        "files_ok": #{ "type": "boolean" },
        "missing": #{ "type": "array", "items": #{ "type": "string" }, "maxItems": 20 },
        "notes": #{ "type": "string" },
    },
};

let test_schema = #{
    "type": "object",
    "required": ["passed", "command", "summary"],
    "properties": #{
        "passed": #{ "type": "boolean" },
        "command": #{ "type": "string" },
        "summary": #{ "type": "string" },
    },
};

let review_schema = #{
    "type": "object",
    "required": ["solid", "gaps", "risks"],
    "properties": #{
        "solid": #{ "type": "boolean" },
        "gaps": #{ "type": "array", "items": #{ "type": "string" }, "maxItems": 12 },
        "risks": #{ "type": "array", "items": #{ "type": "string" }, "maxItems": 12 },
    },
};

phase("Inventory");
let inv = agent(
    "In /home/burgess/Work/krusty, verify these paths exist and briefly note purpose: "
        + "docs/plans/2026-07-31-death-loop-remediation.md, "
        + "crates/krusty-core/src/agent/progress.rs, "
        + "crates/krusty-core/src/agent/hooks/shell_policy.rs, "
        + "crates/krusty-core/src/process/registry.rs, "
        + "crates/krusty-server/src/process_wake.rs, "
        + "apps/mobile/components/chat/composerGrowth.ts. "
        + "Use list_dir/read_file/grep only. Return files_ok=true only if all exist; list any missing.",
    #{ label: "inventory", capability_mode: "read-only", output_schema: inventory_schema },
);
if inv == () || !inv.success || inv.output.files_ok != true {
    complete(#{
        summary: "Inventory failed — remediation files missing",
        inventory: inv,
    });
}

phase("Core tests");
let core = agent(
    "In /home/burgess/Work/krusty run: cargo test -p krusty-core progress shell_policy process -- --nocapture 2>&1 | tail -80. "
        + "Also run: cargo test -p krusty-server process_wake -- --nocapture 2>&1 | tail -40. "
        + "Report passed=true only if both succeed. Include command and a short summary of failures if any.",
    #{ label: "core-tests", capability_mode: "execute", output_schema: test_schema },
);

phase("Mobile");
let mobile = agent(
    "In /home/burgess/Work/krusty run: deno test --unstable-sloppy-imports apps/mobile/test/composerGrowth.test.ts 2>&1. "
        + "Report passed=true only on full green. Note app.json buildNumber should be 273.",
    #{ label: "mobile-tests", capability_mode: "execute", output_schema: test_schema },
);

phase("Review");
let review = agent(
    "Read docs/plans/2026-07-31-death-loop-remediation.md and skim the diffs in progress.rs, shell_policy.rs, process_wake.rs, registry.rs, composerGrowth.ts. "
        + "Adversarially check goals G1-G6: progress evidence fidelity, gh Observe, process wake, soft budget, soft-wrap, verification. "
        + "solid=true only if implementation matches plan with no critical gaps. List gaps and residual risks.",
    #{ label: "adversarial-review", capability_mode: "read-only", output_schema: review_schema },
);

let core_ok = core != () && core.success && core.output.passed == true;
let mobile_ok = mobile != () && mobile.success && mobile.output.passed == true;
let solid = review != () && review.success && review.output.solid == true;

let summary = "G1-G6 verification: core="
    + (if core_ok { "pass" } else { "fail" })
    + " mobile="
    + (if mobile_ok { "pass" } else { "fail" })
    + " review="
    + (if solid { "solid" } else { "gaps" });

complete(#{
    summary: summary,
    inventory: inv.output,
    core: if core != () && core.success { core.output } else { () },
    mobile: if mobile != () && mobile.success { mobile.output } else { () },
    review: if review != () && review.success { review.output } else { () },
    all_green: core_ok && mobile_ok && solid,
});
