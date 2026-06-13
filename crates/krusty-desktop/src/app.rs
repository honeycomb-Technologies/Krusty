use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, App, AppContext as _, Context, Entity, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement as _, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Timer, Window,
};
use gpui_component::input::InputState;
use gpui_component::StyledExt as _;

use crate::api::{ActiveOAuthFlow, KrustyApiClient, ProviderStatus, ServerOverview};
use crate::components::settings_drawer::DRAWER_ANIMATION_DURATION;
use crate::components::{
    auth_settings, bottom_dock, file_picker, landing, settings_drawer, status_bar,
};
use crate::design::theme;
use crate::panel_actions::{self, SwapFocusedPanel, ToggleFocusedPanelAxis};
use crate::panels::{chat, scratch, LayoutNode, PanelId, PanelKind, PanelWorkspace, SplitAxis};
use crate::server;

pub fn init(cx: &mut App) {
    panel_actions::init(cx);
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AuthFlow {
    #[default]
    Choose,
    ApiKey,
}

#[derive(Clone, Debug)]
pub struct ProjectTab {
    pub title: String,
    pub directory: Option<String>,
    workspace: PanelWorkspace,
}

impl ProjectTab {
    fn new(title: String, directory: Option<String>, panel_seed: u64) -> Self {
        Self {
            title,
            directory,
            workspace: PanelWorkspace::starter_with_seed(panel_seed),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ConnectionState {
    #[default]
    Idle,
    Checking,
    Ready(ServerOverview),
    Failed(String),
}

pub struct KrustyDesktop {
    focus_handle: FocusHandle,
    settings_open: bool,
    settings_opening: bool,
    showing_landing: bool,
    projects: Vec<ProjectTab>,
    active_project: usize,
    next_workspace_seed: u64,
    chat_panels: BTreeMap<PanelId, Entity<chat::ChatPanel>>,
    scratch_canvases: BTreeMap<PanelId, Entity<scratch::ScratchCanvasPanel>>,
    server_url_input: Entity<InputState>,
    api_key_input: Entity<InputState>,
    oauth_code_input: Entity<InputState>,
    server_base_url: String,
    connection_state: ConnectionState,
    providers: Vec<ProviderStatus>,
    providers_error: Option<String>,
    selected_provider: Option<String>,
    hover_card_provider: Option<String>,
    auth_flow: AuthFlow,
    auth_pending: bool,
    active_oauth_flow: Option<ActiveOAuthFlow>,
    workspace_dialog_open: bool,
    workspace_picker_root: PathBuf,
    workspace_picker_selected: PathBuf,
    status: SharedString,
}

impl KrustyDesktop {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let server_url_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(server::default_server_url())
                .default_value(server::default_server_url())
        });
        let api_key_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("API key")
                .masked(true)
        });
        let oauth_code_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Authorization code"));
        let cwd = std::env::current_dir().unwrap_or_else(|_| home_dir());
        let picker_root = canonical_dir(cwd);

        Self {
            focus_handle: cx.focus_handle(),
            settings_open: false,
            settings_opening: true,
            showing_landing: true,
            projects: Vec::new(),
            active_project: 0,
            next_workspace_seed: 1,
            chat_panels: BTreeMap::new(),
            scratch_canvases: BTreeMap::new(),
            server_url_input,
            api_key_input,
            oauth_code_input,
            server_base_url: server::default_server_url().to_owned(),
            connection_state: ConnectionState::Idle,
            providers: Vec::new(),
            providers_error: None,
            selected_provider: None,
            hover_card_provider: None,
            auth_flow: AuthFlow::default(),
            auth_pending: false,
            active_oauth_flow: None,
            workspace_dialog_open: false,
            workspace_picker_root: picker_root.clone(),
            workspace_picker_selected: picker_root,
            status: "Krusty desktop shell ready.".into(),
        }
    }

    pub fn open_landing(&mut self, cx: &mut Context<Self>) {
        self.workspace_dialog_open = false;
        self.showing_landing = true;
        self.status = "Landing page active.".into();
        cx.notify();
    }

    pub fn start_workspace(&mut self, cx: &mut Context<Self>) {
        self.start_open_workspace_flow(cx);
    }

    pub fn start_new_workspace(&mut self, cx: &mut Context<Self>) {
        self.settings_open = false;
        self.workspace_dialog_open = false;
        let panel_seed = self.allocate_workspace_panel_seed();
        self.projects
            .push(ProjectTab::new("Untitled".to_owned(), None, panel_seed));
        self.active_project = self.projects.len().saturating_sub(1);
        self.showing_landing = false;
        self.status = "Created untitled workspace.".into();
        self.ensure_server(cx);
        cx.notify();
    }

    pub fn open_mako(&mut self, cx: &mut Context<Self>) {
        let mako_dir = home_dir().join(".krusty").join("mako");
        if std::fs::create_dir_all(&mako_dir).is_err() && !mako_dir.is_dir() {
            self.status = format!("Could not create Mako home at {}", mako_dir.display()).into();
            cx.notify();
            return;
        }
        self.open_workspace_path(mako_dir, cx);
    }

    pub fn start_open_workspace_flow(&mut self, cx: &mut Context<Self>) {
        self.settings_open = false;
        if !self.workspace_picker_root.is_dir() {
            self.workspace_picker_root = home_dir();
        }
        self.workspace_picker_root = canonical_dir(self.workspace_picker_root.clone());
        self.workspace_picker_selected = self.workspace_picker_root.clone();
        self.workspace_dialog_open = true;
        self.status = "Choose a folder to open as a Krusty workspace.".into();
        cx.notify();
    }

    pub fn cancel_workspace_dialog(&mut self, cx: &mut Context<Self>) {
        self.workspace_dialog_open = false;
        self.status = "Workspace selection cancelled.".into();
        cx.notify();
    }

    pub fn navigate_workspace_picker_root(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if !path.is_dir() {
            self.status = format!("Not a folder: {}", path.display()).into();
            cx.notify();
            return;
        }

        self.workspace_picker_root = canonical_dir(path);
        self.workspace_picker_selected = self.workspace_picker_root.clone();
        self.status = format!("Browsing {}", self.workspace_picker_root.display()).into();
        cx.notify();
    }

    pub fn select_workspace_picker_path(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if path.is_dir() {
            self.workspace_picker_selected = canonical_dir(path);
            cx.notify();
        }
    }

    pub fn open_selected_workspace(&mut self, cx: &mut Context<Self>) {
        self.open_workspace_path(self.workspace_picker_selected.clone(), cx);
    }

    pub fn open_workspace_path(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if !path.is_dir() {
            self.status = format!("Not a folder: {}", path.display()).into();
            cx.notify();
            return;
        }

        let root = canonical_dir(path);
        let directory = root.to_string_lossy().to_string();
        if let Some(index) = self
            .projects
            .iter()
            .position(|project| project.directory.as_deref() == Some(directory.as_str()))
        {
            self.select_project_tab(index, cx);
            self.workspace_dialog_open = false;
            return;
        }

        let title = workspace_title(&root);
        let panel_seed = self.allocate_workspace_panel_seed();
        self.projects
            .push(ProjectTab::new(title, Some(directory.clone()), panel_seed));
        self.active_project = self.projects.len().saturating_sub(1);
        self.workspace_dialog_open = false;
        self.showing_landing = false;
        self.status = format!("Opened Krusty workspace: {directory}").into();
        self.ensure_server(cx);
        cx.notify();
    }

    pub fn select_project_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.projects.len() {
            return;
        }
        self.active_project = index;
        self.showing_landing = false;
        self.workspace_dialog_open = false;
        if let Some(project) = self.projects.get(index) {
            self.status = format!("Workspace active: {}", project.title).into();
        }
        cx.notify();
    }

    pub fn close_project_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.projects.len() {
            return;
        }
        let closed = self.projects.remove(index);
        self.remove_panel_entities_for_workspace(&closed.workspace);
        if self.projects.is_empty() {
            self.active_project = 0;
            self.showing_landing = true;
        } else if self.active_project >= self.projects.len() {
            self.active_project = self.projects.len() - 1;
        } else if index < self.active_project {
            self.active_project = self.active_project.saturating_sub(1);
        }
        self.status = format!("Closed Krusty workspace: {}", closed.title).into();
        cx.notify();
    }

    pub fn toggle_settings(&mut self, cx: &mut Context<Self>) {
        if self.settings_open {
            self.close_settings(cx);
            return;
        }

        self.settings_open = true;
        self.settings_opening = true;
        self.status = "Settings drawer opened.".into();
        self.refresh_providers(cx);
        cx.notify();
    }

    pub fn close_settings(&mut self, cx: &mut Context<Self>) {
        if !self.settings_open {
            return;
        }

        self.settings_opening = false;
        self.status = "Settings drawer closed.".into();
        cx.notify();

        cx.spawn(async move |this, cx| {
            Timer::after(DRAWER_ANIMATION_DURATION).await;
            let _ = this.update(cx, |view, cx| {
                view.settings_open = false;
                cx.notify();
            });
        })
        .detach();
    }

    pub fn focus_panel(&mut self, id: PanelId, cx: &mut Context<Self>) {
        let mut next_status = None;
        if let Some(workspace) = self.active_workspace_mut() {
            workspace.focus(id);
            if let Some(panel) = workspace.panel(id) {
                next_status = Some(format!("Focused {} panel.", panel.title));
            }
        }
        if let Some(status) = next_status {
            self.status = status.into();
        }
        cx.notify();
    }

    pub fn focus_chat_input(&mut self, id: PanelId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(panel) = self.chat_panels.get(&id) {
            panel.update(cx, |panel, cx| panel.focus_input(window, cx));
        }
    }

    pub fn focus_next_panel(&mut self, cx: &mut Context<Self>) {
        if let Some(workspace) = self.active_workspace_mut() {
            workspace.focus_next();
            self.status = "Focused next panel.".into();
        }
        cx.notify();
    }

    pub fn swap_focused_panel(&mut self, cx: &mut Context<Self>) {
        if let Some(workspace) = self.active_workspace_mut() {
            if workspace.swap_focused_with_adjacent() {
                self.status = "Swapped focused panel.".into();
            } else {
                self.status = "No adjacent panel to swap.".into();
            }
        }
        cx.notify();
    }

    pub fn toggle_focused_panel_axis(&mut self, cx: &mut Context<Self>) {
        if let Some(workspace) = self.active_workspace_mut() {
            if workspace.toggle_focused_split_axis() {
                self.status = "Toggled focused panel split axis.".into();
            } else {
                self.status = "No split axis to toggle for focused panel.".into();
            }
        }
        cx.notify();
    }

    pub fn split_focused(&mut self, axis: SplitAxis, kind: PanelKind, cx: &mut Context<Self>) {
        self.showing_landing = false;
        if let Some(workspace) = self.active_workspace_mut() {
            let id = workspace.split_focused(axis, kind);
            workspace.focus(id);
            self.status = format!("Added {} panel.", kind.title()).into();
        } else {
            let panel_seed = self.allocate_workspace_panel_seed();
            self.projects
                .push(ProjectTab::new("Untitled".to_owned(), None, panel_seed));
            self.active_project = self.projects.len().saturating_sub(1);
            self.status = "Created untitled workspace.".into();
        }
        cx.notify();
    }

    pub fn set_status(&mut self, status: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.status = status.into();
        cx.notify();
    }

    pub fn refresh_providers(&mut self, cx: &mut Context<Self>) {
        let client = self.api_client();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { client.list_credentials() })
                .await;
            let _ = this.update(cx, |view, cx| {
                match result {
                    Ok(providers) => {
                        view.providers = providers;
                        view.providers_error = None;
                        if view.selected_provider.is_none() {
                            view.selected_provider =
                                view.providers.first().map(|provider| provider.id.clone());
                        }
                        view.status = format!("Loaded {} providers.", view.providers.len()).into();
                    }
                    Err(error) => {
                        let message = format!("{error:#}");
                        view.providers_error = Some(message.clone());
                        view.status = message.into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn select_provider(&mut self, provider: String, cx: &mut Context<Self>) {
        self.selected_provider = Some(provider);
        self.auth_flow = AuthFlow::Choose;
        self.active_oauth_flow = None;
        cx.notify();
    }

    pub fn start_provider_hover(&mut self, provider: String, cx: &mut Context<Self>) {
        self.hover_card_provider = Some(provider);
        cx.notify();
    }

    pub fn end_provider_hover(&mut self, provider: &str, cx: &mut Context<Self>) {
        if self.hover_card_provider.as_deref() == Some(provider) {
            self.hover_card_provider = None;
            cx.notify();
        }
    }

    pub fn show_api_key_flow(&mut self, cx: &mut Context<Self>) {
        self.auth_flow = AuthFlow::ApiKey;
        cx.notify();
    }

    pub fn save_api_key(&mut self, cx: &mut Context<Self>) {
        let Some(provider) = self.selected_provider.clone() else {
            self.status = "Select a provider before saving an API key.".into();
            cx.notify();
            return;
        };
        let api_key = self.api_key_input.read(cx).value().trim().to_string();
        if api_key.is_empty() {
            self.status = "Enter an API key before submitting.".into();
            cx.notify();
            return;
        }

        let client = self.api_client();
        let provider_for_request = provider.clone();
        self.auth_pending = true;
        self.status = format!("Saving API key for {provider}…").into();
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(
                    async move { client.set_credential(&provider_for_request, api_key) },
                )
                .await;
            let _ = this.update(cx, |view, cx| {
                view.auth_pending = false;
                match result {
                    Ok(_) => {
                        view.auth_flow = AuthFlow::Choose;
                        view.status = format!("Saved API key for {provider}.").into();
                        view.refresh_providers(cx);
                    }
                    Err(error) => {
                        view.status =
                            format!("Failed to save API key for {provider}: {error:#}").into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn remove_selected_auth(&mut self, cx: &mut Context<Self>) {
        let Some(provider) = self.selected_provider.clone() else {
            self.status = "Select a provider before removing auth.".into();
            cx.notify();
            return;
        };
        let has_oauth = self
            .providers
            .iter()
            .find(|status| status.id == provider)
            .is_some_and(|status| status.has_oauth);
        let client = self.api_client();
        let provider_for_request = provider.clone();
        self.auth_pending = true;
        self.status = format!("Removing auth for {provider}…").into();
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if has_oauth {
                        client.revoke_oauth(&provider_for_request).map(|_| ())
                    } else {
                        client.delete_credential(&provider_for_request).map(|_| ())
                    }
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.auth_pending = false;
                match result {
                    Ok(()) => {
                        view.active_oauth_flow = None;
                        view.auth_flow = AuthFlow::Choose;
                        view.status = format!("Removed auth for {provider}.").into();
                        view.refresh_providers(cx);
                    }
                    Err(error) => {
                        view.status =
                            format!("Failed to remove auth for {provider}: {error:#}").into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn start_oauth_login(&mut self, flow_type: Option<&str>, cx: &mut Context<Self>) {
        let Some(provider) = self.selected_provider.clone() else {
            self.status = "Select a provider before starting OAuth.".into();
            cx.notify();
            return;
        };

        let client = self.api_client();
        let flow_type = flow_type.map(str::to_owned);
        let provider_for_request = provider.clone();
        self.auth_pending = true;
        self.active_oauth_flow = None;
        self.status = format!("Starting OAuth for {provider}…").into();
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    client.start_oauth(&provider_for_request, flow_type.as_deref())
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                match result {
                    Ok(response) => {
                        if let Some(auth_url) = oauth_browser_url(&response) {
                            if let Err(error) = open_browser(&auth_url) {
                                view.auth_pending = false;
                                view.status =
                                    format!("OAuth started but browser could not open: {error:#}")
                                        .into();
                                cx.notify();
                                return;
                            }
                        }

                        view.active_oauth_flow = Some(ActiveOAuthFlow {
                            provider: response.provider.clone(),
                            flow_type: response.flow_type.clone(),
                            paste_code: response.paste_code,
                            device_user_code: response
                                .device_code
                                .as_ref()
                                .map(|device| device.user_code.clone()),
                        });
                        view.auth_flow = AuthFlow::Choose;

                        if response.paste_code {
                            view.auth_pending = false;
                            view.status = format!(
                                "Paste the authorization code for {} in settings.",
                                response.provider
                            )
                            .into();
                            cx.notify();
                            return;
                        }

                        let poll_client = view.api_client();
                        let poll_provider = response.provider;
                        cx.spawn(async move |this, cx| {
                            let connected = poll_oauth_until_complete(&poll_client, &poll_provider)
                                .await
                                .unwrap_or(false);
                            let _ = this.update(cx, |view, cx| {
                                view.auth_pending = false;
                                if connected {
                                    view.active_oauth_flow = None;
                                    view.status =
                                        format!("OAuth connected for {poll_provider}.").into();
                                    view.refresh_providers(cx);
                                } else {
                                    view.status =
                                        format!("OAuth for {poll_provider} is still pending.")
                                            .into();
                                }
                                cx.notify();
                            });
                        })
                        .detach();
                    }
                    Err(error) => {
                        view.auth_pending = false;
                        view.status =
                            format!("Failed to start OAuth for {provider}: {error:#}").into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn exchange_oauth_code(&mut self, cx: &mut Context<Self>) {
        let Some(flow) = self.active_oauth_flow.clone() else {
            self.status = "No active OAuth flow awaiting a code.".into();
            cx.notify();
            return;
        };
        let code = self.oauth_code_input.read(cx).value().trim().to_string();
        if code.is_empty() {
            self.status = "Paste an authorization code before submitting.".into();
            cx.notify();
            return;
        }

        let client = self.api_client();
        let provider = flow.provider;
        let provider_for_request = provider.clone();
        self.auth_pending = true;
        self.status = format!("Submitting OAuth code for {provider}…").into();
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(
                    async move { client.exchange_oauth_code(&provider_for_request, code) },
                )
                .await;
            let _ = this.update(cx, |view, cx| {
                view.auth_pending = false;
                match result {
                    Ok(()) => {
                        view.active_oauth_flow = None;
                        view.status = format!("OAuth connected for {provider}.").into();
                        view.refresh_providers(cx);
                    }
                    Err(error) => {
                        view.status =
                            format!("Failed to exchange OAuth code for {provider}: {error:#}")
                                .into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn refresh_connection(&mut self, cx: &mut Context<Self>) {
        let client = self.input_api_client(cx);
        let server = client.base_url().to_owned();
        self.server_base_url = server.clone();
        self.connection_state = ConnectionState::Checking;
        self.status = format!("Checking Krusty server at {server}…").into();
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async move { client.overview() }).await;
            let _ = this.update(cx, |view, cx| {
                match result {
                    Ok(overview) => {
                        view.status =
                            format!("Connected to Krusty server: {}", overview.summary()).into();
                        view.connection_state = ConnectionState::Ready(overview);
                        view.refresh_providers(cx);
                    }
                    Err(error) => {
                        let message = format!("Server unavailable at {server}: {error:#}");
                        view.status = message.clone().into();
                        view.connection_state = ConnectionState::Failed(message);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn ensure_server(&mut self, cx: &mut Context<Self>) {
        let preferred_url = self.server_url_input.read(cx).value().to_string();
        self.connection_state = ConnectionState::Checking;
        self.status = "Starting or reusing Krusty server…".into();
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { server::ensure_local_server(preferred_url) })
                .await;
            let _ = this.update(cx, |view, cx| {
                match result {
                    Ok(result) => {
                        let client = KrustyApiClient::new(result.base_url.clone());
                        view.server_base_url = client.base_url().to_owned();
                        match client.overview() {
                            Ok(overview) => {
                                view.status =
                                    format!("{} {}", result.detail, overview.summary()).into();
                                view.connection_state = ConnectionState::Ready(overview);
                                view.refresh_providers(cx);
                            }
                            Err(error) => {
                                let message = format!(
                                    "Server started but overview failed at {}: {error:#}",
                                    client.base_url()
                                );
                                view.status = message.clone().into();
                                view.connection_state = ConnectionState::Failed(message);
                            }
                        }
                    }
                    Err(error) => {
                        let message = format!("Krusty server unavailable: {error:#}");
                        view.status = message.clone().into();
                        view.connection_state = ConnectionState::Failed(message);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn connection_summary(&self) -> String {
        match &self.connection_state {
            ConnectionState::Idle => "Not checked yet".to_owned(),
            ConnectionState::Checking => "Checking…".to_owned(),
            ConnectionState::Ready(overview) => overview.summary(),
            ConnectionState::Failed(error) => error.clone(),
        }
    }

    pub fn oauth_code_input(&self) -> Entity<InputState> {
        self.oauth_code_input.clone()
    }

    fn auth_settings_state(&self) -> auth_settings::AuthSettingsState {
        auth_settings::AuthSettingsState {
            providers: self.providers.clone(),
            providers_error: self.providers_error.clone(),
            selected_provider: self.selected_provider.clone(),
            hover_card_provider: self.hover_card_provider.clone(),
            auth_flow: self.auth_flow,
            pending: self.auth_pending,
            active_oauth_flow: self.active_oauth_flow.clone(),
        }
    }

    fn api_client(&self) -> KrustyApiClient {
        KrustyApiClient::new(self.server_base_url.clone())
    }

    fn input_api_client(&self, cx: &mut Context<Self>) -> KrustyApiClient {
        KrustyApiClient::new(self.server_url_input.read(cx).value().to_string())
    }

    fn active_workspace(&self) -> Option<&PanelWorkspace> {
        self.projects
            .get(self.active_project)
            .map(|project| &project.workspace)
    }

    fn active_workspace_mut(&mut self) -> Option<&mut PanelWorkspace> {
        self.projects
            .get_mut(self.active_project)
            .map(|project| &mut project.workspace)
    }

    fn active_project_dir(&self) -> Option<String> {
        self.projects
            .get(self.active_project)
            .and_then(|project| project.directory.clone())
    }

    fn allocate_workspace_panel_seed(&mut self) -> u64 {
        let seed = self.next_workspace_seed.saturating_mul(1_000);
        self.next_workspace_seed = self.next_workspace_seed.saturating_add(1);
        seed.max(1)
    }

    fn remove_panel_entities_for_workspace(&mut self, workspace: &PanelWorkspace) {
        for id in workspace.panel_ids() {
            self.chat_panels.remove(&id);
            self.scratch_canvases.remove(&id);
        }
    }

    pub fn projects(&self) -> &[ProjectTab] {
        &self.projects
    }

    pub fn active_project(&self) -> usize {
        self.active_project
    }

    pub fn settings_open(&self) -> bool {
        self.settings_open
    }

    pub fn workspace_dialog_open(&self) -> bool {
        self.workspace_dialog_open
    }

    pub fn workspace_picker_root(&self) -> &Path {
        &self.workspace_picker_root
    }

    pub fn workspace_picker_selected(&self) -> &Path {
        &self.workspace_picker_selected
    }

    fn on_swap_focused_panel(
        &mut self,
        _: &SwapFocusedPanel,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.swap_focused_panel(cx);
    }

    fn on_toggle_focused_panel_axis(
        &mut self,
        _: &ToggleFocusedPanelAxis,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_focused_panel_axis(cx);
    }
}

impl Focusable for KrustyDesktop {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for KrustyDesktop {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::app_bg())
            .text_color(theme::text())
            .track_focus(&self.focus_handle)
            .child(status_bar::status_bar(self, cx))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .overflow_hidden()
                    .child(if self.showing_landing || self.projects.is_empty() {
                        landing::landing_page(cx).into_any_element()
                    } else {
                        self.render_workspace(window, cx).into_any_element()
                    })
                    .when(self.settings_open, |this| {
                        this.child(settings_drawer::settings_backdrop(
                            self.settings_opening,
                            cx,
                        ))
                        .child(settings_drawer::settings_drawer(
                            self.settings_opening,
                            self.server_url_input.clone(),
                            self.api_key_input.clone(),
                            theme::current_appearance(),
                            self.connection_summary(),
                            self.auth_settings_state(),
                            cx,
                        ))
                    })
                    .when(self.workspace_dialog_open(), |this| {
                        this.child(file_picker::workspace_dialog_backdrop(cx))
                            .child(file_picker::open_workspace_dialog(
                                self.workspace_picker_root(),
                                self.workspace_picker_selected(),
                                cx,
                            ))
                    }),
            )
            .child(bottom_dock::bottom_dock(self, cx))
            .when_some(
                gpui_component::Root::render_notification_layer(window, cx),
                |this, layer| this.child(layer),
            )
    }
}

impl KrustyDesktop {
    fn render_workspace(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl gpui::IntoElement {
        div()
            .size_full()
            .p_2()
            .flex()
            .flex_col()
            .key_context(panel_actions::WORKSPACE_KEY_CONTEXT)
            .on_action(cx.listener(Self::on_swap_focused_panel))
            .on_action(cx.listener(Self::on_toggle_focused_panel_axis))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .border_1()
                    .border_color(theme::hairline())
                    .bg(theme::app_bg())
                    .child({
                        let layout = self
                            .active_workspace()
                            .map(PanelWorkspace::layout)
                            .cloned()
                            .unwrap_or_else(|| LayoutNode::Panel(PanelId::default()));
                        self.render_layout_node(&layout, window, cx)
                    }),
            )
    }

    fn render_layout_node(
        &mut self,
        node: &LayoutNode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        match node {
            LayoutNode::Panel(id) => self.render_panel(*id, window, cx),
            LayoutNode::Split {
                axis,
                ratio: _,
                first,
                second,
            } => {
                let mut container = div().size_full().gap_2();
                container = match axis {
                    SplitAxis::Horizontal => container.flex().flex_row(),
                    SplitAxis::Vertical => container.flex().flex_col(),
                };

                container
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .child(self.render_layout_node(first, window, cx)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .child(self.render_layout_node(second, window, cx)),
                    )
                    .into_any_element()
            }
        }
    }

    fn render_panel(
        &mut self,
        id: PanelId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(panel) = self
            .active_workspace()
            .and_then(|workspace| workspace.panel(id))
        else {
            return div().child("Missing panel").into_any_element();
        };
        let focused = self
            .active_workspace()
            .is_some_and(|workspace| workspace.focused() == id);
        let kind = panel.kind;
        let title = panel.title.clone();

        div()
            .id(SharedString::from(format!("panel-{}", id.raw())))
            .size_full()
            .min_w_0()
            .min_h_0()
            .border_1()
            .border_color(if focused {
                theme::accent()
            } else {
                theme::hairline()
            })
            .bg(theme::surface())
            .flex()
            .flex_col()
            .child(panel_header(id, title, focused, kind, cx))
            .child(self.render_panel_body(id, kind, window, cx))
            .into_any_element()
    }

    fn render_panel_body(
        &mut self,
        id: PanelId,
        kind: PanelKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        match kind {
            PanelKind::Chat => {
                let client = self.api_client();
                let project_dir = self.active_project_dir();
                let created_client = client.clone();
                let panel = self
                    .chat_panels
                    .entry(id)
                    .or_insert_with(|| {
                        cx.new(|cx| chat::ChatPanel::new(created_client, window, cx))
                    })
                    .clone();
                if panel.read(cx).needs_context_sync(&client, &project_dir) {
                    panel.update(cx, |panel, _| {
                        panel.set_client(client);
                        panel.set_project_dir(project_dir);
                    });
                }
                panel.into_any_element()
            }
            PanelKind::ScratchCanvas => self
                .scratch_canvases
                .entry(id)
                .or_insert_with(|| cx.new(|_| scratch::ScratchCanvasPanel::new()))
                .clone()
                .into_any_element(),
        }
    }
}

fn panel_header(
    id: PanelId,
    title: String,
    focused: bool,
    kind: PanelKind,
    cx: &mut Context<KrustyDesktop>,
) -> gpui::Stateful<gpui::Div> {
    let mut header = div()
        .id(SharedString::from(format!("panel-header-{}", id.raw())))
        .h(px(32.0))
        .px_3()
        .border_b_1()
        .border_color(theme::hairline())
        .flex()
        .items_center()
        .justify_between()
        .cursor_pointer()
        .bg(if focused {
            theme::surface_selected()
        } else {
            theme::surface()
        })
        .child(div().text_sm().font_semibold().child(title));

    header = match kind {
        PanelKind::Chat => header.on_click(cx.listener(move |view, _, window, cx| {
            view.focus_panel(id, cx);
            view.focus_chat_input(id, window, cx);
        })),
        PanelKind::ScratchCanvas => header.on_click(cx.listener(move |view, _, window, cx| {
            view.focus_panel(id, cx);
            view.focus_handle.focus(window);
        })),
    };

    header
}

async fn poll_oauth_until_complete(
    client: &KrustyApiClient,
    provider: &str,
) -> anyhow::Result<bool> {
    for _ in 0..90 {
        let status = client.oauth_status(provider)?;
        if status.has_token {
            return Ok(true);
        }
        if !status.flow_active {
            return Ok(false);
        }
        Timer::after(Duration::from_secs(2)).await;
    }
    Ok(false)
}

fn oauth_browser_url(response: &crate::api::OAuthStartResponse) -> Option<String> {
    if let Some(device) = response.device_code.as_ref() {
        return Some(
            device
                .verification_uri_complete
                .clone()
                .unwrap_or_else(|| device.verification_uri.clone()),
        );
    }

    let auth_url = response.auth_url.trim();
    if auth_url.is_empty() {
        None
    } else {
        Some(auth_url.to_owned())
    }
}

fn open_browser(url: &str) -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| anyhow::anyhow!("failed to open browser with xdg-open: {error}"))?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| anyhow::anyhow!("failed to open browser with open: {error}"))?;
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = url;
        return Err(anyhow::anyhow!(
            "browser launch is not supported on this platform"
        ));
    }

    Ok(())
}

fn canonical_dir(path: PathBuf) -> PathBuf {
    std::fs::canonicalize(&path).unwrap_or(path)
}

fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

fn workspace_title(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::oauth_browser_url;
    use crate::api::{OAuthDeviceCode, OAuthStartResponse};

    #[test]
    fn oauth_browser_url_skips_server_managed_browser_processes() {
        let response = OAuthStartResponse {
            auth_url: String::new(),
            provider: "grok".to_owned(),
            flow_type: "browser_process".to_owned(),
            paste_code: false,
            device_code: None,
        };

        assert_eq!(oauth_browser_url(&response), None);
    }

    #[test]
    fn oauth_browser_url_prefers_device_complete_url() {
        let response = OAuthStartResponse {
            auth_url: "https://fallback.example".to_owned(),
            provider: "openai".to_owned(),
            flow_type: "device".to_owned(),
            paste_code: false,
            device_code: Some(OAuthDeviceCode {
                user_code: "ABCD-EFGH".to_owned(),
                verification_uri: "https://verify.example".to_owned(),
                verification_uri_complete: Some("https://verify.example/complete".to_owned()),
                expires_in: 900,
            }),
        };

        assert_eq!(
            oauth_browser_url(&response).as_deref(),
            Some("https://verify.example/complete")
        );
    }
}
