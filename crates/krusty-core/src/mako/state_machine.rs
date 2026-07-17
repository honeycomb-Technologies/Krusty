use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MakoRunStatus {
    Queued,
    Leased,
    Running,
    Sleeping,
    RetryWait,
    AwaitingInput,
    RecoveryRequired,
    Succeeded,
    Failed,
    Cancelled,
    DeadLetter,
}

impl MakoRunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Leased => "leased",
            Self::Running => "running",
            Self::Sleeping => "sleeping",
            Self::RetryWait => "retry_wait",
            Self::AwaitingInput => "awaiting_input",
            Self::RecoveryRequired => "recovery_required",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::DeadLetter => "dead_letter",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "queued" => Some(Self::Queued),
            "leased" => Some(Self::Leased),
            "running" => Some(Self::Running),
            "sleeping" => Some(Self::Sleeping),
            "retry_wait" => Some(Self::RetryWait),
            "awaiting_input" => Some(Self::AwaitingInput),
            "recovery_required" => Some(Self::RecoveryRequired),
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            "dead_letter" => Some(Self::DeadLetter),
            _ => None,
        }
    }

    pub fn can_transition_to(self, next: Self) -> bool {
        if self == next {
            return true;
        }
        matches!(
            (self, next),
            (Self::Queued, Self::Leased | Self::Cancelled)
                | (
                    Self::Leased,
                    Self::Running | Self::Queued | Self::RecoveryRequired | Self::Cancelled
                )
                | (
                    Self::Running,
                    Self::Sleeping
                        | Self::RetryWait
                        | Self::AwaitingInput
                        | Self::RecoveryRequired
                        | Self::Succeeded
                        | Self::Failed
                        | Self::Cancelled
                        | Self::DeadLetter
                )
                | (Self::Sleeping, Self::Queued | Self::Cancelled)
                | (
                    Self::RetryWait,
                    Self::Queued | Self::DeadLetter | Self::Cancelled
                )
                | (Self::AwaitingInput, Self::Queued | Self::Cancelled)
                | (
                    Self::RecoveryRequired,
                    Self::Queued | Self::Failed | Self::Cancelled
                )
                | (Self::Failed, Self::Queued)
                | (Self::DeadLetter, Self::Queued)
        )
    }

    pub fn ensure_transition_to(self, next: Self) -> Result<(), RunTransitionError> {
        self.can_transition_to(next)
            .then_some(())
            .ok_or(RunTransitionError {
                from: self,
                to: next,
            })
    }

    pub fn holds_worker_lease(self) -> bool {
        matches!(self, Self::Leased | Self::Running)
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::DeadLetter
        )
    }
}

impl std::fmt::Display for MakoRunStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
#[error("illegal Mako run transition from {from} to {to}")]
pub struct RunTransitionError {
    pub from: MakoRunStatus,
    pub to: MakoRunStatus,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_execution_path_is_legal() {
        MakoRunStatus::Queued
            .ensure_transition_to(MakoRunStatus::Leased)
            .unwrap();
        MakoRunStatus::Leased
            .ensure_transition_to(MakoRunStatus::Running)
            .unwrap();
        MakoRunStatus::Running
            .ensure_transition_to(MakoRunStatus::Succeeded)
            .unwrap();
    }

    #[test]
    fn uncertain_running_work_requires_recovery_state() {
        assert!(MakoRunStatus::Running
            .ensure_transition_to(MakoRunStatus::RecoveryRequired)
            .is_ok());
        assert!(MakoRunStatus::Succeeded
            .ensure_transition_to(MakoRunStatus::Running)
            .is_err());
    }

    #[test]
    fn only_manual_requeue_reopens_failed_or_dead_letter_runs() {
        assert!(MakoRunStatus::Failed
            .ensure_transition_to(MakoRunStatus::Queued)
            .is_ok());
        assert!(MakoRunStatus::DeadLetter
            .ensure_transition_to(MakoRunStatus::Queued)
            .is_ok());
        assert!(MakoRunStatus::Cancelled
            .ensure_transition_to(MakoRunStatus::Queued)
            .is_err());
    }
}
