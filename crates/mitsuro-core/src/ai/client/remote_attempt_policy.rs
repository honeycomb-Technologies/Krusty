//! Remote-attempt policy for provider calls.
//!
//! A governed Hive Worker call has a durable provider-call slot before it may
//! cross the network. One slot must never hide transport retries because an
//! ambiguous timeout or disconnect can happen after the provider accepted and
//! billed the request.

/// Controls whether one logical client call may issue another remote request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteAttemptPolicy {
    /// Preserve the bounded retry behavior configured for ordinary Chat/Code
    /// and ungoverned delegated calls.
    ConfiguredRetries,
    /// Issue at most one inference request for an already-started durable Hive
    /// Worker provider-call slot. Ambiguous failures remain unresolved and are
    /// never resent inside that slot.
    GovernedSingleAttempt,
}

impl RemoteAttemptPolicy {
    pub(crate) const fn allows_retry(self) -> bool {
        matches!(self, Self::ConfiguredRetries)
    }
}

#[cfg(test)]
mod tests {
    use super::RemoteAttemptPolicy;

    #[test]
    fn only_configured_policy_allows_a_second_remote_attempt() {
        assert!(RemoteAttemptPolicy::ConfiguredRetries.allows_retry());
        assert!(!RemoteAttemptPolicy::GovernedSingleAttempt.allows_retry());
    }
}
