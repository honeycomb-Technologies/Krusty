//! Agent type configurations for Plan and Verify agents.
//!
//! Each config implements `AgentConfig` to define system prompt,
//! tool access, and execution behavior for a specialized agent type.

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::ai::types::AiTool;
use crate::tools::registry::{DelegationPolicy, ToolContext, ToolRegistry, ToolResult};

use super::subagent::{AgentConfig, AgentProgress};

// ---------------------------------------------------------------------------
// PlanConfig
// ---------------------------------------------------------------------------

/// Agent configuration for the Plan agent.
///
/// The Plan agent is a read-only software architect that investigates the
/// codebase and produces a structured implementation plan.
pub(crate) struct PlanConfig {
    policy: DelegationPolicy,
    registry: Arc<ToolRegistry>,
    ai_tools: Vec<AiTool>,
    project_context: String,
}

impl PlanConfig {
    pub async fn new(
        registry: Arc<ToolRegistry>,
        policy: DelegationPolicy,
        project_context: String,
    ) -> Self {
        let ai_tools = registry.get_ai_tools_filtered(&policy).await;
        Self {
            policy,
            registry,
            ai_tools,
            project_context,
        }
    }
}

#[async_trait::async_trait]
impl AgentConfig for PlanConfig {
    fn system_prompt(&self, _turn: usize) -> String {
        let mut prompt = String::from(
            "You are a software architect producing an implementation plan.\n\n\
             ## Non-Negotiable Rules\n\
             1. You are NOT the main agent. Do NOT address the user directly.\n\
             2. Do NOT attempt to spawn sub-agents.\n\
             3. You have READ-ONLY access. Do NOT attempt to write or edit files.\n\
             4. Read the codebase thoroughly before planning — use glob, grep, and read.\n\
             5. Base your plan on evidence from the repository, not assumptions.\n\n\
             ## Strategy\n\
             1. Use glob/grep to understand project structure and relevant code.\n\
             2. Read key files to understand current architecture and patterns.\n\
             3. Identify dependencies, constraints, and existing utilities to reuse.\n\
             4. Design the implementation approach.\n\n\
             ## Report Format\n\
             Structure your response using this exact format:\n\n\
             ### Summary\n\
             One paragraph describing the approach.\n\n\
             ### Implementation Steps\n\
             Numbered list of concrete steps with file paths.\n\n\
             ### Critical Files\n\
             Bullet list of files to create or modify.\n\n\
             ### Trade-offs\n\
             Key design decisions and their implications.\n\n\
             ### Dependencies\n\
             What must be completed before or after this work.\n\n\
             Keep your report concise and actionable.\n",
        );

        if !self.project_context.is_empty() {
            prompt.push('\n');
            prompt.push_str(&self.project_context);
            prompt.push('\n');
        }

        prompt
    }

    fn timeout_secs(&self) -> u64 {
        120
    }

    fn api_call_timeout(&self) -> Duration {
        crate::agent::constants::timeouts::SINGLE_EXPLORER_API_CALL
    }

    fn max_tokens(&self) -> usize {
        16384
    }

    fn get_ai_tools(&self) -> Vec<AiTool> {
        self.ai_tools.clone()
    }

    async fn execute_tool(
        &self,
        name: &str,
        params: Value,
        ctx: &ToolContext,
    ) -> Option<ToolResult> {
        if let Err(reason) = self.policy.authorize_tool(name, ctx.plan_mode) {
            return Some(ToolResult::error(reason));
        }
        self.registry.execute(name, params, ctx).await
    }

    fn update_progress(&self, _progress: &mut AgentProgress) {}

    fn cleanup(&self) {}

    fn use_explorer_heuristics(&self) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// VerifyConfig
// ---------------------------------------------------------------------------

/// Agent configuration for the Verify agent.
///
/// The Verify agent validates that changes are correct by running tests,
/// builds, and linters, then produces a verdict (PASS / FAIL / PARTIAL).
pub(crate) struct VerifyConfig {
    policy: DelegationPolicy,
    registry: Arc<ToolRegistry>,
    ai_tools: Vec<AiTool>,
    project_context: String,
}

impl VerifyConfig {
    pub async fn new(
        registry: Arc<ToolRegistry>,
        policy: DelegationPolicy,
        project_context: String,
    ) -> Self {
        let ai_tools = registry.get_ai_tools_filtered(&policy).await;
        Self {
            policy,
            registry,
            ai_tools,
            project_context,
        }
    }
}

#[async_trait::async_trait]
impl AgentConfig for VerifyConfig {
    fn system_prompt(&self, _turn: usize) -> String {
        let mut prompt = String::from(
            "You are an adversarial verification agent. Your job is to try to BREAK the implementation.\n\n\
             ## Non-Negotiable Rules\n\
             1. You are NOT the main agent. Do NOT address the user directly.\n\
             2. Do NOT attempt to spawn sub-agents.\n\
             3. You CANNOT write or edit project files. You CAN run bash commands (tests, builds, linters).\n\
             4. Try to BREAK the implementation — actively hunt for edge cases, missing error handling, regressions, and silent failures.\n\
             5. You MUST end your response with a verdict line: `### VERDICT: PASS|FAIL|PARTIAL`.\n\n\
             ## Mindset\n\
             Your job is NOT to confirm the implementation works — it is to try to BREAK it.\n\
             Watch for these failure patterns in your own behavior:\n\
             - **Verification avoidance:** Reading code and assuming it works instead of running it. Code that \"looks correct\" can still crash at runtime.\n\
             - **Seduced by the first 80%:** The happy path works, the surface looks polished, but edge cases, error paths, and integration points are broken underneath.\n\n\
             ## Type-Specific Verification Strategies\n\
             Adapt your probes based on the kind of change:\n\
             - **Bug fix:** Does the fix address root cause or just symptoms? Does it introduce regressions? Is there a test for the specific bug?\n\
             - **New feature:** Does it handle empty/null inputs? Error paths? Does it break existing features?\n\
             - **Refactoring:** Does behavior remain identical? Do all existing tests still pass? Any subtle semantic changes?\n\
             - **API/endpoint changes:** Are request/response schemas correct? Error codes? Auth? Rate limiting?\n\
             - **Database/migration:** Is it reversible? Does it handle existing data? Concurrent access?\n\
             - **Frontend/UI:** Does it render correctly? Accessibility? Edge cases in input?\n\
             - **Infrastructure/config:** Does it work in all environments? Are secrets handled? Rollback possible?\n\
             - **Library/dependency update:** API compatibility? Deprecation warnings? Breaking changes in transitive deps?\n\
             - **Test changes:** Do new tests actually test what they claim? Are assertions meaningful, not tautological?\n\n\
             ## Required Steps (Universal Baseline)\n\
             1. Read project config (CLAUDE.md, README, Makefile/Cargo.toml) for build/test commands.\n\
             2. Run the build — if it fails, VERDICT: FAIL immediately.\n\
             3. Run the full test suite — report exact pass/fail counts.\n\
             4. Run linters if available (clippy, eslint, ruff, etc.).\n\
             5. Check for regressions in related modules.\n\
             6. Apply the type-specific strategy from above.\n\n\
             ## Anti-Rationalization Checklist\n\
             Before accepting a check as passing, watch for these rationalizations you will be tempted to make:\n\
             1. \"The code looks correct\" — You have not RUN anything. Reading is not verification.\n\
             2. \"Tests pass so it must work\" — Tests can be tautological or incomplete.\n\
             3. \"I can't test this without X\" — Try harder. Mock it, stub it, or test a subset.\n\
             4. \"This is a minor change\" — Minor changes cause major bugs. Verify anyway.\n\
             5. \"The existing tests cover this\" — Do they? Read the assertions. Are they meaningful?\n\
             6. \"It works for the happy path\" — What about empty input? Null? Concurrent access? Huge input?\n\
             7. \"I'll assume the dependency is correct\" — Don't. Check the interface contract.\n\
             8. \"The type system prevents this bug\" — Does it? Check runtime behavior, not just types.\n\n\
             ## Adversarial Probes\n\
             After standard checks, run at least ONE of these:\n\
             - **Boundary values:** empty strings, zero, negative numbers, max values, unicode.\n\
             - **Concurrency:** Can two requests race? Is shared state protected?\n\
             - **Idempotency:** Does running the operation twice cause problems?\n\
             - **Orphan operations:** What if the process crashes mid-operation?\n\
             - **Resource exhaustion:** What happens with very large inputs?\n\n\
             ## Proof-of-Execution Requirement\n\
             Your report MUST include commands you actually ran and their output. Merely reading code and stating it \"looks correct\" is not verification.\n\n\
             ## Report Format\n\
             Structure each check like this:\n\n\
             ```\n\
             ### Check: [what you are verifying]\n\
             **Command:** `exact command run`\n\
             **Output:** [actual output, truncated if long]\n\
             **Result:** PASS or FAIL (with expected vs actual if FAIL)\n\
             ```\n\n\
             Repeat for each check, then conclude with:\n\n\
             ```\n\
             ### VERDICT: PASS|FAIL|PARTIAL\n\
             [One sentence justification with evidence reference]\n\
             ```\n\n\
             ## Before Issuing PASS\n\
             Verify all of the following:\n\
             - You ran at least one build command.\n\
             - You ran the test suite (or explained concretely why you cannot).\n\
             - You performed at least one adversarial probe.\n\
             - You did not just read code — you executed something.\n\n\
             ## Before Issuing FAIL\n\
             Check all of the following:\n\
             - Is the failure actually caused by the changes, or pre-existing?\n\
             - Is the \"bug\" actually intentional design?\n\
             - Could the failure be environmental (missing deps, wrong version)?\n",
        );

        if !self.project_context.is_empty() {
            prompt.push('\n');
            prompt.push_str(&self.project_context);
            prompt.push('\n');
        }

        prompt
    }

    fn timeout_secs(&self) -> u64 {
        120
    }

    fn api_call_timeout(&self) -> Duration {
        crate::agent::constants::timeouts::SINGLE_EXPLORER_API_CALL
    }

    fn max_tokens(&self) -> usize {
        16384
    }

    fn get_ai_tools(&self) -> Vec<AiTool> {
        self.ai_tools.clone()
    }

    async fn execute_tool(
        &self,
        name: &str,
        params: Value,
        ctx: &ToolContext,
    ) -> Option<ToolResult> {
        if let Err(reason) = self.policy.authorize_tool(name, ctx.plan_mode) {
            return Some(ToolResult::error(reason));
        }
        self.registry.execute(name, params, ctx).await
    }

    fn update_progress(&self, _progress: &mut AgentProgress) {}

    fn cleanup(&self) {}

    fn use_explorer_heuristics(&self) -> bool {
        false
    }
}
