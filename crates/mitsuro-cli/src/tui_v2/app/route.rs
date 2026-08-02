//! Primary TUI routes.

/// Canonical session identity as consumed by the TUI.
///
/// The wrapper prevents route code from accidentally confusing a session ID
/// with a title, project path, or provider identifier.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SessionId(String);

impl SessionId {
    pub fn from_canonical(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The only three top-level destinations in Mitsuro TUI v2.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum AppRoute {
    Setup,
    #[default]
    Home,
    Conversation {
        session_id: SessionId,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_identity_is_not_a_bare_string_at_the_route_boundary() {
        let session_id = SessionId::from_canonical("session-42");
        let route = AppRoute::Conversation {
            session_id: session_id.clone(),
        };

        assert_eq!(session_id.as_str(), "session-42");
        assert_eq!(
            route,
            AppRoute::Conversation {
                session_id: SessionId::from_canonical("session-42")
            }
        );
    }
}
