//! Files panel — `fs/readDirectory` + `fs/readFile` + `fuzzyFileSearch` surface.
//!
//! Offline path uses [`mitsuro_desktop_backend::FixtureBackend`] virtual tree at `/fixture-project`.
//! Codex dark theme, Mitsuro labels.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _,
};
use gpui_component::input::Input;
use gpui_component::{Icon, IconName, Sizable as _};
use mitsuro_desktop_backend::{
    FsReadDirectoryEntry, FuzzyFileSearchMatchType, FuzzyFileSearchResult,
};

use crate::app::MitsuroApp;
use crate::theme;

const DIRECTORY_RENDER_LIMIT: usize = 200;

/// Full-height Files panel: path bar, search, directory list, file preview.
pub fn files_panel(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let session = app.files_session();
    let path_input = app.files_path_input().clone();
    let search_input = app.files_search_input().clone();
    let name_input = app.files_name_input().clone();
    let editor_input = app.files_editor_input().clone();
    let browsing = session.search_query.is_empty();
    let mutations_available = app.files_mutations_available();

    div()
        .id("files-panel")
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .bg(colors.bg_main)
        .child(files_title_bar(
            session.backend_label.as_ref(),
            session.cwd.as_ref(),
        ))
        .child(path_bar(app, &path_input, cx))
        .child(search_bar(app, &search_input, cx))
        .child(mutation_bar(
            &name_input,
            mutations_available,
            session.selected_path.is_some(),
            app.files_delete_pending(),
            cx,
        ))
        .child(
            div()
                .id("files-body")
                .flex()
                .flex_row()
                .flex_1()
                .min_h_0()
                .w_full()
                .child(if browsing {
                    directory_list(app, &session.entries, session.selected_path.as_deref(), cx)
                        .into_any_element()
                } else {
                    fuzzy_list(
                        app,
                        &session.fuzzy_results,
                        session.selected_path.as_deref(),
                        cx,
                    )
                    .into_any_element()
                })
                .child(preview_pane(
                    session.selected_path.as_deref(),
                    session.preview.as_ref(),
                    session.preview_error.as_deref(),
                    &editor_input,
                    mutations_available,
                )),
        )
        .child(status_footer(
            session.cwd.as_ref(),
            browsing,
            session.entries.len(),
            session.fuzzy_results.len(),
            session.selected_path.as_deref(),
        ))
}

fn files_title_bar(backend: &str, cwd: &str) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("files-title")
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .px(px(16.0))
        .py(px(12.0))
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.bg_sidebar)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .child(
                    Icon::new(IconName::FolderOpen)
                        .with_size(px(16.0))
                        .text_color(colors.text),
                )
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child("Files"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child(format!("· {backend}")),
                ),
        )
        .child(
            div()
                .text_xs()
                .px(px(8.0))
                .py(px(3.0))
                .rounded(px(6.0))
                .bg(colors.bg_elevated)
                .border_1()
                .border_color(colors.border)
                .text_color(colors.text_secondary)
                .child(cwd.to_string()),
        )
}

fn path_bar(
    app: &MitsuroApp,
    path_input: &gpui::Entity<gpui_component::input::InputState>,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let cwd = app.files_session().cwd.to_string();
    div()
        .id("files-path-bar")
        .flex()
        .flex_col()
        .gap(px(6.0))
        .px(px(12.0))
        .py(px(10.0))
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.bg_sidebar)
        // Breadcrumb trail
        .child(files_breadcrumb(&cwd, cx))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .id("files-up")
                        .flex()
                        .items_center()
                        .justify_center()
                        .h(px(34.0))
                        .w(px(34.0))
                        .rounded(px(10.0))
                        .bg(colors.bg_elevated)
                        .border_1()
                        .border_color(colors.border)
                        .cursor_pointer()
                        .hover(|s| s.bg(colors.bg_hover))
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.files_go_up(window, cx);
                        }))
                        .child(
                            Icon::new(IconName::ArrowUp)
                                .with_size(px(14.0))
                                .text_color(colors.text_secondary),
                        ),
                )
                .child(
                    div()
                        .id("files-path-input")
                        .flex()
                        .flex_1()
                        .min_w_0()
                        .h(px(34.0))
                        .px(px(12.0))
                        .rounded(px(10.0))
                        .bg(colors.bg_elevated)
                        .border_1()
                        .border_color(colors.border_heavy)
                        .text_sm()
                        .text_color(colors.text)
                        .child(Input::new(path_input).appearance(false).h(px(28.0))),
                )
                .child(
                    div()
                        .id("files-go")
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .h(px(34.0))
                        .px(px(14.0))
                        .rounded(px(10.0))
                        .bg(colors.accent_soft)
                        .border_1()
                        .border_color(colors.border)
                        .cursor_pointer()
                        .hover(|s| s.bg(colors.bg_hover))
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.files_navigate_path_bar(window, cx);
                        }))
                        .child(
                            div()
                                .text_xs()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.accent)
                                .child("Go"),
                        ),
                ),
        )
}

fn files_breadcrumb(cwd: &str, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let parts: Vec<String> = cwd
        .split('/')
        .filter(|p| !p.is_empty())
        .map(|s| s.to_string())
        .collect();
    div()
        .id("files-breadcrumb")
        .flex()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .overflow_hidden()
        .child(
            div()
                .id("files-crumb-root")
                .px(px(6.0))
                .py(px(2.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .hover(|s| s.bg(colors.bg_hover))
                .on_click(cx.listener(|app, _, window, cx| {
                    app.files_navigate_to("/".to_owned(), window, cx);
                }))
                .child(
                    Icon::new(IconName::Folder)
                        .with_size(px(12.0))
                        .text_color(colors.text_tertiary),
                ),
        )
        .children(parts.into_iter().enumerate().map(|(i, part)| {
            let label = part;
            div()
                .id(("files-crumb", i as u64))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(4.0))
                .child(div().text_xs().text_color(colors.text_tertiary).child("/"))
                .child(
                    div()
                        .px(px(6.0))
                        .py(px(2.0))
                        .rounded(px(6.0))
                        .text_xs()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text_secondary)
                        .child(label),
                )
                .into_any_element()
        }))
}

fn mutation_bar(
    name_input: &gpui::Entity<gpui_component::input::InputState>,
    mutations_available: bool,
    has_selection: bool,
    delete_pending: bool,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("files-mutation-bar")
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .px(px(12.0))
        .py(px(8.0))
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.bg_sidebar)
        .child(
            div()
                .id("files-name-input")
                .flex()
                .flex_1()
                .min_w(px(170.0))
                .h(px(32.0))
                .px(px(10.0))
                .rounded(px(9.0))
                .bg(colors.bg_elevated)
                .border_1()
                .border_color(colors.border)
                .text_sm()
                .text_color(colors.text)
                .child(Input::new(name_input).appearance(false).h(px(26.0))),
        )
        .child(file_action_button(
            "files-create-file",
            "New file",
            mutations_available,
            false,
            cx,
            |app, window, cx| app.files_create_file(window, cx),
        ))
        .child(file_action_button(
            "files-create-folder",
            "New folder",
            mutations_available,
            false,
            cx,
            |app, window, cx| app.files_create_directory(window, cx),
        ))
        .child(file_action_button(
            "files-save",
            "Save",
            mutations_available && has_selection,
            false,
            cx,
            |app, window, cx| app.files_save_selected(window, cx),
        ))
        .child(file_action_button(
            "files-copy",
            "Duplicate",
            mutations_available && has_selection,
            false,
            cx,
            |app, window, cx| app.files_duplicate_selected(window, cx),
        ))
        .child(file_action_button(
            "files-delete",
            if delete_pending {
                "Confirm delete"
            } else {
                "Delete"
            },
            mutations_available && has_selection,
            delete_pending,
            cx,
            |app, window, cx| app.files_delete_selected(window, cx),
        ))
        .when(!mutations_available, |this| {
            this.child(
                div()
                    .text_xs()
                    .text_color(colors.text_tertiary)
                    .child("Read-only on the active backend"),
            )
        })
}

fn file_action_button(
    id: &'static str,
    label: &'static str,
    enabled: bool,
    destructive: bool,
    cx: &mut Context<MitsuroApp>,
    on_click: impl Fn(&mut MitsuroApp, &mut gpui::Window, &mut Context<MitsuroApp>) + 'static,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .h(px(32.0))
        .px(px(10.0))
        .rounded(px(9.0))
        .bg(if destructive {
            colors.bg_selected
        } else {
            colors.bg_button_secondary
        })
        .border_1()
        .border_color(colors.border)
        .text_xs()
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(if enabled {
            if destructive {
                colors.status_error
            } else {
                colors.text_secondary
            }
        } else {
            colors.text_tertiary
        })
        .when(enabled, |this| {
            this.cursor_pointer()
                .hover(|style| style.bg(colors.bg_hover))
                .on_click(cx.listener(move |app, _, window, cx| {
                    on_click(app, window, cx);
                }))
        })
        .child(label)
}

fn search_bar(
    _app: &MitsuroApp,
    search_input: &gpui::Entity<gpui_component::input::InputState>,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("files-search-bar")
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .px(px(12.0))
        .py(px(8.0))
        .border_b_1()
        .border_color(colors.border)
        .bg(colors.bg_sidebar)
        .child(
            Icon::new(IconName::Search)
                .with_size(px(14.0))
                .text_color(colors.text_tertiary),
        )
        .child(
            div()
                .id("files-search-input")
                .flex()
                .flex_1()
                .min_w_0()
                .h(px(32.0))
                .px(px(12.0))
                .rounded(px(10.0))
                .bg(colors.bg_elevated)
                .border_1()
                .border_color(colors.border)
                .text_sm()
                .text_color(colors.text)
                .child(Input::new(search_input).appearance(false).h(px(26.0))),
        )
        .child(
            div()
                .id("files-search-run")
                .flex()
                .flex_row()
                .items_center()
                .justify_center()
                .h(px(32.0))
                .px(px(12.0))
                .rounded(px(10.0))
                .bg(colors.bg_button_secondary)
                .border_1()
                .border_color(colors.border)
                .cursor_pointer()
                .hover(|s| s.bg(colors.bg_hover))
                .on_click(cx.listener(|app, _, window, cx| {
                    app.files_run_fuzzy(window, cx);
                }))
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text_secondary)
                        .child("Search"),
                ),
        )
}

fn directory_list(
    _app: &MitsuroApp,
    entries: &[FsReadDirectoryEntry],
    selected: Option<&str>,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("files-dir-list")
        .flex()
        .flex_col()
        .w(px(280.0))
        .min_w(px(220.0))
        .h_full()
        .border_r_1()
        .border_color(colors.border)
        .bg(colors.bg_sidebar)
        .overflow_y_scroll()
        .py(px(6.0))
        .children(if entries.is_empty() {
            vec![div()
                .px(px(12.0))
                .py(px(8.0))
                .text_xs()
                .text_color(colors.text_tertiary)
                .child("Empty directory")
                .into_any_element()]
        } else {
            let mut rows: Vec<_> = entries
                .iter()
                .take(DIRECTORY_RENDER_LIMIT)
                .enumerate()
                .map(|(i, e)| {
                    let name = e.file_name.clone();
                    let is_dir = e.is_directory;
                    let sel = selected
                        .map(|p| p.ends_with(&format!("/{name}")) || p.ends_with(&name))
                        .unwrap_or(false);
                    let name_for_click = name.clone();
                    entry_row(
                        ("files-entry", i),
                        name.as_str(),
                        is_dir,
                        sel,
                        cx,
                        move |app, window, cx| {
                            app.files_activate_entry(name_for_click.clone(), is_dir, window, cx);
                        },
                    )
                    .into_any_element()
                })
                .collect();
            let hidden = entries.len().saturating_sub(DIRECTORY_RENDER_LIMIT);
            if hidden > 0 {
                rows.push(
                    div()
                        .px(px(12.0))
                        .py(px(10.0))
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child(format!(
                            "{hidden} more entries · use fuzzy search to reach them"
                        ))
                        .into_any_element(),
                );
            }
            rows
        })
}

fn fuzzy_list(
    _app: &MitsuroApp,
    results: &[FuzzyFileSearchResult],
    selected: Option<&str>,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("files-fuzzy-list")
        .flex()
        .flex_col()
        .w(px(280.0))
        .min_w(px(220.0))
        .h_full()
        .border_r_1()
        .border_color(colors.border)
        .bg(colors.bg_sidebar)
        .overflow_y_scroll()
        .py(px(6.0))
        .children(if results.is_empty() {
            vec![div()
                .px(px(12.0))
                .py(px(8.0))
                .text_xs()
                .text_color(colors.text_tertiary)
                .child("No fuzzy matches")
                .into_any_element()]
        } else {
            results
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    let path = r.path.clone();
                    let name = r.file_name.clone();
                    let is_dir = matches!(r.match_type, FuzzyFileSearchMatchType::Directory);
                    let sel = selected == Some(path.as_str());
                    let subtitle = r.path.clone();
                    let path_for_click = path;
                    entry_row_with_sub(
                        ("files-fuzzy", i),
                        name.as_str(),
                        subtitle.as_str(),
                        is_dir,
                        r.score,
                        sel,
                        cx,
                        move |app, window, cx| {
                            if is_dir {
                                app.files_navigate_to(path_for_click.clone(), window, cx);
                            } else {
                                app.files_open_path(path_for_click.clone(), window, cx);
                            }
                        },
                    )
                    .into_any_element()
                })
                .collect()
        })
}

fn entry_row(
    id: impl Into<gpui::ElementId>,
    name: &str,
    is_dir: bool,
    selected: bool,
    cx: &mut Context<MitsuroApp>,
    on_click: impl Fn(&mut MitsuroApp, &mut gpui::Window, &mut Context<MitsuroApp>) + 'static,
) -> impl IntoElement {
    let colors = theme::colors();
    let name = name.to_string();
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .px(px(12.0))
        .py(px(7.0))
        .mx(px(6.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .bg(if selected {
            colors.bg_selected
        } else {
            theme::transparent()
        })
        .hover(|s| s.bg(colors.bg_hover))
        .on_click(cx.listener(move |app, _, window, cx| {
            on_click(app, window, cx);
        }))
        .child(
            Icon::new(if is_dir {
                IconName::Folder
            } else {
                IconName::File
            })
            .with_size(px(14.0))
            .text_color(if is_dir {
                colors.accent
            } else {
                colors.text_tertiary
            }),
        )
        .child(div().text_sm().text_color(colors.text).child(name))
}

fn entry_row_with_sub(
    id: impl Into<gpui::ElementId>,
    name: &str,
    subtitle: &str,
    is_dir: bool,
    score: u32,
    selected: bool,
    cx: &mut Context<MitsuroApp>,
    on_click: impl Fn(&mut MitsuroApp, &mut gpui::Window, &mut Context<MitsuroApp>) + 'static,
) -> impl IntoElement {
    let colors = theme::colors();
    let name = name.to_string();
    let subtitle = subtitle.to_string();
    div()
        .id(id)
        .flex()
        .flex_col()
        .gap(px(2.0))
        .px(px(12.0))
        .py(px(7.0))
        .mx(px(6.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .bg(if selected {
            colors.bg_selected
        } else {
            theme::transparent()
        })
        .hover(|s| s.bg(colors.bg_hover))
        .on_click(cx.listener(move |app, _, window, cx| {
            on_click(app, window, cx);
        }))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(
                    Icon::new(if is_dir {
                        IconName::Folder
                    } else {
                        IconName::File
                    })
                    .with_size(px(14.0))
                    .text_color(if is_dir {
                        colors.accent
                    } else {
                        colors.text_tertiary
                    }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .text_color(colors.text)
                        .child(name),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child(format!("{score}")),
                ),
        )
        .child(
            div()
                .pl(px(22.0))
                .text_xs()
                .text_color(colors.text_tertiary)
                .child(subtitle),
        )
}

fn preview_pane(
    path: Option<&str>,
    preview: &str,
    error: Option<&str>,
    editor_input: &gpui::Entity<gpui_component::input::InputState>,
    writable: bool,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("files-preview")
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .bg(colors.bg_under)
        .child(
            div()
                .px(px(16.0))
                .py(px(10.0))
                .border_b_1()
                .border_color(colors.border)
                .bg(colors.bg_sidebar)
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child(
                            path.unwrap_or("Select a file to preview · fs/readFile")
                                .to_string(),
                        ),
                ),
        )
        .child(
            div()
                .id("files-preview-body")
                .flex()
                .flex_1()
                .min_h_0()
                .px(px(16.0))
                .py(px(12.0))
                .overflow_y_scroll()
                .child(if let Some(err) = error {
                    div()
                        .text_sm()
                        .text_color(colors.status_error)
                        .child(format!("[error] {err}"))
                        .into_any_element()
                } else if writable && path.is_some() {
                    div()
                        .flex()
                        .flex_1()
                        .min_h(px(240.0))
                        .rounded(px(8.0))
                        .bg(colors.bg_elevated)
                        .border_1()
                        .border_color(colors.border)
                        .px(px(8.0))
                        .py(px(6.0))
                        .child(Input::new(editor_input).appearance(false).h_full())
                        .into_any_element()
                } else if preview.is_empty() {
                    div()
                        .text_sm()
                        .text_color(colors.text_tertiary)
                        .child(
                            "File contents appear here. Browse the project tree or fuzzy-search names.",
                        )
                        .into_any_element()
                } else {
                    div()
                        .text_sm()
                        .text_color(colors.text)
                        .child(preview.to_string())
                        .into_any_element()
                }),
        )
}

fn status_footer(
    cwd: &str,
    browsing: bool,
    entry_count: usize,
    fuzzy_count: usize,
    selected: Option<&str>,
) -> impl IntoElement {
    let colors = theme::colors();
    let detail = if browsing {
        format!("list · {entry_count} entries · {cwd}")
    } else {
        format!("fuzzy · {fuzzy_count} hits · root {cwd}")
    };
    div()
        .id("files-status")
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .px(px(16.0))
        .py(px(8.0))
        .border_t_1()
        .border_color(colors.border)
        .bg(colors.bg_sidebar)
        .child(
            div()
                .text_xs()
                .text_color(colors.text_tertiary)
                .child(detail),
        )
        .child(
            div().text_xs().text_color(colors.text_tertiary).child(
                selected
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "fs/readDirectory · fuzzyFileSearch".into()),
            ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_directories_have_a_bounded_render_budget() {
        assert_eq!(DIRECTORY_RENDER_LIMIT, 200);
        assert_eq!(312usize.saturating_sub(DIRECTORY_RENDER_LIMIT), 112);
    }
}
