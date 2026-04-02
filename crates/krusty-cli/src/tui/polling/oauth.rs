//! OAuth status channel polling
//!
//! Handles status updates from background OAuth authentication tasks.

use std::time::Duration;

use krusty_core::ai::providers::ProviderId;
use krusty_core::auth::{OAuthTokenData, OAuthTokenStore};

use crate::tui::popups::auth::AuthPopup;
use crate::tui::utils::AsyncChannels;

use super::{PollAction, PollResult};

const OAUTH_BROWSER_TIMEOUT: Duration = Duration::from_secs(300);

/// Poll OAuth status updates from background authentication tasks
///
/// Returns actions for App to execute (SwitchProvider) to avoid borrow conflicts.
pub fn poll_oauth_status(
    channels: &mut AsyncChannels,
    auth_popup: &mut AuthPopup,
    active_provider: ProviderId,
) -> PollResult {
    let mut result = PollResult::new();

    let Some(mut rx) = channels.oauth_status.take() else {
        return result;
    };

    loop {
        match rx.try_recv() {
            Ok(update) => {
                result.needs_redraw = true;

                // Handle device code info (show to user)
                if let Some(device_info) = &update.device_code {
                    auth_popup
                        .set_device_code(&device_info.user_code, &device_info.verification_uri);
                }

                if update.success {
                    // Save the OAuth token
                    if let Some(token) = update.token {
                        if let Err(e) = save_oauth_token(update.provider, token) {
                            tracing::error!("Failed to save OAuth token: {}", e);
                            auth_popup.set_oauth_error(&format!("Failed to save token: {}", e));
                        } else {
                            // Mark auth as complete
                            auth_popup.set_oauth_complete();

                            // Queue provider switch if needed
                            if active_provider != update.provider {
                                result =
                                    result.with_action(PollAction::SwitchProvider(update.provider));
                            }

                            result = result.with_message(
                                "system",
                                format!("{} authenticated via OAuth!", update.provider),
                            );
                        }
                    }
                } else {
                    // Show error
                    auth_popup.set_oauth_error(&update.message);
                }
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                if auth_popup.expire_oauth_browser_waiting(OAUTH_BROWSER_TIMEOUT) {
                    result.needs_redraw = true;
                    result = result.with_message(
                        "system",
                        "OAuth timed out before the browser callback arrived. Retry authentication if you still want to connect this provider.".to_string(),
                    );
                }
                channels.oauth_status = Some(rx);
                break;
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                if auth_popup.is_browser_waiting() {
                    auth_popup.set_oauth_error(
                        "OAuth flow ended unexpectedly before authentication completed.",
                    );
                    result.needs_redraw = true;
                    result = result.with_message(
                        "system",
                        "OAuth flow ended unexpectedly before authentication completed."
                            .to_string(),
                    );
                }
                break;
            }
        }
    }

    result
}

/// Save OAuth token to storage
fn save_oauth_token(provider: ProviderId, token: OAuthTokenData) -> anyhow::Result<()> {
    let mut store = OAuthTokenStore::load()?;
    store.set(provider, token);
    store.save()?;
    Ok(())
}
