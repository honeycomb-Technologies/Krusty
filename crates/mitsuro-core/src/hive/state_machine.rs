use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HiveRunStatus {
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

impl HiveRunStatus {
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

impl std::fmt::Display for HiveRunStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
#[error("illegal Hive run transition from {from} to {to}")]
pub struct RunTransitionError {
    pub from: HiveRunStatus,
    pub to: HiveRunStatus,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_execution_path_is_legal() {
        HiveRunStatus::Queued
            .ensure_transition_to(HiveRunStatus::Leased)
            .unwrap();
        HiveRunStatus::Leased
            .ensure_transition_to(HiveRunStatus::Running)
            .unwrap();
        HiveRunStatus::Running
            .ensure_transition_to(HiveRunStatus::Succeeded)
            .unwrap();
    }

    #[test]
    fn uncertain_running_work_requires_recovery_state() {
        assert!(HiveRunStatus::Running
            .ensure_transition_to(HiveRunStatus::RecoveryRequired)
            .is_ok());
        assert!(HiveRunStatus::Succeeded
            .ensure_transition_to(HiveRunStatus::Running)
            .is_err());
    }

    #[test]
    fn only_manual_requeue_reopens_failed_or_dead_letter_runs() {
        assert!(HiveRunStatus::Failed
            .ensure_transition_to(HiveRunStatus::Queued)
            .is_ok());
        assert!(HiveRunStatus::DeadLetter
            .ensure_transition_to(HiveRunStatus::Queued)
            .is_ok());
        assert!(HiveRunStatus::Cancelled
            .ensure_transition_to(HiveRunStatus::Queued)
            .is_err());
    }
}
