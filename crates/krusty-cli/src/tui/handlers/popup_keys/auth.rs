//! Authentication popup keyboard handler

use crossterm::event::{KeyCode, KeyModifiers};

use crate::ai::providers::ProviderId;
use crate::tui::app::{App, Popup};
use crate::tui::popups::auth::AuthState;
use crate::tui::utils::{DeviceCodeInfo, OAuthStatusUpdate};
use krusty_core::auth::{openai_oauth_config, AuthMethod, BrowserOAuthFlow, DeviceCodeFlow};

impl App {
    /// Handle auth popup keyboard events
    pub fn handle_auth_popup_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        match &self.ui.popups.auth.state {
            AuthState::ProviderSelection { .. } => match code {
                KeyCode::Esc => self.ui.popup = Popup::None,
                KeyCode::Up => self.ui.popups.auth.prev_provider(),
                KeyCode::Down => self.ui.popups.auth.next_provider(),
                KeyCode::Enter => {
                    self.ui.popups.auth.confirm_provider();
                }
                _ => {}
            },
            AuthState::ApiKeyInput { provider, .. } => {
                let provider = *provider;
                self.handle_api_key_input(code, modifiers, provider);
            }
            AuthState::AuthMethodSelection { .. } => match code {
                KeyCode::Esc => self.ui.popups.auth.go_back(),
                KeyCode::Up => self.ui.popups.auth.prev_auth_method(),
                KeyCode::Down => self.ui.popups.auth.next_auth_method(),
                KeyCode::Enter => {
                    if let Some((provider, method)) = self.ui.popups.auth.confirm_auth_method() {
                        self.start_oauth_flow(provider, method);
                    }
                }
                _ => {}
            },
            AuthState::OAuthBrowserWaiting { .. } | AuthState::OAuthDeviceCode { .. } => {
                if code == KeyCode::Esc {
                    self.ui.popups.auth.go_back();
                }
            }
            AuthState::Complete { .. } => {
                if code == KeyCode::Esc || code == KeyCode::Enter {
                    self.ui.popups.auth.reset();
                    self.ui.popup = Popup::None;
                }
            }
        }
    }

    /// Handle API key input
    fn handle_api_key_input(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
        provider: ProviderId,
    ) {
        match code {
            KeyCode::Esc => self.ui.popups.auth.go_back(),
            KeyCode::Backspace if self.ui.popups.auth.get_api_key().is_none_or(str::is_empty) => {
                self.ui.popups.auth.go_back();
            }
            KeyCode::Backspace => self.ui.popups.auth.backspace_api_key(),
            KeyCode::Char('v') if modifiers.contains(KeyModifiers::CONTROL) => {
                if let Ok(mut clipboard) = arboard::Clipboard::new() {
                    if let Ok(text) = clipboard.get_text() {
                        for c in text.trim().chars() {
                            self.ui.popups.auth.add_api_key_char(c);
                        }
                    }
                }
            }
            KeyCode::Char(c) if !modifiers.contains(KeyModifiers::CONTROL) => {
                self.ui.popups.auth.add_api_key_char(c);
            }
            KeyCode::Enter => {
                let key = self.ui.popups.auth.get_api_key().map(|k| k.to_string());
                if let Some(key) = key {
                    if !key.is_empty() {
                        if self.runtime.active_provider != provider {
                            self.switch_provider(provider);
                        }
                        self.set_api_key(key);
                        self.runtime
                            .chat
                            .messages
                            .push(("system".to_string(), format!("{} API key saved!", provider)));
                        self.ui.popups.auth.set_api_key_complete();

                        if provider == ProviderId::OpenRouter {
                            self.start_openrouter_fetch();
                        }
                        if provider == ProviderId::OpenCodeZen {
                            self.start_opencodezen_fetch();
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Start OAuth flow for a provider
    pub(super) fn start_oauth_flow(&mut self, provider: ProviderId, method: AuthMethod) {
        let status_tx = self.services.oauth_status_tx.clone();

        match method {
            AuthMethod::OAuthBrowser => {
                self.ui
                    .popups
                    .auth
                    .set_oauth_browser_status("Opening browser...");

                tokio::spawn(async move {
                    let config = match provider {
                        ProviderId::OpenAI => openai_oauth_config(),
                        _ => {
                            let _ = status_tx.send(OAuthStatusUpdate {
                                provider,
                                success: false,
                                message: format!("{} does not support OAuth", provider),
                                device_code: None,
                                token: None,
                            });
                            return;
                        }
                    };

                    let flow = BrowserOAuthFlow::new(config);
                    match flow.run().await {
                        Ok(token) => {
                            let _ = status_tx.send(OAuthStatusUpdate {
                                provider,
                                success: true,
                                message: "Authentication successful".to_string(),
                                device_code: None,
                                token: Some(token),
                            });
                        }
                        Err(e) => {
                            let _ = status_tx.send(OAuthStatusUpdate {
                                provider,
                                success: false,
                                message: format!("OAuth failed: {}", e),
                                device_code: None,
                                token: None,
                            });
                        }
                    }
                });
            }
            AuthMethod::OAuthDevice => {
                tokio::spawn(async move {
                    let config = match provider {
                        ProviderId::OpenAI => openai_oauth_config(),
                        _ => {
                            let _ = status_tx.send(OAuthStatusUpdate {
                                provider,
                                success: false,
                                message: format!("{} does not support OAuth", provider),
                                device_code: None,
                                token: None,
                            });
                            return;
                        }
                    };

                    let flow = DeviceCodeFlow::new(config);

                    match flow.request_code().await {
                        Ok(code_response) => {
                            let _ = status_tx.send(OAuthStatusUpdate {
                                provider,
                                success: true,
                                message: "Enter the code in your browser".to_string(),
                                device_code: Some(DeviceCodeInfo {
                                    user_code: code_response.user_code.clone(),
                                    verification_uri: code_response.verification_uri.clone(),
                                }),
                                token: None,
                            });

                            match flow
                                .poll_for_token(&code_response.device_code, code_response.interval)
                                .await
                            {
                                Ok(token) => {
                                    let _ = status_tx.send(OAuthStatusUpdate {
                                        provider,
                                        success: true,
                                        message: "Authentication successful".to_string(),
                                        device_code: None,
                                        token: Some(token),
                                    });
                                }
                                Err(e) => {
                                    let _ = status_tx.send(OAuthStatusUpdate {
                                        provider,
                                        success: false,
                                        message: format!("Device auth failed: {}", e),
                                        device_code: None,
                                        token: None,
                                    });
                                }
                            }
                        }
                        Err(e) => {
                            let _ = status_tx.send(OAuthStatusUpdate {
                                provider,
                                success: false,
                                message: format!("Failed to get device code: {}", e),
                                device_code: None,
                                token: None,
                            });
                        }
                    }
                });
            }
            AuthMethod::ApiKey => {
                // Handled internally by confirm_auth_method
            }
        }
    }
}
