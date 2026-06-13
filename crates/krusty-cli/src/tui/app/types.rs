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

impl From<WorkMode> for krusty_core::storage::WorkMode {
    fn from(mode: WorkMode) -> Self {
        match mode {
            WorkMode::Build => krusty_core::storage::WorkMode::Build,
            WorkMode::Plan => krusty_core::storage::WorkMode::Plan,
        }
    }
}

impl From<krusty_core::storage::WorkMode> for WorkMode {
    fn from(mode: krusty_core::storage::WorkMode) -> Self {
        match mode {
            krusty_core::storage::WorkMode::Build => WorkMode::Build,
            krusty_core::storage::WorkMode::Plan => WorkMode::Plan,
        }
    }
}

/// Thinking intensity level for Tab-cycling in the TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingLevel {
    Off,
    Low,
    Medium,
    High,
    XHigh,
}

impl ThinkingLevel {
    pub fn is_enabled(self) -> bool {
        !matches!(self, Self::Off)
    }

    pub fn cycle_codex(self) -> Self {
        match self {
            Self::Off => Self::Low,
            Self::Low => Self::Medium,
            Self::Medium => Self::High,
            Self::High => Self::XHigh,
            Self::XHigh => Self::Off,
        }
    }

    /// Cycle for Anthropic Opus: Off -> Low -> Medium -> High -> XHigh -> Off
    pub fn cycle_anthropic(self) -> Self {
        match self {
            Self::Off => Self::Low,
            Self::Low => Self::Medium,
            Self::Medium => Self::High,
            Self::High => Self::XHigh,
            Self::XHigh => Self::Off,
        }
    }

    pub fn toggle_basic(self) -> Self {
        if self.is_enabled() {
            Self::Off
        } else {
            Self::Medium
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
        }
    }
}
