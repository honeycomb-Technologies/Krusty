//! Observation system for context synchronization
//!
//! Little Claw doesn't execute tools, but needs to know what Big Claw did.
//! Observations are summaries of Big Claw's actions injected into Little Claw's context.

use std::fmt;

/// An observation of Big Claw's action for Little Claw's context
#[derive(Debug, Clone)]
pub struct Observation {
    /// What tool/action was used
    pub action: ObservedAction,

    /// Brief description of what happened
    pub summary: String,

    /// The actual content (code, output, etc.)
    pub content: Option<String>,

    /// Was it successful?
    pub success: bool,
}

impl Observation {
    /// Create a file edit observation
    pub fn file_edit(path: &str, summary: &str, content: &str) -> Self {
        Self {
            action: ObservedAction::FileEdit {
                path: path.to_string(),
            },
            summary: summary.to_string(),
            content: Some(content.to_string()),
            success: true,
        }
    }

    /// Create a file write observation
    pub fn file_write(path: &str, summary: &str) -> Self {
        Self {
            action: ObservedAction::FileWrite {
                path: path.to_string(),
            },
            summary: summary.to_string(),
            content: None,
            success: true,
        }
    }

    /// Create a bash command observation
    pub fn bash(command: &str, output: &str, success: bool) -> Self {
        Self {
            action: ObservedAction::Bash {
                command: command.to_string(),
            },
            summary: if success {
                "Command executed successfully".to_string()
            } else {
                "Command failed".to_string()
            },
            content: Some(output.to_string()),
            success,
        }
    }

    /// Create a tool result observation
    pub fn tool_result(tool_name: &str, summary: &str, success: bool) -> Self {
        Self {
            action: ObservedAction::Other {
                tool: tool_name.to_string(),
            },
            summary: summary.to_string(),
            content: None,
            success,
        }
    }

    /// Format as a message for Little Claw's context
    pub fn as_context_message(&self) -> String {
        let mut msg = format!("[Observation] {}: {}", self.action, self.summary);

        if let Some(content) = &self.content {
            // Truncate long content
            let truncated = if content.len() > 2000 {
                format!("{}...[truncated]", &content[..2000])
            } else {
                content.clone()
            };
            msg.push_str(&format!("\n```\n{}\n```", truncated));
        }

        if !self.success {
            msg.push_str("\n[Status: FAILED]");
        }

        msg
    }
}

/// Types of actions that can be observed
#[derive(Debug, Clone)]
pub enum ObservedAction {
    FileEdit { path: String },
    FileWrite { path: String },
    FileRead { path: String },
    Bash { command: String },
    Other { tool: String },
}

impl fmt::Display for ObservedAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ObservedAction::FileEdit { path } => write!(f, "Edit {}", path),
            ObservedAction::FileWrite { path } => write!(f, "Write {}", path),
            ObservedAction::FileRead { path } => write!(f, "Read {}", path),
            ObservedAction::Bash { command } => {
                // Truncate long commands
                let cmd = if command.len() > 50 {
                    format!("{}...", &command[..50])
                } else {
                    command.clone()
                };
                write!(f, "Bash: {}", cmd)
            }
            ObservedAction::Other { tool } => write!(f, "{}", tool),
        }
    }
}

/// Track what Big Claw has done in a session
#[derive(Debug, Default)]
pub struct ObservationLog {
    observations: Vec<Observation>,
}

impl ObservationLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an observation
    pub fn record(&mut self, observation: Observation) {
        self.observations.push(observation);
    }

    /// Get recent observations (last N)
    #[allow(dead_code)]
    pub fn recent(&self, n: usize) -> &[Observation] {
        let start = self.observations.len().saturating_sub(n);
        &self.observations[start..]
    }

    /// Get all observations
    #[allow(dead_code)]
    pub fn all(&self) -> &[Observation] {
        &self.observations
    }

    /// Get observations since a certain index
    pub fn since(&self, index: usize) -> &[Observation] {
        if index < self.observations.len() {
            &self.observations[index..]
        } else {
            &[]
        }
    }

    /// Current count
    pub fn len(&self) -> usize {
        self.observations.len()
    }

    /// Check if empty
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.observations.is_empty()
    }

    /// Format recent observations as context
    #[allow(dead_code)]
    pub fn as_context(&self, n: usize) -> String {
        self.recent(n)
            .iter()
            .map(|o| o.as_context_message())
            .collect::<Vec<_>>()
            .join("\n\n")
    }
}
