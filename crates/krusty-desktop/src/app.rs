use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, App, AppContext as _, Context, Entity, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement as _, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window,
};
use gpui_component::input::InputState;
use gpui_component::StyledExt as _;

use crate::api::{KrustyApiClient, ServerOverview};
use crate::components::{bottom_dock, file_picker, landing, settings_drawer, status_bar};
use crate::design::theme;
use crate::panels::{chat, scratch, LayoutNode, PanelId, PanelKind, PanelWorkspace, SplitAxis};
use crate::server;

pub fn init(_cx: &mut App) {}

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
    showing_landing: bool,
    projects: Vec<ProjectTab>,
    active_project: usize,
    next_workspace_seed: u64,
    chat_panels: BTreeMap<PanelId, Entity<chat::ChatPanel>>,
    scratch_canvases: BTreeMap<PanelId, Entity<scratch::ScratchCanvasPanel>>,
    server_url_input: Entity<InputState>,
    server_base_url: String,
    connection_state: ConnectionState,
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
        let cwd = std::env::current_dir().unwrap_or_else(|_| home_dir());
        let picker_root = canonical_dir(cwd);

        Self {
            focus_handle: cx.focus_handle(),
            settings_open: false,
            showing_landing: true,
            projects: Vec::new(),
            active_project: 0,
            next_workspace_seed: 1,
            chat_panels: BTreeMap::new(),
            scratch_canvases: BTreeMap::new(),
            server_url_input,
            server_base_url: server::default_server_url().to_owned(),
            connection_state: ConnectionState::Idle,
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
        self.settings_open = !self.settings_open;
        self.status = if self.settings_open {
            "Settings drawer opened."
        } else {
            "Settings drawer closed."
        }
        .into();
        cx.notify();
    }

    pub fn close_settings(&mut self, cx: &mut Context<Self>) {
        self.settings_open = false;
        cx.notify();
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

    pub fn focus_next_panel(&mut self, cx: &mut Context<Self>) {
        if let Some(workspace) = self.active_workspace_mut() {
            workspace.focus_next();
            self.status = "Focused next panel.".into();
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
                        this.child(settings_drawer::settings_backdrop(cx)).child(
                            settings_drawer::settings_drawer(
                                self.server_url_input.clone(),
                                theme::current_appearance(),
                                self.connection_summary(),
                                cx,
                            ),
                        )
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
        div().size_full().p_2().flex().flex_col().child(
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
            .on_click(cx.listener(move |view, _, _window, cx| {
                view.focus_panel(id, cx);
            }))
            .child(panel_header(title, focused))
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
                let created_client = client.clone();
                let panel = self
                    .chat_panels
                    .entry(id)
                    .or_insert_with(|| {
                        cx.new(|cx| chat::ChatPanel::new(created_client, window, cx))
                    })
                    .clone();
                panel.update(cx, |panel, _| panel.set_client(client));
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

fn panel_header(title: String, focused: bool) -> gpui::Div {
    div()
        .h(px(32.0))
        .px_3()
        .border_b_1()
        .border_color(theme::hairline())
        .flex()
        .items_center()
        .justify_between()
        .bg(if focused {
            theme::surface_selected()
        } else {
            theme::surface()
        })
        .child(div().text_sm().font_semibold().child(title))
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
