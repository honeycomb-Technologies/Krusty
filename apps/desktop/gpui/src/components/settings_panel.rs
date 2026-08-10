//! Settings surface: two-column tree matching ChatGPT/Codex desktop (bar 1:1).
//!
//! LEFT (~260px): Back to app · search · Personal / Integrations / Coding / Archived
//! RIGHT: section content with persisted desktop preferences plus explicitly
//! labeled live backend/account/catalog controls.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, Context, HighlightStyle, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, StyledText,
};
use gpui_component::input::Input;
use gpui_component::{Icon, IconName, Sizable as _};
use mitsuro_desktop_backend::{AppInfo, BackendKind, HookMetadata, InstalledApp};

use crate::app::{
    AccountSession, McpAddTransport, MitsuroApp, ProductMode, SettingsNavGroup, SettingsSection,
    SurfaceDataState, UiConnection,
};
use crate::theme;

/// Settings left column width (bar ~240–280).
const NAV_W: f32 = 260.0;

/// Full-height Settings panel for ProductMode::Settings.
pub fn settings_panel(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let section = app.settings_section();
    let query = app.settings_search_query().to_string();
    let search_input = app.settings_search_input().clone();

    div()
        .id("settings-panel")
        .flex()
        .flex_row()
        .flex_1()
        .min_w_0()
        .h_full()
        .bg(colors.bg_main)
        .child(settings_nav(app, &search_input, &query, section, cx))
        .child(settings_content(app, section, cx))
}

// ─── Left nav ───────────────────────────────────────────────────────────────

fn settings_nav(
    app: &MitsuroApp,
    search_input: &gpui::Entity<gpui_component::input::InputState>,
    query: &str,
    selected: SettingsSection,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let q = query.to_string();

    div()
        .id("settings-nav")
        .flex()
        .flex_col()
        .w(px(NAV_W))
        .h_full()
        .flex_shrink_0()
        .bg(colors.bg_sidebar)
        .border_r_1()
        .border_color(colors.border)
        .child(back_to_app_row(cx))
        .child(settings_search_row(search_input))
        .child(
            div()
                .id("settings-nav-scroll")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px(px(10.0))
                .pb(px(16.0))
                .gap(px(2.0))
                .children(
                    SettingsNavGroup::all()
                        .iter()
                        .filter_map(|group| {
                            let items: Vec<SettingsSection> = SettingsSection::all()
                                .iter()
                                .copied()
                                .filter(|s| s.group() == *group && s.matches_query(&q))
                                .collect();
                            if items.is_empty() {
                                None
                            } else {
                                Some(nav_group(*group, &items, selected, cx))
                            }
                        })
                        .collect::<Vec<_>>(),
                ),
        )
        .child(
            div()
                .px(px(12.0))
                .py(px(8.0))
                .border_t_1()
                .border_color(colors.border_subtle)
                .text_xs()
                .text_color(colors.text_tertiary)
                .child(format!("Mitsuro · {}", app.connection().chip_label())),
        )
}

fn back_to_app_row(cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("settings-back")
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .px(px(12.0))
        .pt(px(12.0))
        .pb(px(6.0))
        .cursor_pointer()
        .hover(|s| s.bg(colors.bg_hover))
        .on_click(cx.listener(|app, _, window, cx| {
            app.leave_settings(window, cx);
        }))
        .child(
            Icon::new(IconName::ArrowLeft)
                .with_size(px(14.0))
                .text_color(colors.text_secondary),
        )
        .child(
            div()
                .text_sm()
                .text_color(colors.text_secondary)
                .child("Back to app"),
        )
}

fn settings_search_row(
    search_input: &gpui::Entity<gpui_component::input::InputState>,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("settings-search-wrap")
        .px(px(10.0))
        .py(px(8.0))
        .child(
            div()
                .id("settings-search-field")
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .h(px(32.0))
                .px(px(10.0))
                .rounded(px(999.0))
                .bg(colors.bg_elevated)
                .border_1()
                .border_color(colors.border)
                .child(
                    Icon::new(IconName::Search)
                        .with_size(px(13.0))
                        .text_color(colors.text_tertiary),
                )
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .text_color(colors.text)
                        .child(Input::new(search_input).appearance(false).h(px(24.0))),
                ),
        )
}

fn nav_group(
    group: SettingsNavGroup,
    items: &[SettingsSection],
    selected: SettingsSection,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let group_id = format!("settings-group-{}", group.label());
    div()
        .id(SharedId(group_id))
        .flex()
        .flex_col()
        .gap(px(1.0))
        .mt(px(10.0))
        .child(
            div()
                .px(px(10.0))
                .py(px(4.0))
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(colors.text_tertiary)
                .child(group.label().to_string()),
        )
        .children(
            items
                .iter()
                .map(|s| nav_item(*s, selected == *s, cx))
                .collect::<Vec<_>>(),
        )
}

fn nav_item(
    section: SettingsSection,
    selected: bool,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let id = format!("settings-nav-{}", section.label());
    let icon_color = if selected {
        colors.text
    } else {
        colors.text_secondary
    };
    let label_color = icon_color;
    div()
        .id(SharedId(id))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .h(px(30.0))
        .px(px(10.0))
        .rounded(px(999.0))
        .cursor_pointer()
        .when(selected, |s| s.bg(colors.bg_selected))
        .when(!selected, |s| s.hover(|s| s.bg(colors.bg_hover)))
        .on_click(cx.listener(move |app, _, _, cx| {
            app.set_settings_section(section, cx);
        }))
        .child(
            Icon::empty()
                .path(section_icon_path(section))
                .with_size(px(14.0))
                .text_color(icon_color),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .text_color(label_color)
                .child(section.label().to_string()),
        )
        // Bar Account row shows trailing external-link (↗) affordance.
        .when(section == SettingsSection::Account, |this| {
            this.child(
                Icon::empty()
                    .path("icons/external-link.svg")
                    .with_size(px(12.0))
                    .text_color(colors.text_tertiary),
            )
        })
}

/// Settings left-nav glyphs — aligned to ChatGPT/Codex bar (lucide assets).
fn section_icon_path(section: SettingsSection) -> &'static str {
    match section {
        SettingsSection::General => "icons/settings.svg",
        SettingsSection::LinuxDesktop => "icons/monitor.svg",
        SettingsSection::Import => "icons/arrow-down.svg",
        SettingsSection::Profile => "icons/user.svg",
        SettingsSection::Appearance => "icons/sun.svg",
        SettingsSection::Voice => "icons/mic.svg",
        SettingsSection::Configuration => "icons/settings-2.svg",
        SettingsSection::Personalization => "icons/heart.svg",
        SettingsSection::Pets => "icons/bot.svg",
        SettingsSection::KeyboardShortcuts => "icons/keyboard.svg",
        SettingsSection::UsageBilling => "icons/chart-pie.svg",
        SettingsSection::Account => "icons/circle-user.svg",
        SettingsSection::Plugins => "icons/puzzle.svg",
        SettingsSection::Browser => "icons/app-window.svg",
        SettingsSection::ComputerUse => "icons/sparkles.svg",
        SettingsSection::Hooks => "icons/anchor.svg",
        SettingsSection::Connections => "icons/network.svg",
        SettingsSection::Git => "icons/git-branch.svg",
        SettingsSection::Environments => "icons/building-2.svg",
        SettingsSection::Worktrees => "icons/git-branch.svg",
        SettingsSection::ArchivedChats => "icons/inbox.svg",
    }
}

// ─── Right content ──────────────────────────────────────────────────────────

fn settings_content(
    app: &MitsuroApp,
    section: SettingsSection,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("settings-content")
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .bg(colors.bg_main)
        .child(
            div()
                .id("settings-content-scroll")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px(px(40.0))
                .py(px(28.0))
                .child(content_title(section.label()))
                .child(settings_scope_notice(section))
                .child(section_body(app, section, cx)),
        )
}

fn content_title(title: &str) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .mb(px(20.0))
        .text_xl()
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.text)
        .child(title.to_string())
}

fn settings_scope_notice(section: SettingsSection) -> impl IntoElement {
    let colors = theme::colors();
    let copy = match section {
        SettingsSection::Connections => {
            "Backend selection and reconnect are live. Unavailable connection mutations are labeled on their rows."
        }
        SettingsSection::Account | SettingsSection::UsageBilling => {
            "Account and usage data come from the connected backend. Unsupported account actions are labeled before use."
        }
        _ => {
            "Preferences below are saved on this device. They do not change Mitsuro server or ChatGPT / Codex configuration unless a row is explicitly labeled live."
        }
    };
    div()
        .mb(px(18.0))
        .w_full()
        .max_w(px(720.0))
        .border_l_2()
        .border_color(colors.border)
        .pl(px(11.0))
        .py(px(2.0))
        .text_xs()
        .text_color(colors.text_tertiary)
        .child(copy)
}

fn section_body(
    app: &MitsuroApp,
    section: SettingsSection,
    cx: &mut Context<MitsuroApp>,
) -> gpui::AnyElement {
    match section {
        SettingsSection::General => general_body(app, cx).into_any_element(),
        SettingsSection::LinuxDesktop => linux_body(app, cx).into_any_element(),
        SettingsSection::Import => import_body(app, cx).into_any_element(),
        SettingsSection::Profile => profile_body(app, cx).into_any_element(),
        SettingsSection::Appearance => appearance_body(app, cx).into_any_element(),
        SettingsSection::Voice => voice_body(app, cx).into_any_element(),
        SettingsSection::Configuration => configuration_body(app, cx).into_any_element(),
        SettingsSection::Personalization => personalization_body(app, cx).into_any_element(),
        SettingsSection::Pets => pets_body(app, cx).into_any_element(),
        SettingsSection::KeyboardShortcuts => keyboard_body(app, cx).into_any_element(),
        SettingsSection::UsageBilling => usage_body(app, cx).into_any_element(),
        SettingsSection::Account => account_body(app, cx).into_any_element(),
        SettingsSection::Plugins => plugins_body(app, cx).into_any_element(),
        SettingsSection::Browser => browser_body(app, cx).into_any_element(),
        SettingsSection::ComputerUse => computer_use_body(app, cx).into_any_element(),
        SettingsSection::Hooks => hooks_body(app, cx).into_any_element(),
        SettingsSection::Connections => connections_body(app, cx).into_any_element(),
        SettingsSection::Git => git_body(app, cx).into_any_element(),
        SettingsSection::Environments => environments_body(app, cx).into_any_element(),
        SettingsSection::Worktrees => worktrees_body(app, cx).into_any_element(),
        SettingsSection::ArchivedChats => archived_body(app, cx).into_any_element(),
    }
}

// ─── General (bar 10-settings-general + reverse asar) ───────────────────────

fn general_body(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    div()
        .id("settings-general")
        .flex()
        .flex_col()
        .gap(px(22.0))
        .max_w(px(720.0))
        .child(group_label("Permissions"))
        .child(
            settings_card()
                .child(toggle_row(
                    "Default permissions",
                    "Saved locally for desktop parity; the connected backend remains the authority for actual tool permissions",
                    "default_permissions",
                    true,
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(full_access_row(app, cx)),
        )
        .child(group_label("General"))
        .child(
            settings_card()
                .child(select_row(
                    "Default file open destination",
                    "Where files and folders open by default",
                    "file_open_dest",
                    &["Zed", "VS Code", "System"],
                    "Zed",
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(select_row(
                    "Language",
                    "Language for the app UI",
                    "language",
                    &["Auto detect", "English", "日本語"],
                    "Auto detect",
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(toggle_row(
                    "Bottom panel",
                    "Show the bottom panel control in the app header",
                    "bottom_panel",
                    true,
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(segment_row(
                    "Default terminal location",
                    "Choose where the terminal shortcut and environment actions open terminal tabs",
                    "terminal_location",
                    &["Bottom", "Right"],
                    "Bottom",
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(toggle_row(
                    "Prevent sleep while running",
                    "Keep your computer awake while Mitsuro is running a task",
                    "prevent_sleep",
                    false,
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(select_row(
                    "Speed",
                    "Choose how quickly Mitsuro runs across chats, subagents, and compaction",
                    "speed",
                    &["Fast", "Balanced", "Thorough"],
                    "Fast",
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(toggle_row(
                    "Suggested prompts",
                    "Suggest what to do next by searching project files and connected apps",
                    "suggested_prompts",
                    true,
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(action_row(
                    "Open source licenses",
                    "Third-party notices for bundled dependencies",
                    "View",
                    "oss-licenses",
                    cx,
                )),
        )
        // Bar first-screen stack: Permissions → General → Composer → Popout Window.
        // Notifications / Experimental stay below Popout (reverse density, not between Composer/Popout).
        .child(group_label("Composer"))
        .child(
            settings_card()
                .child(toggle_row(
                    "Show context window usage",
                    "Display how much of the model context window is in use",
                    "show_context_usage",
                    false,
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(select_row(
                    "Send shortcut",
                    "Choose when Enter sends a prompt or inserts a new line",
                    "send_shortcut",
                    &["Enter", "Ctrl+Enter"],
                    "Enter",
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(segment_row(
                    "Follow-up behavior",
                    "Queue follow-ups while Mitsuro runs or steer the current run. Press Ctrl+e to do the opposite for one message",
                    "follow_up",
                    &["Queue", "Steer"],
                    "Queue",
                    app,
                    cx,
                )),
        )
        .child(group_label("Popout Window"))
        .child(
            settings_card()
                .child(hotkey_row(
                    "Popout Window hotkey",
                    "Set a global shortcut for Popout Window. Leave unset to keep it off.",
                    "Off",
                    "popout-hotkey",
                    cx,
                ))
                .child(card_divider())
                .child(toggle_row(
                    "Default to standalone chat",
                    "Start new chats outside of any project",
                    "popout_standalone",
                    false,
                    app,
                    cx,
                )),
        )
        .child(group_label("Notifications"))
        .child(
            settings_card()
                .child(toggle_row(
                    "Turn complete",
                    "Set when Mitsuro alerts you that it's finished",
                    "notify_turn_complete",
                    true,
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(toggle_row(
                    "Permission needed",
                    "Show alerts when notification permissions are required",
                    "notify_permission",
                    true,
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(toggle_row(
                    "Input needed",
                    "Show alerts when input is needed to continue",
                    "notify_input",
                    true,
                    app,
                    cx,
                )),
        )
        .child(group_label("Experimental features (Beta)"))
        .child(
            settings_card()
                .child(toggle_row(
                    "Plugins",
                    "Enable the plugins experience in Mitsuro",
                    "exp_plugins",
                    true,
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(toggle_row(
                    "Request user input",
                    "Allow Mitsuro to ask questions outside Plan mode. Changes apply only to new threads",
                    "exp_request_user_input",
                    false,
                    app,
                    cx,
                )),
        )
}

// ─── Linux desktop ──────────────────────────────────────────────────────────

fn linux_body(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let build = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let app_label = format!("Mitsuro Desktop {} ({build})", env!("CARGO_PKG_VERSION"));
    let binary = std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "Unavailable".into());
    let protocol = app
        .active_backend_kind()
        .map(MitsuroApp::backend_display_name)
        .unwrap_or("No backend selected");
    div()
        .id("settings-linux")
        .flex()
        .flex_col()
        .gap(px(22.0))
        .max_w(px(720.0))
        .child(group_label("Desktop integration"))
        .child(
            settings_card()
                .child(toggle_row(
                    "Compact prompt window",
                    "Use a smaller always-on-top prompt window for quick Mitsuro tasks",
                    "compact_prompt",
                    true,
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(toggle_row(
                    "System tray",
                    "Keep Mitsuro available from the system tray when the main window is closed",
                    "system_tray",
                    true,
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(toggle_row(
                    "Warm start",
                    "Preload app-server so the first prompt after launch is faster",
                    "warm_start",
                    true,
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(toggle_row(
                    "Install updates when closing",
                    "Apply downloaded updates the next time you quit Mitsuro",
                    "install_updates_on_close",
                    false,
                    app,
                    cx,
                )),
        )
        .child(group_label("Build info"))
        .child(
            settings_card()
                .child(info_row("App", &app_label))
                .child(card_divider())
                .child(info_row("Binary", &binary))
                .child(card_divider())
                .child(info_row("Backend", protocol)),
        )
}

// ─── Other sections (bar-like multi-row chrome) ─────────────────────────────

fn import_body(_app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    div()
        .id("settings-import")
        .flex()
        .flex_col()
        .gap(px(22.0))
        .max_w(px(720.0))
        .child(group_label("Import from other tools"))
        .child(
            div()
                .text_xs()
                .text_color(theme::colors().text_tertiary)
                .child(
                    "Import adapters are not connected in this native build. Existing settings are unchanged."
                        .to_string(),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(12.0))
                .child(import_source_card(
                    "Claude Code",
                    "Projects, CLAUDE.md → AGENTS.md, settings.json → config.toml, skills & plugins",
                    "Import",
                    "import-claude-code",
                    cx,
                ))
                .child(import_source_card(
                    "Cursor",
                    "Cursor rules → AGENTS.md, Cursor settings → config.toml, recent projects",
                    "Import",
                    "import-cursor",
                    cx,
                ))
                .child(import_source_card(
                    "Claude Cowork",
                    "Shared workspaces and instruction packs from Claude Cowork",
                    "Import",
                    "import-claude-cowork",
                    cx,
                )),
        )
}

fn profile_body(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let name = app.profile_display_name().to_string();
    let email = app
        .account_session()
        .email_display
        .clone()
        .unwrap_or_else(|| "Not reported".into());
    let plan = app
        .account_session()
        .plan_label
        .clone()
        .unwrap_or_else(|| "Not reported".into());
    div()
        .id("settings-profile")
        .flex()
        .flex_col()
        .gap(px(22.0))
        .max_w(px(720.0))
        .child(group_label("Profile"))
        .child(
            settings_card()
                .child(info_row("Display name", &name))
                .child(card_divider())
                .child(info_row("Email", &email))
                .child(card_divider())
                .child(info_row("Plan", &plan))
                .child(card_divider())
                .child(toggle_row(
                    "Show name in sidebar",
                    "Display your profile name under the avatar in the home sidebar",
                    "profile_show_name",
                    true,
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(action_row(
                    "Avatar",
                    "Local PNG via assets/avatars (AssetSource)",
                    "Change",
                    "profile-avatar",
                    cx,
                )),
        )
        .child(group_label("Workspace"))
        .child(
            settings_card()
                .child(info_row("Workspace", "Personal"))
                .child(card_divider())
                .child(toggle_row(
                    "Share usage analytics",
                    "Help improve Mitsuro with anonymous product metrics",
                    "profile_analytics",
                    false,
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(action_row(
                    "Export profile data",
                    "Download a local copy of profile preferences",
                    "Export",
                    "profile-export",
                    cx,
                )),
        )
}

fn appearance_body(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    // Reverse asar (general-settings / settings.general.appearance.*): Theme +
    // Preferences (accent/bg/fg/contrast/fonts/diff/motion) + Light/Dark chrome.
    div()
        .id("settings-appearance")
        .flex()
        .flex_col()
        .gap(px(22.0))
        .max_w(px(720.0))
        // ── Theme ──────────────────────────────────────────────────────────
        .child(group_label("Theme"))
        .child(settings_card().child(segment_row(
            "Theme",
            "Use light, dark, or match your system",
            "theme",
            &["Light", "Dark", "System"],
            "Dark",
            app,
            cx,
        )))
        // ── Preferences (bar multi-block density) ──────────────────────────
        .child(group_label("Preferences"))
        .child(
            settings_card()
                .child(accent_swatch_row(app, cx))
                .child(card_divider())
                .child(color_chip_row(
                    "Background",
                    "Surface color behind chat and settings chrome",
                    "bg_color",
                    0x0d0d0d,
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(color_chip_row(
                    "Foreground",
                    "Primary ink color for text and icons",
                    "fg_color",
                    0xe8e8e8,
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(segment_row(
                    "Contrast",
                    "Boost border and fill separation",
                    "contrast",
                    &["Default", "Medium", "High"],
                    "Default",
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(toggle_row(
                    "Translucent sidebar",
                    "Let the desktop wallpaper show through the home sidebar",
                    "translucent_sidebar",
                    true,
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(toggle_row(
                    "Use system accent",
                    "Match window chrome accents to the desktop theme",
                    "use_system_theme",
                    false,
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(toggle_row(
                    "Use pointer cursors",
                    "Change the cursor to a pointer when hovering over interactive elements",
                    "pointer_cursors",
                    true,
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(segment_row(
                    "Diff markers",
                    "Show changes using colors or +/− markers",
                    "diff_markers",
                    &["Color", "+/-"],
                    "Color",
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(segment_row(
                    "Reduce motion",
                    "Reduce animations or match your system",
                    "reduce_motion",
                    &["System", "On", "Off"],
                    "Off",
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(select_row(
                    "UI font",
                    "Sans family used across the shell",
                    "ui_font",
                    &["Inter", "System UI", "SF Pro", "Noto Sans"],
                    "Inter",
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(select_row(
                    "UI font size",
                    "Adjust the base size used for the Mitsuro UI",
                    "ui_font_size",
                    &["12px", "13px", "14px", "15px", "16px"],
                    "14px",
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(select_row(
                    "Code font",
                    "Monospace family used for code, diffs, and terminal",
                    "code_font",
                    &["JetBrains Mono", "SF Mono", "Fira Code", "System mono"],
                    "JetBrains Mono",
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(select_row(
                    "Code font size",
                    "Adjust the base size used for code across chats and diffs",
                    "code_font_size",
                    &["12px", "13px", "14px", "15px"],
                    "13px",
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(toggle_row(
                    "Font smoothing",
                    "Use native font anti-aliasing for UI text",
                    "font_smoothing",
                    true,
                    app,
                    cx,
                )),
        )
        // ── Interface density ──────────────────────────────────────────────
        .child(group_label("Interface density"))
        .child(settings_card().child(segment_row(
            "Density",
            "Spacing of lists, cards, and settings rows",
            "density",
            &["Comfortable", "Compact"],
            "Comfortable",
            app,
            cx,
        )))
        // ── Dark theme chrome ──────────────────────────────────────────────
        .child(group_label("Dark theme"))
        .child(
            settings_card()
                .child(action_row(
                    "Copy theme",
                    "Copy a shareable dark-theme string to the clipboard",
                    "Copy",
                    "copy-dark-theme",
                    cx,
                ))
                .child(card_divider())
                .child(action_row(
                    "Import theme",
                    "Paste a theme share string to override dark chrome",
                    "Import",
                    "import-dark-theme",
                    cx,
                )),
        )
        // ── Light theme chrome ─────────────────────────────────────────────
        .child(group_label("Light theme"))
        .child(
            settings_card()
                .child(action_row(
                    "Copy theme",
                    "Copy a shareable light-theme string to the clipboard",
                    "Copy",
                    "copy-light-theme",
                    cx,
                ))
                .child(card_divider())
                .child(action_row(
                    "Import theme",
                    "Paste a theme share string to override light chrome",
                    "Import",
                    "import-light-theme",
                    cx,
                )),
        )
}

/// 8 clickable accent color circles (fixture only — selection stored in settings_choices).
fn accent_swatch_row(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let current = app.settings_choice("accent_color", "Blue");
    // Name + hex pairs — Codex-like accent palette.
    const SWATCHES: &[(&str, u32)] = &[
        ("Blue", 0x339cff),
        ("Purple", 0xa78bfa),
        ("Green", 0x04b84c),
        ("Orange", 0xf5a524),
        ("Pink", 0xf472b6),
        ("Red", 0xfa423e),
        ("Cyan", 0x22d3ee),
        ("Gray", 0x9ca3af),
    ];
    div()
        .id("accent-swatch-row")
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(16.0))
        .px(px(14.0))
        .py(px(14.0))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(3.0))
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text)
                        .child("Accent color"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child("Highlight color for selection and focus rings"),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .children(SWATCHES.iter().map(|(name, hex)| {
                    let selected = current.as_str() == *name;
                    let name_owned = (*name).to_string();
                    let swatch_id = format!("accent-swatch-{name}");
                    div()
                        .id(SharedId(swatch_id))
                        .size(px(22.0))
                        .rounded_full()
                        .bg(theme::hex(*hex))
                        .cursor_pointer()
                        .border_2()
                        .border_color(if selected {
                            theme::hex(0xffffff)
                        } else {
                            theme::hex_alpha(0xffffff, 0.12)
                        })
                        .when(selected, |s| {
                            s.shadow(vec![gpui::BoxShadow {
                                color: theme::hex_alpha(*hex, 0.45),
                                offset: gpui::point(px(0.0), px(0.0)),
                                blur_radius: px(8.0),
                                spread_radius: px(1.0),
                            }])
                        })
                        .hover(|s| s.opacity(0.9))
                        .on_click(cx.listener(move |app, _, _, cx| {
                            app.set_settings_choice("accent_color", name_owned.clone(), cx);
                        }))
                })),
        )
}

fn voice_body(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let state = app.realtime_voices_state();
    if state != SurfaceDataState::Live {
        let detail = match state {
            SurfaceDataState::Loading => {
                "Waiting for thread/realtime/listVoices from the Codex app-server."
            }
            SurfaceDataState::Unsupported => {
                "Realtime voice is a Codex app-server capability. Mitsuro HTTP does not currently expose an equivalent audio transport."
            }
            SurfaceDataState::Error => {
                "The Codex app-server did not return its realtime voice catalog."
            }
            _ => "Realtime voice is unavailable for this backend.",
        };
        return div()
            .id("settings-voice")
            .flex()
            .flex_col()
            .gap(px(12.0))
            .max_w(px(720.0))
            .child(group_label("Voice"))
            .child(settings_card().child(empty_list_message(
                &format!("Realtime voice · {}", state.label()),
                detail,
            )));
    }

    let options = app.realtime_voice_options();
    let selected = app.selected_realtime_voice_label();
    div()
        .id("settings-voice")
        .flex()
        .flex_col()
        .gap(px(22.0))
        .max_w(px(720.0))
        .child(group_label("General"))
        .child(
            settings_card()
                .child(realtime_voice_row(options, selected, cx))
                .child(card_divider())
                .child(info_row("Microphone", "System default via PipeWire"))
                .child(card_divider())
                .child(info_row(
                    "Availability",
                    "Voice chat starts from the conversation composer",
                )),
        )
        .child(group_label("Current support"))
        .child(settings_card().child(empty_list_message(
            "Codex realtime v3",
            "Audio is captured and played through PipeWire. Global hotkeys, screen context, and standalone dictation remain unavailable until their native contracts are implemented.",
        )))
}

fn realtime_voice_row(
    options: Vec<String>,
    selected: String,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .px(px(14.0))
        .py(px(12.0))
        .flex()
        .flex_col()
        .gap(px(10.0))
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(colors.text)
                .child("Voice"),
        )
        .child(
            div()
                .text_xs()
                .text_color(colors.text_tertiary)
                .child("Choose from the catalog returned by the connected Codex app-server"),
        )
        .child(div().flex().flex_row().flex_wrap().gap(px(7.0)).children(
            options.into_iter().enumerate().map(|(index, label)| {
                let active = label == selected;
                let click_label = label.clone();
                div()
                    .id(("realtime-voice-option", index))
                    .px(px(10.0))
                    .py(px(6.0))
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(if active { colors.accent } else { colors.border })
                    .bg(if active {
                        theme::hex_alpha(0x2f6df6, 0.18)
                    } else {
                        colors.bg_button_secondary
                    })
                    .text_xs()
                    .text_color(if active {
                        colors.text
                    } else {
                        colors.text_secondary
                    })
                    .cursor_pointer()
                    .hover(|style| style.bg(colors.bg_hover))
                    .on_click(cx.listener(move |app, _, _, cx| {
                        app.select_realtime_voice(click_label.clone(), cx);
                    }))
                    .child(label)
            }),
        ))
}

fn configuration_body(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let snippet = app.config_snippet().to_string();
    let body = if snippet.trim().is_empty() {
        "# ~/.config/mitsuro/config.toml\nmodel = \"sol-ultra\"\nsandbox = \"workspace-write\"\n# Set MITSURO_* env vars to override at launch"
            .to_string()
    } else {
        snippet
    };
    let colors = theme::colors();
    div()
        .id("settings-configuration")
        .flex()
        .flex_col()
        .gap(px(22.0))
        .max_w(px(720.0))
        .child(group_label("config.toml"))
        .child(
            settings_card()
                .child(info_row("Path", "~/.config/mitsuro/config.toml"))
                .child(card_divider())
                .child(
                    div()
                        .px(px(14.0))
                        .py(px(12.0))
                        .text_xs()
                        .font_family("monospace")
                        .text_color(colors.text_secondary)
                        .child(body),
                )
                .child(card_divider())
                .child(action_row(
                    "Open config.toml",
                    "Edit model, sandbox, provider, and custom tables",
                    "Open",
                    "open-config",
                    cx,
                ))
                .child(card_divider())
                .child(toggle_row(
                    "Prefer project AGENTS.md",
                    "Merge workspace agent instructions ahead of user defaults",
                    "prefer_agents_md",
                    true,
                    app,
                    cx,
                )),
        )
        .child(group_label("Environment"))
        .child(
            settings_card()
                .child(info_row(
                    "Env vars",
                    "MITSURO_FORCE_FIXTURE, MITSURO_START_MODE, OPENAI_API_KEY…",
                ))
                .child(card_divider())
                .child(
                    div()
                        .px(px(14.0))
                        .py(px(10.0))
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child(
                            "Environment variables override config.toml at process launch. \
Shell profile and desktop launchers are not rewritten by Mitsuro."
                                .to_string(),
                        ),
                )
                .child(card_divider())
                .child(action_row(
                    "Custom config.toml settings",
                    "Advanced keys not exposed in the Settings UI",
                    "View",
                    "custom-config-keys",
                    cx,
                )),
        )
}

fn personalization_body(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    // Reverse: personalization-settings — Personality Friendly/Pragmatic, agents.md, Memory + Chronicle.
    div()
        .id("settings-personalization")
        .flex()
        .flex_col()
        .gap(px(22.0))
        .max_w(px(720.0))
        .child(group_label("Personality"))
        .child(
            settings_card()
                .child(segment_row(
                    "Personality",
                    "Choose a default tone for Mitsuro responses",
                    "personality",
                    &["Friendly", "Pragmatic"],
                    "Friendly",
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(
                    div()
                        .px(px(14.0))
                        .py(px(10.0))
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child(
                            "Personality settings are not supported by every model. Mitsuro's tone can be customized in Custom instructions."
                                .to_string(),
                        ),
                )
                .child(card_divider())
                .child(toggle_row(
                    "Remember project preferences",
                    "Carry style and tooling notes across threads in a workspace",
                    "remember_project_prefs",
                    true,
                    app,
                    cx,
                )),
        )
        .child(group_label("Custom instructions"))
        .child(
            settings_card()
                .child(
                    div()
                        .px(px(14.0))
                        .pt(px(12.0))
                        .pb(px(6.0))
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text)
                        .child("Custom instructions".to_string()),
                )
                .child(
                    div()
                        .px(px(14.0))
                        .pb(px(8.0))
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .gap(px(4.0))
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.text_tertiary)
                                .child(
                                    "Give Mitsuro extra instructions and context for all chats on this host."
                                        .to_string(),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.accent)
                                .child("Learn more".to_string()),
                        ),
                )
                .child(
                    div()
                        .mx(px(14.0))
                        .mb(px(12.0))
                        .min_h(px(96.0))
                        .px(px(12.0))
                        .py(px(10.0))
                        .rounded(px(10.0))
                        .bg(colors.bg_main)
                        .border_1()
                        .border_color(colors.border)
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(
                            div()
                                .text_sm()
                                .text_color(colors.text_tertiary)
                                .child("Add your custom instructions…".to_string()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.text_tertiary)
                                .child(
                                    "Prefer concise diffs. Always run cargo check after edits."
                                        .to_string(),
                                ),
                        ),
                )
                .child(card_divider())
                .child(action_row(
                    "Save",
                    "Writes to local agents.md / personalization store",
                    "Save",
                    "save-custom-instructions",
                    cx,
                )),
        )
        .child(group_label("Memory"))
        .child(
            settings_card()
                .child(
                    div()
                        .px(px(14.0))
                        .pt(px(12.0))
                        .pb(px(8.0))
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .gap(px(4.0))
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.text_tertiary)
                                .child(
                                    "Configure how local memories are collected, retained, and consolidated on this computer."
                                        .to_string(),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.accent)
                                .child("Learn more".to_string()),
                        ),
                )
                .child(card_divider())
                .child(toggle_row(
                    "Enable local memories",
                    "Create memories from chats on this computer and use them to personalize future chats on this computer",
                    "enable_local_memories",
                    true,
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(toggle_row(
                    "Memories from tool-assisted chats",
                    "Generate memories from chats that used MCP tools or web search",
                    "memory_from_tools",
                    false,
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(action_row(
                    "Delete local memories",
                    "Delete all memories stored locally on this computer",
                    "Delete",
                    "delete-memories",
                    cx,
                )),
        )
        .child(group_label("Chronicle research preview"))
        .child(
            settings_card()
                .child(toggle_row(
                    "Enable Chronicle research preview",
                    "Augment local memories with on-screen context so Mitsuro can help with anything you're working on",
                    "chronicle_preview",
                    false,
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(info_row("Screen Recording", "Not requested"))
                .child(card_divider())
                .child(info_row("Accessibility", "Not requested")),
        )
}

fn pets_body(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    // Reverse pets-settings: Pick a pet, Appearance (size), Custom pets.
    div()
        .id("settings-pets")
        .flex()
        .flex_col()
        .gap(px(22.0))
        .max_w(px(720.0))
        .child(group_label("Pick a pet"))
        .child(
            settings_card()
                .child(select_row(
                    "Pick a pet",
                    "Pets manage threads and surface what needs attention",
                    "pet_kind",
                    &["Fox", "Robot", "Cat", "Octopus"],
                    "Fox",
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(toggle_row(
                    "Wake pet",
                    "Show the desktop pet overlay when enabled",
                    "pets_enabled",
                    false,
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(action_row(
                    "Tuck away pet",
                    "Hide the pet without changing your selection",
                    "Tuck",
                    "pet-tuck",
                    cx,
                )),
        )
        .child(group_label("Appearance"))
        .child(
            settings_card()
                .child(select_row(
                    "Pet size",
                    "Adjust the size of your pet",
                    "pet_size",
                    &["Small", "Medium", "Large"],
                    "Medium",
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(select_row(
                    "Position",
                    "Where the pet docks relative to the Mitsuro window",
                    "pet_position",
                    &["Bottom-right", "Bottom-left", "Top-right"],
                    "Bottom-right",
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(toggle_row(
                    "React to turn progress",
                    "Animate when a run starts or finishes",
                    "pets_react",
                    true,
                    app,
                    cx,
                )),
        )
        .child(group_label("Custom pets"))
        .child(
            settings_card()
                .child(action_row(
                    "Create your own pet",
                    "Add a custom companion from a local folder",
                    "Create",
                    "pet-create",
                    cx,
                ))
                .child(card_divider())
                .child(action_row(
                    "Open folder",
                    "~/.local/share/mitsuro/pets",
                    "Open",
                    "pet-open-folder",
                    cx,
                ))
                .child(card_divider())
                .child(action_row(
                    "Refresh",
                    "Reload custom pets from disk",
                    "Refresh",
                    "pet-refresh",
                    cx,
                )),
        )
}

fn keyboard_body(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    // Reverse: keyboard-shortcuts-settings — searchable list + reset all.
    let colors = theme::colors();
    div()
        .id("settings-keyboard")
        .flex()
        .flex_col()
        .gap(px(22.0))
        .max_w(px(720.0))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .h(px(32.0))
                .px(px(12.0))
                .rounded(px(8.0))
                .bg(colors.bg_elevated)
                .border_1()
                .border_color(colors.border)
                .child(
                    Icon::new(IconName::Search)
                        .with_size(px(13.0))
                        .text_color(colors.text_tertiary),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(colors.text_tertiary)
                        .child("Search shortcuts".to_string()),
                ),
        )
        .child(group_label("General"))
        .child(
            settings_card()
                .child(shortcut_row("Open settings", "Ctrl+,"))
                .child(card_divider())
                .child(shortcut_row("New chat", "Ctrl+N"))
                .child(card_divider())
                .child(shortcut_row("Toggle sidebar", "Ctrl+B"))
                .child(card_divider())
                .child(shortcut_row("Focus composer", "Ctrl+L"))
                .child(card_divider())
                .child(shortcut_row("Command palette", "Ctrl+K"))
                .child(card_divider())
                .child(shortcut_row("Show keyboard shortcuts", "Ctrl+/"))
                .child(card_divider())
                .child(shortcut_row("Archive current chat", "Ctrl+Shift+A"))
                .child(card_divider())
                .child(shortcut_row("Popout Window hotkey", "Off")),
        )
        .child(group_label("Chat & runs"))
        .child(
            settings_card()
                .child(shortcut_row("Send message", "Enter"))
                .child(card_divider())
                .child(shortcut_row("New line in composer", "Shift+Enter"))
                .child(card_divider())
                .child(shortcut_row("Stop generation", "Esc"))
                .child(card_divider())
                .child(shortcut_row("Toggle voice chat", "Ctrl+Shift+V"))
                .child(card_divider())
                .child(shortcut_row("Start dictation", "Ctrl+Shift+D"))
                .child(card_divider())
                .child(shortcut_row("Toggle Fast mode", "Ctrl+Shift+F"))
                .child(card_divider())
                .child(shortcut_row("Toggle plan mode", "Ctrl+Shift+P"))
                .child(card_divider())
                .child(shortcut_row("Approve tool call", "Ctrl+Enter")),
        )
        .child(group_label("Global dictation & voice"))
        .child(
            settings_card()
                .child(shortcut_row("Hold-to-dictate hotkey", "Off"))
                .child(card_divider())
                .child(shortcut_row("Toggle dictation hotkey", "Off"))
                .child(card_divider())
                .child(shortcut_row("Voice Chat hotkey", "Off"))
                .child(card_divider())
                .child(shortcut_row("End Voice Chat", "Esc"))
                .child(card_divider())
                .child(shortcut_row("Toggle Voice Chat microphone", "M")),
        )
        .child(group_label("Navigation"))
        .child(
            settings_card()
                .child(shortcut_row("Go to Chat", "Ctrl+1"))
                .child(card_divider())
                .child(shortcut_row("Go to Work", "Ctrl+2"))
                .child(card_divider())
                .child(shortcut_row("Go to Codex", "Ctrl+3"))
                .child(card_divider())
                .child(shortcut_row("Open terminal", "Ctrl+`"))
                .child(card_divider())
                .child(shortcut_row("Toggle browser panel", "Ctrl+Shift+B"))
                .child(card_divider())
                .child(shortcut_row("Show pet", "Ctrl+Shift+."))
                .child(card_divider())
                .child(toggle_row(
                    "Use Emacs-style bindings in composer",
                    "Ctrl+A/E/K style editing in the prompt box",
                    "emacs_bindings",
                    false,
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(action_row(
                    "Reset all to defaults",
                    "This will discard all custom shortcuts and restore the defaults",
                    "Reset",
                    "keyboard-reset",
                    cx,
                )),
        )
}

fn usage_body(app: &MitsuroApp, _cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let account = app.account_session();
    let state = app.account_state();
    if !matches!(state, SurfaceDataState::Live | SurfaceDataState::Fixture) {
        let detail = match state {
            SurfaceDataState::Loading => {
                "Waiting for account/usage/read and account/rateLimits/read."
            }
            SurfaceDataState::Unsupported => {
                "The Mitsuro server does not expose ChatGPT account or usage data."
            }
            SurfaceDataState::Error => {
                "The connected backend did not return a complete account usage snapshot."
            }
            _ => "Account usage is unavailable.",
        };
        return div()
            .id("settings-usage")
            .flex()
            .flex_col()
            .gap(px(12.0))
            .max_w(px(720.0))
            .child(group_label("Usage"))
            .child(settings_card().child(empty_list_message(
                &format!("Usage · {}", state.label()),
                detail,
            )));
    }

    let primary = account.primary_used_percent().clamp(0, 100) as f32;
    let secondary = account.secondary_used_percent().clamp(0, 100) as f32;
    let lifetime = format_token_count(account.lifetime_tokens());
    let plan = account
        .plan_label
        .clone()
        .unwrap_or_else(|| "Not reported".into());
    div()
        .id("settings-usage")
        .flex()
        .flex_col()
        .gap(px(22.0))
        .max_w(px(720.0))
        .child(group_label("Backend account snapshot"))
        .child(
            settings_card()
                .child(info_row("Source", account.source))
                .child(card_divider())
                .child(info_row("Plan", &plan))
                .child(card_divider())
                .child(info_row("Lifetime tokens", &lifetime)),
        )
        .child(group_label("Rate limits"))
        .child(
            settings_card().child(
                div()
                    .px(px(14.0))
                    .py(px(12.0))
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .child(usage_bar_row(
                        "Primary window",
                        primary,
                        format!("{primary:.0}% used"),
                    ))
                    .child(usage_bar_row(
                        "Secondary window",
                        secondary,
                        format!("{secondary:.0}% used"),
                    )),
            ),
        )
}

fn account_body(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let data_state = app.account_state();
    if !matches!(
        data_state,
        SurfaceDataState::Live | SurfaceDataState::Fixture
    ) {
        let detail = match data_state {
            SurfaceDataState::Loading => {
                "Waiting for the connected backend to return account data."
            }
            SurfaceDataState::Unsupported => {
                "The Mitsuro server does not expose ChatGPT account operations."
            }
            SurfaceDataState::Error => {
                "The connected backend could not return a complete account snapshot."
            }
            _ => "Account data is unavailable.",
        };
        return div()
            .id("settings-account")
            .flex()
            .flex_col()
            .gap(px(12.0))
            .max_w(px(720.0))
            .child(group_label("Account"))
            .child(settings_card().child(empty_list_message(
                &format!("Account · {}", data_state.label()),
                detail,
            )));
    }
    let account = app.account_session().clone();
    let status = app.account_status_label();
    let conn = app.connection();
    let profile_name = app.profile_display_name().to_string();
    div()
        .id("settings-account")
        .flex()
        .flex_col()
        .gap(px(22.0))
        .max_w(px(720.0))
        .child(group_label("Account"))
        .child(account_section(
            &account,
            status.as_ref(),
            conn,
            &profile_name,
            cx,
        ))
}

fn plugins_body(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let count = app.plugins().len();
    let installed = app.plugins().iter().filter(|p| p.installed).count();
    div()
        .id("settings-plugins")
        .flex()
        .flex_col()
        .gap(px(22.0))
        .max_w(px(720.0))
        .child(group_label("Plugins"))
        .child(
            settings_card()
                .child(info_row(
                    "Installed",
                    &format!("{installed} of {count} listed"),
                ))
                .child(card_divider())
                .child(toggle_row(
                    "Auto-update plugins",
                    "Check marketplaces for updates on launch",
                    "plugins_auto_update",
                    true,
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(toggle_row(
                    "Allow skill install from marketplace",
                    "Install skills and MCP connectors listed in Plugins",
                    "plugins_allow_marketplace",
                    true,
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(action_row(
                    "Manage plugins",
                    "Open the Plugins surface in the home sidebar",
                    "Open",
                    "open-plugins",
                    cx,
                ))
                .child(card_divider())
                .child(action_row(
                    "Refresh catalog",
                    "Reload installed plugins and marketplace listings",
                    "Refresh",
                    "plugins-refresh",
                    cx,
                )),
        )
        .child(group_label("Skills & MCPs"))
        .child(
            settings_card()
                .child(info_row("Skills", "Bundled + marketplace"))
                .child(card_divider())
                .child(info_row("MCP from plugins", "Shown under Connections"))
                .child(card_divider())
                .child(action_row(
                    "Open Connections",
                    "Manage MCP servers connected via plugins",
                    "Open",
                    "plugins-open-connections",
                    cx,
                )),
        )
}

fn browser_body(_app: &MitsuroApp, _cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    div()
        .id("settings-browser")
        .flex()
        .flex_col()
        .gap(px(22.0))
        .max_w(px(720.0))
        .child(group_label("Browser"))
        .child(
            settings_card()
                .child(info_row("Default browser", "System default"))
                .child(card_divider())
                .child(info_row("Atlas surface", "External browser bridge"))
                .child(card_divider())
                .child(info_row("Embedded browser", "Unavailable in this build")),
        )
        .child(group_label("Data"))
        .child(
            settings_card()
                .child(unavailable_action_row(
                    "Cookies and site data",
                    "Owned by your system browser; Mitsuro does not read or clear them",
                    "Managed externally",
                ))
                .child(card_divider())
                .child(unavailable_action_row(
                    "Browser profile import",
                    "Profile discovery does not copy cookies, logins, or browsing data",
                    "Unavailable",
                )),
        )
}

fn computer_use_body(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    div()
        .id("settings-computer-use")
        .flex()
        .flex_col()
        .gap(px(22.0))
        .max_w(px(720.0))
        .child(group_label("Computer use"))
        .child(
            settings_card()
                .child(toggle_row(
                    "Allow computer use",
                    "Let Mitsuro control apps and the desktop when you approve a session",
                    "computer_use_enabled",
                    true,
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(toggle_row(
                    "Confirm destructive actions",
                    "Ask before file deletes or system-level commands",
                    "computer_confirm_actions",
                    true,
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(toggle_row(
                    "Allow network tools",
                    "Permit tools that reach the network without re-prompt",
                    "computer_network",
                    false,
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(select_row(
                    "Default environment",
                    "Environment used for computer-use sessions",
                    "computer_env",
                    &["Local", "Remote sandbox"],
                    "Local",
                    app,
                    cx,
                )),
        )
        .child(group_label("Always-allowed apps"))
        .child(
            settings_card()
                .child(empty_list_message(
                    "None yet",
                    "Apps you always allow for computer use will appear here.",
                ))
                .child(card_divider())
                .child(action_row(
                    "Manage allowed apps",
                    "Review apps that skip the permission prompt",
                    "Manage",
                    "computer-allowed-apps",
                    cx,
                )),
        )
}

fn hooks_body(app: &MitsuroApp, _cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let state = app.hooks_state();
    let hooks = app.flattened_hooks();
    let workspace_count = app.hooks().len();
    let warning_count: usize = app.hooks().iter().map(|entry| entry.warnings.len()).sum();
    let error_count: usize = app.hooks().iter().map(|entry| entry.errors.len()).sum();
    let status = match state {
        SurfaceDataState::Live => "Live Codex catalog",
        SurfaceDataState::Fixture => "Explicit fixture · no hook catalog",
        SurfaceDataState::Loading => "Loading",
        SurfaceDataState::Unsupported => "Unsupported by active backend",
        SurfaceDataState::Error => "Hook catalog error",
    };
    div()
        .id("settings-hooks")
        .flex()
        .flex_col()
        .gap(px(22.0))
        .max_w(px(720.0))
        .child(group_label("Hooks"))
        .child(
            settings_card()
                .child(info_row("Source", "hooks/list"))
                .child(card_divider())
                .child(info_row("Status", status))
                .child(card_divider())
                .child(info_row("Workspace results", &workspace_count.to_string()))
                .child(card_divider())
                .child(info_row(
                    "Diagnostics",
                    &format!("{warning_count} warning(s) · {error_count} error(s)"),
                )),
        )
        .child(group_label("Configured hooks"))
        .child(
            settings_card()
                .children(hooks.iter().enumerate().flat_map(|(index, hook)| {
                    let mut rows = Vec::new();
                    if index > 0 {
                        rows.push(card_divider().into_any_element());
                    }
                    rows.push(hook_row(hook).into_any_element());
                    rows
                }))
                .when(hooks.is_empty(), |this| {
                    this.child(match state {
                        SurfaceDataState::Unsupported => empty_list_message(
                            "Hooks unavailable",
                            "The active backend does not expose a lifecycle hook catalog.",
                        ),
                        SurfaceDataState::Error => empty_list_message(
                            "Could not load hooks",
                            "The Codex hook catalog request failed; reconnect to retry.",
                        ),
                        SurfaceDataState::Loading => {
                            empty_list_message("Loading hooks", "Reading the active workspace…")
                        }
                        _ => empty_list_message(
                            "No hooks found",
                            "The backend returned an empty catalog for this workspace.",
                        ),
                    })
                }),
        )
}

fn hook_row(hook: &HookMetadata) -> impl IntoElement {
    let colors = theme::colors();
    let state = if hook.enabled {
        if hook.is_managed {
            format!("{} · managed", hook.trust_status.label())
        } else {
            hook.trust_status.label().to_owned()
        }
    } else {
        "disabled".to_owned()
    };
    let detail = format!(
        "{} · {} · {} · {}",
        hook.event_name.label(),
        hook.handler_type.label(),
        hook.source.label(),
        hook.source_path
    );
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(16.0))
        .px(px(14.0))
        .py(px(12.0))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(3.0))
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text)
                        .child(hook.key.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .overflow_hidden()
                        .child(detail),
                ),
        )
        .child(
            div()
                .px(px(9.0))
                .py(px(4.0))
                .rounded(px(999.0))
                .bg(colors.bg_button_secondary)
                .text_xs()
                .text_color(if hook.enabled {
                    colors.text_secondary
                } else {
                    colors.text_tertiary
                })
                .child(state),
        )
}

fn connections_body(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let chip = app.connection().chip_label().to_string();
    let detail = app
        .connection()
        .detail()
        .unwrap_or("No transport detail available")
        .to_string();
    let active = app.active_backend_kind();
    let servers: Vec<_> = app.mcp_servers().to_vec();
    const CONNECTOR_PREVIEW_LIMIT: usize = 16;
    let connector_total = app.connector_apps().len();
    let connector_apps: Vec<_> = app
        .connector_apps()
        .iter()
        .take(CONNECTOR_PREVIEW_LIMIT)
        .cloned()
        .map(|connector| {
            let installed = app.installed_app(&connector.id).cloned();
            (connector, installed)
        })
        .collect();
    let connector_state = app.connector_apps_state();
    let connector_status = match connector_state {
        SurfaceDataState::Live => format!(
            "Live Codex catalog · {} available · {} installed",
            connector_total,
            app.installed_apps_count()
        ),
        SurfaceDataState::Fixture => "Explicit fixture · no connector catalog".to_owned(),
        SurfaceDataState::Loading => "Loading connector catalog".to_owned(),
        SurfaceDataState::Unsupported => "Unsupported by active backend".to_owned(),
        SurfaceDataState::Error => "Connector catalog error".to_owned(),
    };
    div()
        .id("settings-connections")
        .flex()
        .flex_col()
        .gap(px(22.0))
        .max_w(px(720.0))
        .child(group_label("Agent backend"))
        .child(
            settings_card()
                .child(backend_choice_row(
                    BackendKind::MitsuroHttp,
                    "Mitsuro server",
                    "Sessions and streamed turns over HTTP/SSE; Hive, schedules, and process catalogs are currently read-only",
                    active,
                    app.connection(),
                    cx,
                ))
                .child(card_divider())
                .child(backend_choice_row(
                    BackendKind::CodexStdio,
                    "ChatGPT / Codex",
                    "Managed local Codex app-server process over stdio",
                    active,
                    app.connection(),
                    cx,
                )),
        )
        .child(group_label("Connection"))
        .child(
            settings_card()
                .child(info_row("Status", &chip))
                .child(card_divider())
                .child(info_row(
                    "Active transport",
                    active
                        .map(MitsuroApp::backend_display_name)
                        .unwrap_or("None"),
                ))
                .child(card_divider())
                .child(info_row("Detail", &detail))
                .child(card_divider())
                .child(reconnect_backend_row(cx)),
        )
        .child(group_label("MCP servers"))
        .child(
            settings_card()
                .children(
                    servers
                        .iter()
                        .enumerate()
                        .flat_map(|(i, s)| {
                            let mut rows = Vec::new();
                            if i > 0 {
                                rows.push(card_divider().into_any_element());
                            }
                            rows.push(mcp_server_row(s, app, cx).into_any_element());
                            rows
                        })
                        .collect::<Vec<_>>(),
                )
                .when(servers.is_empty(), |this| {
                    this.child(empty_list_message(
                        "No MCP servers",
                        "Connect a custom MCP server to list tools here.",
                    ))
                })
                .child(card_divider())
                .child(mcp_add_form(app, cx)),
        )
        .child(group_label("Apps and connectors"))
        .child(
            settings_card()
                .child(info_row("Source", "app/list · app/installed"))
                .child(card_divider())
                .child(info_row("Status", &connector_status))
                .child(card_divider())
                .children(connector_apps.iter().enumerate().flat_map(
                    |(index, (connector, installed))| {
                        let mut rows = Vec::new();
                        if index > 0 {
                            rows.push(card_divider().into_any_element());
                        }
                        rows.push(
                            connector_app_row(connector, installed.as_ref(), cx)
                                .into_any_element(),
                        );
                        rows
                    },
                ))
                .when(connector_total > connector_apps.len(), |this| {
                    this.child(card_divider()).child(info_row(
                        "Catalog preview",
                        &format!(
                            "Showing first {} of {} server-returned apps",
                            connector_apps.len(),
                            connector_total
                        ),
                    ))
                })
                .when(connector_apps.is_empty(), |this| {
                    this.child(match connector_state {
                        SurfaceDataState::Unsupported => empty_list_message(
                            "Apps unavailable",
                            "The active backend does not expose the Codex app and connector catalog.",
                        ),
                        SurfaceDataState::Error => empty_list_message(
                            "Could not load apps",
                            "The Codex app catalog request failed; reconnect to retry.",
                        ),
                        SurfaceDataState::Loading => empty_list_message(
                            "Loading apps",
                            "Reading the live Codex connector catalog…",
                        ),
                        SurfaceDataState::Fixture => empty_list_message(
                            "No fixture apps",
                            "Fixture mode does not invent connector account state.",
                        ),
                        SurfaceDataState::Live => empty_list_message(
                            "No apps available",
                            "Codex returned an empty app catalog for this account.",
                        ),
                    })
                }),
        )
}

fn mcp_add_form(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let available = app.mcp_add_available();
    let busy = app.mcp_add_in_progress();
    let transport = app.mcp_add_transport();
    let name_input = app.mcp_add_name_input().clone();
    let target_input = app.mcp_add_target_input().clone();
    let args_input = app.mcp_add_args_input().clone();

    div()
        .id("settings-mcp-add")
        .flex()
        .flex_col()
        .gap(px(10.0))
        .px(px(14.0))
        .py(px(14.0))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(12.0))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(3.0))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(colors.text)
                                .child("Add server"),
                        )
                        .child(div().text_xs().text_color(colors.text_tertiary).child(
                            if available {
                                "Writes the Codex user config, then reloads MCP servers"
                            } else {
                                "MCP configuration writes are unavailable for this backend"
                            },
                        )),
                )
                .when(available, |this| {
                    this.child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(2.0))
                            .p(px(2.0))
                            .rounded(px(8.0))
                            .bg(colors.bg_button_secondary)
                            .children([McpAddTransport::Http, McpAddTransport::Stdio].map(
                                |option| {
                                    let selected = option == transport;
                                    div()
                                        .id(SharedId(format!(
                                            "mcp-add-transport-{}",
                                            option.label().to_ascii_lowercase()
                                        )))
                                        .px(px(10.0))
                                        .py(px(4.0))
                                        .rounded(px(6.0))
                                        .cursor_pointer()
                                        .when(selected, |style| style.bg(colors.bg_elevated))
                                        .on_click(cx.listener(move |app, _, window, cx| {
                                            app.set_mcp_add_transport(option, window, cx);
                                        }))
                                        .child(
                                            div()
                                                .text_xs()
                                                .font_weight(if selected {
                                                    gpui::FontWeight::SEMIBOLD
                                                } else {
                                                    gpui::FontWeight::NORMAL
                                                })
                                                .text_color(if selected {
                                                    colors.text
                                                } else {
                                                    colors.text_tertiary
                                                })
                                                .child(option.label()),
                                        )
                                },
                            )),
                    )
                }),
        )
        .when(available, |this| {
            this.child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(8.0))
                    .child(mcp_add_input("mcp-add-name", name_input))
                    .child(mcp_add_input("mcp-add-target", target_input)),
            )
            .when(transport == McpAddTransport::Stdio, |this| {
                this.child(mcp_add_input("mcp-add-args", args_input))
            })
            .child(
                div().flex().flex_row().justify_end().child(
                    div()
                        .id("mcp-add-submit")
                        .h(px(32.0))
                        .px(px(14.0))
                        .rounded(px(8.0))
                        .bg(colors.accent_soft)
                        .border_1()
                        .border_color(colors.border)
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(!busy, |button| {
                            button
                                .cursor_pointer()
                                .hover(|style| style.bg(colors.bg_hover))
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.add_mcp_server(cx);
                                }))
                        })
                        .when(busy, |button| button.opacity(0.55))
                        .child(
                            div()
                                .text_xs()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.accent)
                                .child(if busy { "Adding…" } else { "Add server" }),
                        ),
                ),
            )
        })
}

fn mcp_add_input(
    id: &'static str,
    input: gpui::Entity<gpui_component::input::InputState>,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id(id)
        .flex()
        .flex_1()
        .min_w_0()
        .h(px(34.0))
        .px(px(10.0))
        .rounded(px(8.0))
        .bg(colors.bg_sidebar)
        .border_1()
        .border_color(colors.border)
        .child(Input::new(&input).appearance(false).h(px(30.0)))
}

fn backend_choice_row(
    kind: BackendKind,
    title: &str,
    subtitle: &str,
    active: Option<BackendKind>,
    connection: &UiConnection,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let selected = active == Some(kind);
    let state = if selected {
        connection.chip_label()
    } else {
        "Switch"
    };
    let title = title.to_owned();
    let subtitle = subtitle.to_owned();
    div()
        .id(SharedId(format!("backend-choice-{}", kind.id())))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(16.0))
        .px(px(14.0))
        .py(px(13.0))
        .cursor_pointer()
        .hover(|style| style.bg(colors.bg_hover))
        .on_click(cx.listener(move |app, _, _, cx| app.switch_backend(kind, cx)))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(3.0))
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text)
                        .child(title),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child(subtitle),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .px(px(9.0))
                .py(px(4.0))
                .rounded(px(999.0))
                .bg(if selected {
                    theme::hex_alpha(0x3b82f6, 0.16)
                } else {
                    colors.bg_button_secondary
                })
                .text_xs()
                .font_weight(if selected {
                    gpui::FontWeight::SEMIBOLD
                } else {
                    gpui::FontWeight::NORMAL
                })
                .text_color(if selected {
                    colors.accent
                } else {
                    colors.text_secondary
                })
                .child(state),
        )
}

fn reconnect_backend_row(cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("reconnect-backend")
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(16.0))
        .px(px(14.0))
        .py(px(12.0))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(3.0))
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text)
                        .child("Reconnect"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child("Restart the selected transport and reload its catalogs"),
                ),
        )
        .child(
            div()
                .id("reconnect-backend-button")
                .h(px(28.0))
                .px(px(12.0))
                .rounded(px(8.0))
                .bg(colors.bg_button_secondary)
                .border_1()
                .border_color(colors.border)
                .cursor_pointer()
                .hover(|style| style.bg(colors.bg_hover))
                .flex()
                .items_center()
                .justify_center()
                .on_click(cx.listener(|app, _, _, cx| app.reconnect_backend(cx)))
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text_secondary)
                        .child("Reconnect"),
                ),
        )
}

fn unavailable_action_row(title: &str, subtitle: &str, label: &str) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(16.0))
        .px(px(14.0))
        .py(px(12.0))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(3.0))
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text)
                        .child(title.to_owned()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child(subtitle.to_owned()),
                ),
        )
        .child(
            div()
                .px(px(9.0))
                .py(px(4.0))
                .rounded(px(999.0))
                .bg(colors.bg_button_secondary)
                .text_xs()
                .text_color(colors.text_tertiary)
                .child(label.to_owned()),
        )
}

fn git_body(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    div()
        .id("settings-git")
        .flex()
        .flex_col()
        .gap(px(22.0))
        .max_w(px(720.0))
        .child(group_label("Git"))
        .child(
            settings_card()
                .child(select_row(
                    "Default branch",
                    "Preferred base branch for new work",
                    "git_default_branch",
                    &["main", "master", "develop"],
                    "main",
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(toggle_row(
                    "Sign commits",
                    "GPG/SSH-sign commits created by Mitsuro when credentials are available",
                    "git_sign_commits",
                    false,
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(toggle_row(
                    "Auto-stage agent edits",
                    "Stage files Mitsuro touches after a successful turn",
                    "git_auto_stage",
                    false,
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(toggle_row(
                    "Always force push",
                    "Use --force-with-lease when pushing from Mitsuro",
                    "git_force_push",
                    false,
                    app,
                    cx,
                )),
        )
        .child(group_label("Pull requests"))
        .child(
            settings_card()
                .child(toggle_row(
                    "Show PR helper",
                    "Offer pull-request drafts from the home sidebar",
                    "git_pr_helper",
                    true,
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(segment_row(
                    "Pull request merge method",
                    "Choose how Mitsuro merges pull requests",
                    "git_pr_merge",
                    &["Merge", "Squash"],
                    "Squash",
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(action_row(
                    "Pull request instructions",
                    "Added to PR title/description generation prompts",
                    "Edit",
                    "git-pr-instructions",
                    cx,
                )),
        )
}

fn environments_body(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    // Settings Environments page shows empty + create (bar cloud/local empty chrome).
    // Live catalog still available on Computer surface.
    let _catalog_n = app.environments().len();
    div()
        .id("settings-environments")
        .flex()
        .flex_col()
        .gap(px(22.0))
        .max_w(px(720.0))
        .child(group_label("Environments"))
        .child(
            settings_card()
                .child(empty_list_message(
                    "No environments yet",
                    "No local environment is configured for this project yet. Create one to pin setup scripts and secrets.",
                ))
                .child(card_divider())
                .child(action_row(
                    "Create environment",
                    "Add a local or cloud environment for this workspace",
                    "Create",
                    "env-create",
                    cx,
                ))
                .child(card_divider())
                .child(toggle_row(
                    "Prefer local environment",
                    "Use the local host before remote sandboxes",
                    "env_prefer_local",
                    true,
                    app,
                    cx,
                )),
        )
}

fn worktrees_body(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    div()
        .id("settings-worktrees")
        .flex()
        .flex_col()
        .gap(px(22.0))
        .max_w(px(720.0))
        .child(group_label("Worktrees"))
        .child(
            settings_card()
                .child(toggle_row(
                    "Enable worktrees",
                    "Let Mitsuro create isolated git worktrees for parallel work",
                    "worktrees_enabled",
                    true,
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(info_row("Root", "~/.mitsuro/worktrees"))
                .child(card_divider())
                .child(toggle_row(
                    "Automatically delete old worktrees",
                    "Recommended for most users. Turn this off only if you manage disk usage yourself.",
                    "worktrees_auto_prune",
                    true,
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(select_row(
                    "Auto-delete limit",
                    "Number of managed worktrees to keep before older ones are pruned",
                    "worktree_keep_count",
                    &["3", "5", "10", "20"],
                    "5",
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(select_row(
                    "Default strategy",
                    "How Mitsuro creates isolated workspaces",
                    "worktree_strategy",
                    &["Git worktree", "Copy", "None"],
                    "Git worktree",
                    app,
                    cx,
                )),
        )
        .child(group_label("Managed worktrees"))
        .child(
            settings_card().child(empty_list_message(
                "No worktrees yet",
                "Worktrees created by Mitsuro will appear here",
            )),
        )
}

fn archived_body(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let archived_n = app
        .threads()
        .iter()
        .filter(|t| t.summary.archived.unwrap_or(false))
        .count();
    div()
        .id("settings-archived")
        .flex()
        .flex_col()
        .gap(px(22.0))
        .max_w(px(720.0))
        .child(group_label("Archived chats"))
        .child(
            settings_card()
                .child(if archived_n == 0 {
                    empty_list_message(
                        "No archived chats",
                        "Chats you archive will appear here. They stay hidden from Recents by default.",
                    )
                    .into_any_element()
                } else {
                    info_row(
                        "Archived",
                        &format!("{archived_n} conversation(s)"),
                    )
                    .into_any_element()
                })
                .child(card_divider())
                .child(toggle_row(
                    "Show archived in Recents",
                    "Include archived threads in the home sidebar list",
                    "archived_show_in_recents",
                    false,
                    app,
                    cx,
                ))
                .child(card_divider())
                .child(action_row(
                    "Empty archive",
                    "Permanently delete archived chats",
                    "Empty",
                    "empty-archive",
                    cx,
                )),
        )
}

fn import_source_card(
    title: &str,
    subtitle: &str,
    _button: &str,
    id: &'static str,
    _cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let title = title.to_string();
    let subtitle = subtitle.to_string();
    div()
        .id(SharedId(format!("import-card-{id}")))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(16.0))
        .px(px(16.0))
        .py(px(14.0))
        .rounded(px(12.0))
        .bg(colors.bg_elevated)
        .border_1()
        .border_color(colors.border)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child(title),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child(subtitle),
                ),
        )
        .child(
            div()
                .id(SharedId(format!("import-btn-{id}")))
                .h(px(30.0))
                .px(px(14.0))
                .rounded(px(8.0))
                .bg(colors.bg_button_secondary)
                .border_1()
                .border_color(colors.border)
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text_tertiary)
                        .child("Unavailable"),
                ),
        )
}

fn empty_list_message(title: &str, subtitle: &str) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(6.0))
        .px(px(16.0))
        .py(px(28.0))
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(colors.text_secondary)
                .child(title.to_string()),
        )
        .child(
            div()
                .text_xs()
                .text_color(colors.text_tertiary)
                .text_center()
                .max_w(px(420.0))
                .child(subtitle.to_string()),
        )
}

fn mcp_server_row(
    server: &mitsuro_desktop_backend::McpServerStatus,
    app: &MitsuroApp,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let title = server.display_title().to_string();
    let status = server.status_label();
    let name = server.name.clone();
    let action_id = SharedId(format!("mcp-auth-{}", server.name));
    let pending = app.mcp_oauth_pending(&server.name);
    let can_login = app.mcp_oauth_available()
        && server.auth_status == mitsuro_desktop_backend::McpAuthStatus::NotLoggedIn;
    let server_for_login = server.clone();
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(12.0))
        .px(px(14.0))
        .py(px(12.0))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text)
                        .child(title),
                )
                .child(
                    div()
                        .text_xs()
                        .font_family("monospace")
                        .text_color(colors.text_tertiary)
                        .child(name),
                ),
        )
        .child(
            div()
                .id(action_id)
                .px(px(8.0))
                .py(px(3.0))
                .rounded(px(999.0))
                .bg(colors.bg_button_secondary)
                .text_xs()
                .text_color(colors.text_secondary)
                .when(can_login && !pending, |this| {
                    this.cursor_pointer()
                        .hover(|style| style.bg(colors.bg_hover))
                        .on_click(cx.listener(move |app, _, _, cx| {
                            app.start_mcp_oauth(server_for_login.clone(), cx);
                        }))
                })
                .when(pending, |this| this.opacity(0.6))
                .child(if pending {
                    "Waiting…".to_owned()
                } else if can_login {
                    "Sign in".to_owned()
                } else {
                    status
                }),
        )
}

fn connector_app_row(
    connector: &AppInfo,
    installed: Option<&InstalledApp>,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let title = connector.name.clone();
    let id = connector.id.clone();
    let detail = connector
        .description
        .clone()
        .or_else(|| connector.category())
        .or_else(|| connector.distribution_channel.clone())
        .unwrap_or_else(|| "No description provided".to_owned());
    let (label, can_connect) = if !connector.is_enabled {
        ("Disabled", false)
    } else if installed.is_some_and(|app| app.callable) {
        ("Connected", false)
    } else if installed.is_some_and(|app| app.enabled) {
        ("Installed", false)
    } else if connector.install_url.is_some() {
        ("Connect", true)
    } else if connector.is_accessible {
        ("Accessible", false)
    } else {
        ("Unavailable", false)
    };
    let connector_for_action = connector.clone();

    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(12.0))
        .px(px(14.0))
        .py(px(12.0))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text)
                        .child(title),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .overflow_hidden()
                        .child(detail),
                ),
        )
        .child(
            div()
                .id(SharedId(format!("connector-app-{id}")))
                .px(px(9.0))
                .py(px(4.0))
                .rounded(px(999.0))
                .bg(colors.bg_button_secondary)
                .text_xs()
                .text_color(if can_connect {
                    colors.accent
                } else {
                    colors.text_secondary
                })
                .when(can_connect, |this| {
                    this.cursor_pointer()
                        .hover(|style| style.bg(colors.bg_hover))
                        .on_click(cx.listener(move |app, _, _, cx| {
                            app.open_connector_install(connector_for_action.clone(), cx);
                        }))
                })
                .child(label),
        )
}

// ─── Shared chrome ──────────────────────────────────────────────────────────

fn group_label(title: &str) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .text_xs()
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(colors.text_tertiary)
        .child(title.to_string())
}

fn settings_card() -> gpui::Div {
    let colors = theme::colors();
    div()
        .flex()
        .flex_col()
        .rounded(px(12.0))
        .bg(colors.bg_elevated)
        .border_1()
        .border_color(colors.border)
        .overflow_hidden()
}

fn card_divider() -> impl IntoElement {
    let colors = theme::colors();
    div().h(px(1.0)).w_full().bg(colors.border)
}

fn toggle_row(
    title: &str,
    subtitle: &str,
    key: &'static str,
    default: bool,
    app: &MitsuroApp,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let on = app.settings_toggle(key, default);
    let title = title.to_string();
    let subtitle = subtitle.to_string();
    let row_id = format!("toggle-{key}");
    div()
        .id(SharedId(row_id))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(16.0))
        .px(px(14.0))
        .py(px(12.0))
        .cursor_pointer()
        .hover(|s| s.bg(colors.bg_hover))
        .on_click(cx.listener(move |app, _, _, cx| {
            app.flip_settings_toggle(key, default, cx);
        }))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(3.0))
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text)
                        .child(title),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child(subtitle),
                ),
        )
        .child(toggle_switch(on))
}

fn toggle_switch(on: bool) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .flex()
        .flex_row()
        .items_center()
        .w(px(40.0))
        .h(px(22.0))
        .rounded(px(999.0))
        .px(px(2.0))
        .flex_shrink_0()
        .bg(if on {
            colors.accent
        } else {
            theme::hex_alpha(0xffffff, 0.14)
        })
        .justify_end()
        .when(!on, |s| s.justify_start())
        .child(div().size(px(18.0)).rounded_full().bg(theme::hex(0xffffff)))
}

fn select_row(
    title: &str,
    subtitle: &str,
    key: &'static str,
    options: &'static [&'static str],
    default: &'static str,
    app: &MitsuroApp,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let current = app.settings_choice(key, default);
    let title = title.to_string();
    let subtitle = subtitle.to_string();
    let label = current;
    let row_id = format!("select-{key}");
    // Cycle on click through options
    let opts: Vec<&'static str> = options.to_vec();
    div()
        .id(SharedId(row_id))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(16.0))
        .px(px(14.0))
        .py(px(12.0))
        .cursor_pointer()
        .hover(|s| s.bg(colors.bg_hover))
        .on_click(cx.listener(move |app, _, _, cx| {
            let cur = app.settings_choice(key, default);
            let idx = opts.iter().position(|o| *o == cur.as_str()).unwrap_or(0);
            let next = opts[(idx + 1) % opts.len()];
            app.set_settings_choice(key, next, cx);
        }))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(3.0))
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text)
                        .child(title),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child(subtitle),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(4.0))
                .h(px(28.0))
                .px(px(10.0))
                .rounded(px(8.0))
                .bg(colors.bg_button_secondary)
                .border_1()
                .border_color(colors.border)
                // Bar Default file open destination: folder glyph + value ▾.
                .when(key == "file_open_dest", |s| {
                    s.child(
                        Icon::empty()
                            .path("icons/folder.svg")
                            .with_size(px(12.0))
                            .text_color(colors.text_tertiary),
                    )
                })
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_secondary)
                        .child(label),
                )
                .child(
                    Icon::new(IconName::ChevronDown)
                        .with_size(px(12.0))
                        .text_color(colors.text_tertiary),
                ),
        )
}

fn segment_row(
    title: &str,
    subtitle: &str,
    key: &'static str,
    options: &'static [&'static str],
    default: &'static str,
    app: &MitsuroApp,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let current = app.settings_choice(key, default);
    let title = title.to_string();
    let subtitle = subtitle.to_string();
    let row_id = format!("segment-{key}");
    div()
        .id(SharedId(row_id))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(16.0))
        .px(px(14.0))
        .py(px(12.0))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(3.0))
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text)
                        .child(title),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child(subtitle),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(2.0))
                .p(px(2.0))
                .rounded(px(8.0))
                .bg(colors.bg_button_secondary)
                .children(
                    options
                        .iter()
                        .map(|opt| {
                            let selected = current.as_str() == *opt;
                            let value = (*opt).to_string();
                            let opt_id = format!("seg-{key}-{opt}");
                            div()
                                .id(SharedId(opt_id))
                                .px(px(10.0))
                                .py(px(4.0))
                                .rounded(px(6.0))
                                .cursor_pointer()
                                .when(selected, |s| s.bg(colors.bg_elevated))
                                .on_click(cx.listener(move |app, _, _, cx| {
                                    app.set_settings_choice(key, value.clone(), cx);
                                }))
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(if selected {
                                            gpui::FontWeight::SEMIBOLD
                                        } else {
                                            gpui::FontWeight::NORMAL
                                        })
                                        .text_color(if selected {
                                            colors.text
                                        } else {
                                            colors.text_tertiary
                                        })
                                        .child((*opt).to_string()),
                                )
                        })
                        .collect::<Vec<_>>(),
                ),
        )
}

fn action_row(
    title: &str,
    subtitle: &str,
    button: &str,
    id: &'static str,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let title = title.to_string();
    let subtitle = subtitle.to_string();
    let button = button.to_string();
    let wired = matches!(
        id,
        "open-plugins" | "plugins-refresh" | "plugins-open-connections" | "env-create"
    );
    let row_id = format!("action-{id}");
    div()
        .id(SharedId(row_id))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(16.0))
        .px(px(14.0))
        .py(px(12.0))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(3.0))
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text)
                        .child(title),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child(subtitle),
                ),
        )
        .child(
            div()
                .id(SharedId(format!("action-btn-{id}")))
                .h(px(28.0))
                .px(px(12.0))
                .rounded(px(8.0))
                .bg(colors.bg_button_secondary)
                .border_1()
                .border_color(colors.border)
                .flex()
                .items_center()
                .justify_center()
                .when(wired, |this| {
                    this.cursor_pointer()
                        .hover(|style| style.bg(colors.bg_hover))
                        .on_click(cx.listener(move |app, _, window, cx| match id {
                            "open-plugins" => app.set_mode(ProductMode::Extensions, window, cx),
                            "plugins-refresh" => app.refresh_extensions(window, cx),
                            "plugins-open-connections" => {
                                app.set_settings_section(SettingsSection::Connections, cx)
                            }
                            "env-create" => app.set_mode(ProductMode::Computer, window, cx),
                            _ => {}
                        }))
                })
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(if wired {
                            colors.text_secondary
                        } else {
                            colors.text_tertiary
                        })
                        .child(if wired {
                            button
                        } else {
                            "Not wired".to_owned()
                        }),
                ),
        )
}

/// Full access row with bar-matching mid-sentence blue "Learn more" link.
///
/// Bar / reverse body is one flowing tertiary paragraph ending with
/// `<a>Learn more</a> about elevated risks.` (only "Learn more" is link-colored).
/// Uses `StyledText` + `whitespace_normal` + `min_w_0`/`w_full` so wrap width is
/// definite under the flex text column (avoids unbounded measure under the toggle).
fn full_access_row(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let on = app.settings_toggle("full_access", true);
    // Single flowing paragraph; only "Learn more" is accent-colored.
    const BODY: &str = "Saved locally only. The GPUI client does not currently change backend sandbox or approval policy from this control. Full access can permit file, command, and network mutations when a backend explicitly supports it. Learn more about elevated risks.";
    const LINK: &str = "Learn more";
    let link_start = BODY.find(LINK).expect("Learn more in full-access body");
    let link_end = link_start + LINK.len();
    let accent = colors.accent;
    div()
        .id("toggle-full_access")
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(16.0))
        .px(px(14.0))
        .py(px(12.0))
        .cursor_pointer()
        .hover(|s| s.bg(colors.bg_hover))
        .on_click(cx.listener(|app, _, _, cx| {
            app.flip_settings_toggle("full_access", true, cx);
        }))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(3.0))
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text)
                        .child("Full access".to_string()),
                )
                .child(
                    // whitespace_normal + min_w_0 + w_full: definite wrap width inside flex_1 column.
                    div()
                        .min_w_0()
                        .w_full()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .whitespace_normal()
                        .child(StyledText::new(BODY).with_highlights([(
                            link_start..link_end,
                            HighlightStyle {
                                color: Some(accent),
                                ..Default::default()
                            },
                        )])),
                ),
        )
        .child(toggle_switch(on))
}

/// Hotkey capture row matching bar Popout chrome: bare "Off" + pen icon (not a heavy button).
fn hotkey_row(
    title: &str,
    subtitle: &str,
    value: &str,
    id: &'static str,
    _cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let title = title.to_string();
    let subtitle = subtitle.to_string();
    let value = value.to_string();
    let row_id = format!("hotkey-{id}");
    div()
        .id(SharedId(row_id))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(16.0))
        .px(px(14.0))
        .py(px(12.0))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(3.0))
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text)
                        .child(title),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child(subtitle),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(
                    div()
                        .text_sm()
                        .text_color(colors.text_secondary)
                        .child(value),
                )
                .child(
                    Icon::empty()
                        .path("icons/pen-line.svg")
                        .with_size(px(14.0))
                        .text_color(colors.status_offline),
                ),
        )
}

/// Appearance color chip row (Background / Foreground style density).
fn color_chip_row(
    title: &str,
    subtitle: &str,
    key: &'static str,
    default_hex: u32,
    app: &MitsuroApp,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let label = app.settings_choice(key, &format!("#{default_hex:06x}"));
    let title = title.to_string();
    let subtitle = subtitle.to_string();
    let row_id = format!("color-{key}");
    // Cycle through a small palette on click (fixture-only).
    const PALETTE: &[&str] = &[
        "#0d0d0d", "#1a1a1a", "#e8e8e8", "#f5f5f5", "#0f172a", "#111827",
    ];
    div()
        .id(SharedId(row_id))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(16.0))
        .px(px(14.0))
        .py(px(12.0))
        .cursor_pointer()
        .hover(|s| s.bg(colors.bg_hover))
        .on_click(cx.listener(move |app, _, _, cx| {
            let cur = app.settings_choice(key, &format!("#{default_hex:06x}"));
            let idx = PALETTE.iter().position(|c| *c == cur.as_str()).unwrap_or(0);
            let next = PALETTE[(idx + 1) % PALETTE.len()];
            app.set_settings_choice(key, next, cx);
        }))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(3.0))
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text)
                        .child(title),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child(subtitle),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .size(px(22.0))
                        .rounded_full()
                        .border_1()
                        .border_color(theme::hex_alpha(0xffffff, 0.18))
                        .bg(theme::hex(parse_hex_color(&label).unwrap_or(default_hex))),
                )
                .child(
                    div()
                        .text_xs()
                        .font_family("monospace")
                        .text_color(colors.text_secondary)
                        .child(label),
                ),
        )
}

fn parse_hex_color(s: &str) -> Option<u32> {
    let t = s.trim().trim_start_matches('#');
    if t.len() != 6 {
        return None;
    }
    u32::from_str_radix(t, 16).ok()
}

fn info_row(title: &str, value: &str) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(16.0))
        .px(px(14.0))
        .py(px(12.0))
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(colors.text)
                .child(title.to_string()),
        )
        .child(
            div()
                .text_xs()
                .text_color(colors.text_tertiary)
                .font_family("monospace")
                .child(value.to_string()),
        )
}

fn shortcut_row(title: &str, keys: &str) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(16.0))
        .px(px(14.0))
        .py(px(12.0))
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(colors.text)
                .child(title.to_string()),
        )
        .child(
            div()
                .px(px(8.0))
                .py(px(3.0))
                .rounded(px(6.0))
                .bg(colors.bg_button_secondary)
                .border_1()
                .border_color(colors.border)
                .text_xs()
                .font_family("monospace")
                .text_color(colors.text_secondary)
                .child(keys.to_string()),
        )
}

// ─── Account (reused chrome) ────────────────────────────────────────────────

fn account_section(
    account: &AccountSession,
    status_label: &str,
    conn: &UiConnection,
    profile_name: &str,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let email = account.email_display.clone().unwrap_or_else(|| "—".into());
    let plan = account.plan_label.clone().unwrap_or_else(|| "—".into());
    let primary_pct = account.primary_used_percent().clamp(0, 100) as f32;
    let secondary_pct = account.secondary_used_percent().clamp(0, 100) as f32;
    let lifetime = account.lifetime_tokens();
    let lifetime_label = format_token_count(lifetime);
    let login_detail = account.login_detail.clone();
    let login_pending = account.pending_login_id.is_some();
    let login_url_available = account.pending_login_url.is_some();
    let signed_in = account.signed_in;
    let source = account.source;
    let profile = profile_name.to_string();
    let profile_initial = profile
        .chars()
        .find(|character| character.is_alphanumeric())
        .map(|character| character.to_uppercase().to_string())
        .unwrap_or_else(|| "M".to_owned());

    div()
        .id("account-section")
        .flex()
        .flex_col()
        .gap(px(12.0))
        .px(px(14.0))
        .py(px(14.0))
        .rounded(px(12.0))
        .bg(colors.bg_elevated)
        .border_1()
        .border_color(colors.border)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(12.0))
                .child(
                    div()
                        .w(px(40.0))
                        .h(px(40.0))
                        .rounded_full()
                        .border_1()
                        .border_color(theme::hex_alpha(0xffe8d8, 0.16))
                        .flex()
                        .items_center()
                        .justify_center()
                        .flex_shrink_0()
                        .text_size(px(14.0))
                        .text_color(colors.text_secondary)
                        .child(profile_initial),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .min_w_0()
                        .flex_1()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.text)
                                .child(profile),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.text_tertiary)
                                .child(format!("{email} · plan {plan}")),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.text_tertiary)
                                .child(format!(
                                    "{status_label} · {source} · {}",
                                    if signed_in { "signed in" } else { "signed out" }
                                )),
                        ),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(usage_bar_row(
                    "Primary rate limit",
                    primary_pct,
                    format!("{primary_pct:.0}% used"),
                ))
                .child(usage_bar_row(
                    "Secondary rate limit",
                    secondary_pct,
                    format!("{secondary_pct:.0}% used"),
                ))
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child(format!("Lifetime tokens · {lifetime_label}")),
                ),
        )
        .when_some(login_detail, |this, detail| {
            this.child(
                div()
                    .px(px(10.0))
                    .py(px(8.0))
                    .rounded(px(8.0))
                    .bg(colors.bg_button_secondary)
                    .text_xs()
                    .font_family("monospace")
                    .text_color(colors.text_secondary)
                    .child(detail),
            )
        })
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .when(!signed_in && !login_pending, |this| {
                    this.child(account_action_button(
                        "account-sign-in",
                        "Sign in",
                        true,
                        cx,
                        |app, window, cx| app.account_sign_in(window, cx),
                    ))
                })
                .when(login_pending && login_url_available, |this| {
                    this.child(account_action_button(
                        "account-open-sign-in",
                        "Open sign-in",
                        true,
                        cx,
                        |app, _window, cx| app.account_open_sign_in(cx),
                    ))
                })
                .when(login_pending, |this| {
                    this.child(account_action_button(
                        "account-cancel-sign-in",
                        "Cancel",
                        false,
                        cx,
                        |app, _window, cx| app.account_cancel_sign_in(cx),
                    ))
                })
                .when(signed_in, |this| {
                    this.child(account_action_button(
                        "account-sign-out",
                        "Sign out",
                        false,
                        cx,
                        |app, window, cx| app.account_sign_out(window, cx),
                    ))
                })
                .child(account_action_button(
                    "account-refresh",
                    "Refresh",
                    false,
                    cx,
                    |app, window, cx| app.refresh_account(window, cx),
                )),
        )
        .child(
            div()
                .text_xs()
                .text_color(colors.text_tertiary)
                .child(format!("Connection · {}", chip_label(conn))),
        )
}

fn chip_label(conn: &UiConnection) -> String {
    conn.chip_label().to_string()
}

fn format_token_count(n: i64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.0}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn usage_bar_row(label: &str, used_percent: f32, value: String) -> impl IntoElement {
    let colors = theme::colors();
    let fill = (used_percent / 100.0).clamp(0.0, 1.0);
    div()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_secondary)
                        .child(label.to_string()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child(value),
                ),
        )
        .child(
            div()
                .h(px(6.0))
                .w_full()
                .rounded(px(999.0))
                .bg(colors.bg_button_secondary)
                .overflow_hidden()
                .child(
                    div()
                        .h_full()
                        .w(px((fill * 280.0).max(2.0)))
                        .rounded(px(999.0))
                        .bg(if used_percent >= 90.0 {
                            colors.status_error
                        } else if used_percent >= 70.0 {
                            colors.status_connecting
                        } else {
                            colors.accent
                        }),
                ),
        )
}

fn account_action_button(
    id: &'static str,
    label: &'static str,
    primary: bool,
    cx: &mut Context<MitsuroApp>,
    on_click: impl Fn(&mut MitsuroApp, &mut gpui::Window, &mut Context<MitsuroApp>) + 'static,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .justify_center()
        .h(px(30.0))
        .px(px(12.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .when(primary, |s| {
            s.bg(colors.accent_soft)
                .border_1()
                .border_color(colors.border)
                .hover(|s| s.bg(colors.bg_hover))
        })
        .when(!primary, |s| {
            s.bg(colors.bg_button_secondary)
                .border_1()
                .border_color(colors.border)
                .hover(|s| s.bg(colors.bg_hover))
        })
        .on_click(cx.listener(move |app, _, window, cx| {
            on_click(app, window, cx);
        }))
        .child(
            div()
                .text_xs()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(if primary {
                    colors.accent
                } else {
                    colors.text_secondary
                })
                .child(label),
        )
}

/// Stable element id wrapper (avoids lifetime issues with dynamic strings).
struct SharedId(String);

impl From<SharedId> for gpui::ElementId {
    fn from(value: SharedId) -> Self {
        gpui::ElementId::Name(value.0.into())
    }
}
