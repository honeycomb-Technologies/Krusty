#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    pub fn api_value(self) -> Option<&'static str> {
        match self {
            Self::Off => None,
            Self::Minimal => Some("minimal"),
            Self::Low => Some("low"),
            Self::Medium => Some("medium"),
            Self::High => Some("high"),
            Self::XHigh => Some("xhigh"),
            Self::Max => Some("max"),
            Self::Ultra => Some("ultra"),
        }
    }

    pub fn cycle(self) -> Self {
        match self {
            Self::Off => Self::Minimal,
            Self::Minimal => Self::Low,
            Self::Low => Self::Medium,
            Self::Medium => Self::High,
            Self::High => Self::XHigh,
            Self::XHigh => Self::Max,
            Self::Max => Self::Ultra,
            Self::Ultra => Self::Off,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Minimal => "min",
            Self::Low => "low",
            Self::Medium => "med",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
            Self::Ultra => "ultra",
        }
    }

    pub fn from_api_value(value: &str) -> Option<Self> {
        match value {
            "none" | "off" => Some(Self::Off),
            "minimal" => Some(Self::Minimal),
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "xhigh" => Some(Self::XHigh),
            "max" => Some(Self::Max),
            "ultra" => Some(Self::Ultra),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionMode {
    Supervised,
    Autonomous,
}

impl PermissionMode {
    pub fn api_value(self) -> &'static str {
        match self {
            Self::Supervised => "supervised",
            Self::Autonomous => "autonomous",
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            Self::Supervised => Self::Autonomous,
            Self::Autonomous => Self::Supervised,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Supervised => "supervised",
            Self::Autonomous => "auto",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkMode {
    Build,
    Plan,
}

impl WorkMode {
    pub fn api_value(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Plan => "plan",
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            Self::Build => Self::Plan,
            Self::Plan => Self::Build,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Plan => "plan",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ChatSessionState {
    pub session_id: Option<String>,
    pub project_dir: Option<String>,
    pub model: Option<String>,
    pub thinking_level: ThinkingLevel,
    pub permission_mode: PermissionMode,
    pub fast_mode: bool,
    pub work_mode: WorkMode,
    pub is_streaming: bool,
}

impl ChatSessionState {
    pub fn new() -> Self {
        Self {
            thinking_level: ThinkingLevel::Medium,
            permission_mode: PermissionMode::Autonomous,
            fast_mode: false,
            work_mode: WorkMode::Build,
            session_id: None,
            project_dir: None,
            model: None,
            is_streaming: false,
        }
    }
}
