use std::path::{Component, Path, PathBuf};

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _,
};
use gpui_component::{Icon, StyledExt as _};

use crate::app::KrustyDesktop;
use crate::design::theme;

const FOLDER_ICON: &str = "icons/folder.svg";
const FOLDER_OPEN_ICON: &str = "icons/folder-open.svg";

pub fn workspace_dialog_backdrop(cx: &mut Context<KrustyDesktop>) -> impl IntoElement {
    div()
        .id("workspace-dialog-backdrop")
        .absolute()
        .top_0()
        .right_0()
        .bottom_0()
        .left_0()
        .bg(gpui::black().opacity(0.56))
        .on_click(cx.listener(|view, _, _, cx| {
            view.cancel_workspace_dialog(cx);
        }))
}

pub fn open_workspace_dialog(
    root_path: &Path,
    selected_path: &Path,
    cx: &mut Context<KrustyDesktop>,
) -> AnyElement {
    div()
        .absolute()
        .top_0()
        .right_0()
        .bottom_0()
        .left_0()
        .p_6()
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .id("open-workspace-dialog")
                .w(px(680.0))
                .occlude()
                .border_1()
                .border_color(theme::hairline())
                .bg(theme::surface())
                .p_4()
                .flex()
                .flex_col()
                .gap_3()
                .child(header(root_path, cx))
                .child(
                    div()
                        .border_1()
                        .border_color(theme::hairline())
                        .bg(theme::app_bg())
                        .id("workspace-directory-list")
                        .h(px(360.0))
                        .overflow_y_scroll()
                        .children(directory_rows(root_path, selected_path, cx)),
                )
                .child(footer(selected_path, cx)),
        )
        .into_any_element()
}

fn header(root_path: &Path, cx: &mut Context<KrustyDesktop>) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(div().text_lg().font_semibold().child("Open Workspace"))
        .child(breadcrumb(root_path, cx))
}

fn breadcrumb(root_path: &Path, cx: &mut Context<KrustyDesktop>) -> impl IntoElement {
    let items = breadcrumb_items(root_path);
    let last_index = items.len().saturating_sub(1);
    let mut children = Vec::new();

    for (index, (label, path, current)) in items.into_iter().enumerate() {
        children.push(
            div()
                .id(("workspace-breadcrumb", index))
                .text_sm()
                .text_color(if current {
                    theme::text()
                } else {
                    theme::text_muted()
                })
                .when(!current, |this| {
                    this.cursor_pointer()
                        .hover(|style| style.text_color(theme::text()))
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.navigate_workspace_picker_root(path.clone(), cx);
                        }))
                })
                .child(label)
                .into_any_element(),
        );
        if index != last_index {
            children.push(
                div()
                    .text_sm()
                    .text_color(theme::text_muted())
                    .child("›")
                    .into_any_element(),
            );
        }
    }

    div().flex().items_center().gap_1().children(children)
}

fn directory_rows(
    root_path: &Path,
    selected_path: &Path,
    cx: &mut Context<KrustyDesktop>,
) -> Vec<AnyElement> {
    let mut rows = Vec::new();

    if let Some(parent) = root_path.parent() {
        let parent = parent.to_path_buf();
        rows.push(
            div()
                .id("workspace-parent-row")
                .h(px(34.0))
                .px_3()
                .border_b_1()
                .border_color(theme::hairline())
                .flex()
                .items_center()
                .gap_2()
                .cursor_pointer()
                .text_color(theme::text_muted())
                .hover(|style| style.bg(theme::surface_hover()))
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.navigate_workspace_picker_root(parent.clone(), cx);
                }))
                .child("..")
                .child("Parent folder")
                .into_any_element(),
        );
    }

    for path in child_directories(root_path) {
        rows.push(directory_row(path, selected_path, cx));
    }

    if rows.is_empty() {
        rows.push(
            div()
                .p_4()
                .text_sm()
                .text_color(theme::text_muted())
                .child("No folders found here.")
                .into_any_element(),
        );
    }

    rows
}

fn directory_row(
    path: PathBuf,
    selected_path: &Path,
    cx: &mut Context<KrustyDesktop>,
) -> AnyElement {
    let selected = paths_equal(&path, selected_path);
    let label = directory_label(&path);
    let select_path = path.clone();
    let browse_path = path.clone();

    div()
        .id(SharedString::from(format!(
            "workspace-folder-row-{}",
            path.display()
        )))
        .h(px(38.0))
        .px_3()
        .border_b_1()
        .border_color(theme::hairline())
        .bg(if selected {
            theme::surface_selected()
        } else {
            gpui::transparent_black()
        })
        .flex()
        .items_center()
        .justify_between()
        .gap_3()
        .cursor_pointer()
        .hover(|style| style.bg(theme::surface_hover()))
        .on_click(cx.listener(move |view, _, _, cx| {
            view.select_workspace_picker_path(select_path.clone(), cx);
        }))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    Icon::empty()
                        .path(if selected {
                            FOLDER_OPEN_ICON
                        } else {
                            FOLDER_ICON
                        })
                        .size(px(18.0))
                        .text_color(if selected {
                            theme::accent()
                        } else {
                            theme::text_muted()
                        }),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(div().text_sm().child(label))
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::text_muted())
                                .truncate()
                                .child(path.display().to_string()),
                        ),
                ),
        )
        .child(
            div()
                .id(SharedString::from(format!(
                    "browse-workspace-folder-{}",
                    path.display()
                )))
                .px_2()
                .py_1()
                .border_1()
                .border_color(theme::hairline())
                .text_xs()
                .text_color(theme::text_muted())
                .hover(|style| style.bg(theme::surface_hover()))
                .on_click(cx.listener(move |view, _, _, cx| {
                    cx.stop_propagation();
                    view.navigate_workspace_picker_root(browse_path.clone(), cx);
                }))
                .child("Browse"),
        )
        .into_any_element()
}

fn footer(selected_path: &Path, cx: &mut Context<KrustyDesktop>) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            div()
                .min_w_0()
                .text_xs()
                .text_color(theme::text_muted())
                .truncate()
                .child(format!("Selected: {}", selected_path.display())),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    dialog_button("cancel-open-workspace", "Cancel", false).on_click(cx.listener(
                        |view, _, _, cx| {
                            view.cancel_workspace_dialog(cx);
                        },
                    )),
                )
                .child(
                    dialog_button("confirm-open-workspace", "Open", true).on_click(cx.listener(
                        |view, _, _, cx| {
                            view.open_selected_workspace(cx);
                        },
                    )),
                ),
        )
}

fn dialog_button(
    id: &'static str,
    label: &'static str,
    primary: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .h(px(30.0))
        .px_3()
        .border_1()
        .border_color(if primary {
            theme::accent()
        } else {
            theme::hairline()
        })
        .bg(if primary {
            theme::accent().opacity(0.16)
        } else {
            gpui::transparent_black()
        })
        .text_color(if primary {
            theme::accent()
        } else {
            theme::text_muted()
        })
        .hover(|style| style.bg(theme::surface_hover()))
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_center()
        .text_sm()
        .child(label)
}

fn child_directories(path: &Path) -> Vec<PathBuf> {
    let mut directories = std::fs::read_dir(path)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.filter_map(Result::ok))
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter(|path| !is_hidden(path))
        .collect::<Vec<_>>();

    directories.sort_by_key(|path| directory_label(path).to_lowercase());
    directories
}

fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.'))
}

fn directory_label(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| path.display().to_string())
}

fn breadcrumb_items(path: &Path) -> Vec<(String, PathBuf, bool)> {
    let mut items = Vec::new();
    let mut current = PathBuf::new();
    let components = path.components().collect::<Vec<_>>();

    for component in &components {
        match component {
            Component::RootDir => {
                current.push(component.as_os_str());
                items.push(("/".to_owned(), current.clone(), false));
            }
            Component::Normal(label) => {
                current.push(label);
                items.push((label.to_string_lossy().to_string(), current.clone(), false));
            }
            Component::Prefix(prefix) => {
                current.push(prefix.as_os_str());
                items.push((
                    prefix.as_os_str().to_string_lossy().to_string(),
                    current.clone(),
                    false,
                ));
            }
            Component::CurDir | Component::ParentDir => {}
        }
    }

    if items.is_empty() {
        items.push((path.display().to_string(), path.to_path_buf(), false));
    }
    if let Some(last) = items.last_mut() {
        last.2 = true;
    }
    items
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    let left = std::fs::canonicalize(left).unwrap_or_else(|_| left.to_path_buf());
    let right = std::fs::canonicalize(right).unwrap_or_else(|_| right.to_path_buf());
    left == right
}
