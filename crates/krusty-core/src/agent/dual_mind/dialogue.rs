//! Dialogue management between Big Claw and Little Claw
//!
//! Handles the communication protocol, buffering, and consensus detection.

use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::timeout;

use super::ClawRole;

/// A single turn in the dialogue
#[derive(Debug, Clone)]
pub struct DialogueTurn {
    pub speaker: Speaker,
    pub content: String,
}

/// Who is speaking
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speaker {
    BigClaw,
    LittleClaw,
}

impl Speaker {
    pub fn display_name(&self) -> &'static str {
        match self {
            Speaker::BigClaw => "Big Claw",
            Speaker::LittleClaw => "Little Claw",
        }
    }

    pub fn from_role(role: ClawRole) -> Self {
        match role {
            ClawRole::BigClaw => Speaker::BigClaw,
            ClawRole::LittleClaw => Speaker::LittleClaw,
        }
    }
}

/// Result of a dialogue exchange
#[derive(Debug)]
pub enum DialogueResult {
    /// Both agreed, proceed with action
    Consensus { dialogue: Vec<DialogueTurn> },

    /// Little Claw raised concerns that were addressed
    Refined { dialogue: Vec<DialogueTurn> },

    /// Little Claw found issues post-action, needs enhancement
    NeedsEnhancement {
        dialogue: Vec<DialogueTurn>,
        critique: String,
    },

    /// Couldn't agree, Big Claw proceeds with noted concern
    BigClawDecides {
        dialogue: Vec<DialogueTurn>,
        concern: String,
    },

    /// Dual-mind disabled or trivial action
    Skipped,
}

impl DialogueResult {
    /// Check if this result requires action refinement
    pub fn needs_enhancement(&self) -> bool {
        matches!(self, DialogueResult::NeedsEnhancement { .. })
    }

    /// Get the dialogue transcript
    pub fn dialogue(&self) -> &[DialogueTurn] {
        match self {
            DialogueResult::Consensus { dialogue }
            | DialogueResult::Refined { dialogue }
            | DialogueResult::NeedsEnhancement { dialogue, .. }
            | DialogueResult::BigClawDecides { dialogue, .. } => dialogue,
            DialogueResult::Skipped => &[],
        }
    }

    /// Format dialogue for thinking block display
    pub fn format_for_thinking(&self) -> String {
        let mut output = String::new();

        for turn in self.dialogue() {
            output.push_str(&format!(
                "[{}] {}\n\n",
                turn.speaker.display_name(),
                turn.content
            ));
        }

        output
    }
}

/// Manages the dialogue between claws
pub struct DialogueManager {
    /// Incoming messages from Little Claw
    rx: mpsc::UnboundedReceiver<DialogueTurn>,

    /// Buffered dialogue for rendering
    buffer: Vec<DialogueTurn>,

    /// Maximum back-and-forth before forcing decision
    max_exchanges: usize,

    /// Timeout for waiting on responses
    response_timeout: Duration,
}

impl DialogueManager {
    pub fn new(rx: mpsc::UnboundedReceiver<DialogueTurn>) -> Self {
        Self {
            rx,
            buffer: Vec::new(),
            max_exchanges: 5,
            response_timeout: Duration::from_secs(30),
        }
    }

    /// Add a turn to the dialogue buffer
    pub fn add_turn(&mut self, speaker: Speaker, content: String) {
        self.buffer.push(DialogueTurn { speaker, content });
    }

    /// Wait for Little Claw's response with timeout
    pub async fn await_little_claw(&mut self) -> Option<DialogueTurn> {
        match timeout(self.response_timeout, self.rx.recv()).await {
            Ok(Some(turn)) => {
                self.buffer.push(turn.clone());
                Some(turn)
            }
            Ok(None) => None, // Channel closed
            Err(_) => None,   // Timeout
        }
    }

    /// Drain and return all buffered dialogue
    pub async fn drain(&mut self) -> Vec<DialogueTurn> {
        std::mem::take(&mut self.buffer)
    }

    /// Check if we've exceeded max exchanges
    pub fn is_at_limit(&self) -> bool {
        self.buffer.len() >= self.max_exchanges * 2
    }

    /// Clear the buffer
    pub fn clear(&mut self) {
        self.buffer.clear();
    }
}

/// Keywords that indicate agreement
#[allow(dead_code)]
const AGREEMENT_SIGNALS: &[&str] = &[
    "proceed",
    "agreed",
    "looks good",
    "no issues",
    "correct",
    "approve",
    "continue",
];

/// Keywords that indicate concern
#[allow(dead_code)]
const CONCERN_SIGNALS: &[&str] = &[
    "wait",
    "hold on",
    "concern",
    "issue",
    "problem",
    "question",
    "why",
    "should we",
    "consider",
    "instead",
];

/// Analyze Little Claw's response to determine intent
#[allow(dead_code)]
pub fn analyze_response(response: &str) -> ResponseIntent {
    let lower = response.to_lowercase();

    // Check for explicit agreement
    for signal in AGREEMENT_SIGNALS {
        if lower.contains(signal) {
            return ResponseIntent::Agree;
        }
    }

    // Check for concerns
    for signal in CONCERN_SIGNALS {
        if lower.contains(signal) {
            return ResponseIntent::Question;
        }
    }

    // Default: needs more context
    ResponseIntent::Unclear
}

/// What Little Claw intends with their response
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseIntent {
    /// Agrees with the approach
    Agree,
    /// Has a question or concern
    Question,
    /// Unclear, needs clarification
    Unclear,
}
