/// View types
#[derive(Debug, Clone, PartialEq)]
pub enum View {
    StartMenu,
    Chat,
}

/// Popup types
#[derive(Debug, Clone, PartialEq)]
pub enum Popup {
    None,
    Auth,
    ModelSelect,
    ThemeSelect,
    Help,
    SessionList,
    McpBrowser,
    ProcessList,
    PluginsBrowser,
    FilePreview,
    SkillsBrowser,
    Hooks,
}

/// Work mode - BUILD (coding) or PLAN (planning)
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WorkMode {
    Build,
    Plan,
}

impl WorkMode {
    pub fn toggle(&self) -> Self {
        match self {
            WorkMode::Build => WorkMode::Plan,
            WorkMode::Plan => WorkMode::Build,
        }
    }
}

impl From<WorkMode> for mitsuro_core::storage::WorkMode {
    fn from(mode: WorkMode) -> Self {
        match mode {
            WorkMode::Build => mitsuro_core::storage::WorkMode::Build,
            WorkMode::Plan => mitsuro_core::storage::WorkMode::Plan,
        }
    }
}

impl From<mitsuro_core::storage::WorkMode> for WorkMode {
    fn from(mode: mitsuro_core::storage::WorkMode) -> Self {
        match mode {
            mitsuro_core::storage::WorkMode::Build => WorkMode::Build,
            mitsuro_core::storage::WorkMode::Plan => WorkMode::Plan,
        }
    }
}

/// Thinking intensity level for Tab-cycling in the TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingLevel {
    Off,
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
    Max,
    Ultra,
}

impl ThinkingLevel {
    pub fn is_enabled(self) -> bool {
        !matches!(self, Self::Off)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
            Self::Ultra => "ultra",
        }
    }

    pub fn from_reasoning_effort(effort: mitsuro_core::ai::providers::ReasoningEffort) -> Self {
        use mitsuro_core::ai::providers::ReasoningEffort;
        match effort {
            ReasoningEffort::None => Self::Off,
            ReasoningEffort::Minimal => Self::Minimal,
            ReasoningEffort::Low => Self::Low,
            ReasoningEffort::Medium => Self::Medium,
            ReasoningEffort::High => Self::High,
            ReasoningEffort::XHigh => Self::XHigh,
            ReasoningEffort::Max => Self::Max,
            ReasoningEffort::Ultra => Self::Ultra,
        }
    }
}
