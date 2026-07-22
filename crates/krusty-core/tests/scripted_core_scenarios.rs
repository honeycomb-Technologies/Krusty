mod support;

use krusty_core::agent::{ActionClass, ProgressLedger};
use krusty_core::ai::types::{AiToolCall, Content};
use support::scripted_scenario::{
    run_scenario, surface_parity_violations, validate_replay_trace, EffectClass, ExpectedMetrics,
    ScenarioFixtureFile, ScenarioPolicy, ScenarioStopReason, ScriptedModelStep, ScriptedScenario,
    ScriptedToolCall, SurfaceParityFixtureFile, VirtualEffect,
};

fn production_call(id: &str, command: &str) -> AiToolCall {
    AiToolCall {
        id: id.to_string(),
        name: "bash".to_string(),
        arguments: serde_json::json!({"command": command}),
    }
}

fn production_result(id: &str, output: &str) -> Content {
    Content::ToolResult {
        tool_use_id: id.to_string(),
        output: serde_json::Value::String(output.to_string()),
        is_error: Some(false),
    }
}

fn production_unchanged_result(id: &str) -> Content {
    Content::ToolResult {
        tool_use_id: id.to_string(),
        output: serde_json::json!({
            "ok": true,
            "data": {"output": "already up to date"},
            "metadata": {"changed": false}
        }),
        is_error: Some(false),
    }
}

fn scenario_files() -> [(&'static str, &'static str); 3] {
    [
        (
            "workflows",
            include_str!("fixtures/scripted_scenarios/workflows.json"),
        ),
        (
            "grok loops",
            include_str!("fixtures/scripted_scenarios/grok_loops.json"),
        ),
        (
            "provider faults",
            include_str!("fixtures/scripted_scenarios/provider_faults.json"),
        ),
    ]
}

#[test]
fn scripted_scenarios_meet_acceptance_metrics() {
    let mut scenarios_run = 0;
    for (fixture_name, contents) in scenario_files() {
        let fixtures: ScenarioFixtureFile = serde_json::from_str(contents)
            .unwrap_or_else(|error| panic!("invalid {fixture_name} fixture: {error}"));
        for scenario in fixtures.scenarios {
            let report = run_scenario(&scenario)
                .unwrap_or_else(|error| panic!("scenario {} failed: {error}", scenario.name));
            let violations = report.expectation_violations(&scenario.expect);
            assert!(
                violations.is_empty(),
                "scenario {} violated its metrics:\n{}",
                scenario.name,
                violations.join("\n")
            );
            validate_replay_trace(&report).unwrap_or_else(|violations| {
                panic!(
                    "scenario {} emitted an invalid replay trace:\n{}",
                    scenario.name,
                    violations.join("\n")
                )
            });
            scenarios_run += 1;
        }
    }

    assert_eq!(
        scenarios_run, 11,
        "the core scenario corpus changed unexpectedly"
    );
}

#[test]
fn resolved_run_contract_is_identical_across_surfaces() {
    let fixtures: SurfaceParityFixtureFile = serde_json::from_str(include_str!(
        "fixtures/scripted_scenarios/surface_parity.json"
    ))
    .expect("surface parity fixture must be valid");

    for fixture in fixtures.fixtures {
        let violations = surface_parity_violations(&fixture);
        assert!(
            violations.is_empty(),
            "surface fixture {} diverged:\n{}",
            fixture.name,
            violations.join("\n")
        );
    }
}

#[test]
fn progressive_workflow_can_exceed_fifty_provider_turns() {
    let mut steps = (0..60)
        .map(|index| ScriptedModelStep::ToolBatch {
            calls: vec![ScriptedToolCall {
                id: format!("long-audit-{index}"),
                tool: "read".to_string(),
                arguments: serde_json::json!({"file_path": format!("src/file_{index}.rs")}),
                intent: format!("audit:src/file_{index}.rs"),
                effect: VirtualEffect {
                    class: EffectClass::Observe,
                    ok: true,
                    evidence: Some(format!("src/file_{index}.rs:evidence")),
                    resource: Some(format!("src/file_{index}.rs")),
                    changed: false,
                    side_effect_key: None,
                    result: None,
                },
            }],
        })
        .collect::<Vec<_>>();
    steps.push(ScriptedModelStep::Complete {
        text: "Long audit completed after continuing to discover evidence.".to_string(),
    });

    let scenario = ScriptedScenario {
        name: "progressive_workflow_over_fifty_turns".to_string(),
        policy: ScenarioPolicy::default(),
        steps,
        expect: ExpectedMetrics::default(),
    };
    let report = run_scenario(&scenario).expect("progressive scenario must complete");

    assert_eq!(report.stop_reason, ScenarioStopReason::Completed);
    assert_eq!(report.metrics.provider_calls, 61);
    assert_eq!(report.metrics.progress_events, 60);
    assert_eq!(report.metrics.no_progress_cycles, 0);
    assert_eq!(report.metrics.side_effects, 0);
    validate_replay_trace(&report).expect("long-run replay trace must remain valid");
}

#[test]
fn production_progress_guard_converges_cosmetic_grok_bash_variants() {
    let mut ledger = ProgressLedger::new();
    let commands = [
        "rg -n TODO src | head -20",
        "pwd && rg --line-number TODO src | head -50",
        "rg -n TODO src | head -100",
        "rg --line-number TODO src",
    ];

    for (index, command) in commands.iter().enumerate() {
        let id = format!("production-grok-{index}");
        let call = production_call(&id, command);
        let fingerprint = krusty_core::agent::progress::action_fingerprint(&call);
        assert_eq!(fingerprint.class, ActionClass::Observe);
        let telemetry = ledger.record_turn(&[call], &[production_result(&id, "same evidence")]);
        match index {
            0 => assert!(telemetry.is_none(), "first evidence should be accepted"),
            1 | 2 => assert!(!telemetry.expect("repeat telemetry").triggered),
            3 => {
                let telemetry = telemetry.expect("terminal guard telemetry");
                assert!(telemetry.triggered);
                assert_eq!(telemetry.no_progress_turns, 3);
            }
            _ => unreachable!(),
        }
    }
}

#[test]
fn production_progress_guard_allows_sixty_distinct_evidence_turns() {
    let mut ledger = ProgressLedger::new();
    for index in 0..60 {
        let id = format!("production-long-audit-{index}");
        let call = AiToolCall {
            id: id.clone(),
            name: "read".to_string(),
            arguments: serde_json::json!({"file_path": format!("src/file_{index}.rs")}),
        };
        let telemetry = ledger.record_turn(
            &[call],
            &[production_result(
                &id,
                &format!("distinct evidence {index}"),
            )],
        );
        assert!(
            telemetry.is_none(),
            "new evidence at turn {} must not trip the progress guard",
            index + 1
        );
    }
}

#[test]
fn production_progress_guard_converges_successful_idempotent_mutations() {
    let mut ledger = ProgressLedger::new();
    let commands = [
        "touch already-present.txt",
        "pwd && touch already-present.txt",
        "true && touch already-present.txt",
    ];

    for (index, command) in commands.iter().enumerate() {
        let id = format!("production-noop-{index}");
        let call = production_call(&id, command);
        let fingerprint = krusty_core::agent::progress::action_fingerprint(&call);
        assert_eq!(fingerprint.class, ActionClass::Mutate);
        let telemetry = ledger.record_turn(&[call], &[production_unchanged_result(&id)]);
        match index {
            0 | 1 => assert!(!telemetry.expect("no-op telemetry").triggered),
            2 => {
                let telemetry = telemetry.expect("terminal guard telemetry");
                assert!(telemetry.triggered);
                assert_eq!(telemetry.no_progress_turns, 3);
            }
            _ => unreachable!(),
        }
    }
}
