//! Tool implementations for Krusty
//!
//! Provides the stable public tools facade: registry types, registration helpers,
//! and selected concrete tool implementations used by internal callers.

pub mod git_identity;
pub mod image;
mod implementations;
pub mod matching;
pub mod registry;
pub mod truncation;

pub use git_identity::{GitIdentity, GitIdentityMode};
pub use image::{
    is_image_extension, is_supported_file, load_from_clipboard_rgba, load_from_path, load_from_url,
};
pub use implementations::{
    register_acp_tools, register_agent_tool, register_all_tools, register_mako_tools,
    AddSubtaskTool, AgentTool, ApplyPatchTool, AskUserQuestionTool, AutonomousTaskTool, BashTool,
    EditTool, EnterPlanModeTool, GlobTool, GrepTool, ListTool, MemoryTool, MultiEditTool,
    ProcessesTool, ReadTool, ReportTool, SendUserMessageTool, SetDependencyTool, SetWorkModeTool,
    SetWorkspaceContextTool, SkillTool, SleepTool, TaskCompleteTool, TaskStartTool, ToolSearchTool,
    WebFetchTool, WebSearchTool, WriteTool,
};
pub use registry::{
    parse_params, FileObservationTracker, ToolContext, ToolOutputChunk, ToolRegistry, ToolResult,
};
