use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, AnyElement, Context, Entity, InteractiveElement as _, IntoElement as _,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
};
use gpui_component::input::InputState;
use gpui_component::StyledExt as _;

use crate::app::KrustyDesktop;
use crate::components::input::krusty_input;
use crate::design::theme::{self, AppFont, AppearanceSettings, ThemeOption};

pub fn settings_backdrop(cx: &mut Context<KrustyDesktop>) -> impl gpui::IntoElement {
    div()
        .id("settings-backdrop")
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .bottom_0()
        .bg(gpui::black().opacity(0.22))
        .on_click(cx.listener(|view, _, _window, cx| {
            view.close_settings(cx);
        }))
}

pub fn settings_drawer(
    server_url_input: Entity<InputState>,
    appearance: AppearanceSettings,
    connection_summary: String,
    cx: &mut Context<KrustyDesktop>,
) -> impl gpui::IntoElement {
    let themes = theme::available_themes(cx);

    div()
        .absolute()
        .top_0()
        .right_0()
        .bottom_0()
        .w(px(480.0))
        .border_l_1()
        .border_color(theme::hairline())
        .bg(theme::surface())
        .shadow_lg()
        .flex()
        .flex_col()
        .child(
            div()
                .h(px(46.0))
                .px_4()
                .border_b_1()
                .border_color(theme::hairline())
                .flex()
                .items_center()
                .justify_between()
                .child(div().font_semibold().child("Settings"))
                .child(
                    div()
                        .id("settings-close")
                        .px_2()
                        .py_1()
                        .cursor_pointer()
                        .hover(|style| style.bg(theme::surface_hover()))
                        .text_color(theme::text_muted())
                        .child("Close")
                        .on_click(cx.listener(|view, _, _window, cx| {
                            view.close_settings(cx);
                        })),
                ),
        )
        .child(
            div()
                .id("settings-scroll")
                .flex_1()
                .overflow_y_scroll()
                .p_4()
                .flex()
                .flex_col()
                .gap_4()
                .child(connection_settings(
                    server_url_input,
                    connection_summary,
                    cx,
                ))
                .child(theme_settings(themes, appearance, cx))
                .child(panel_settings(cx)),
        )
}

fn connection_settings(
    server_url_input: Entity<InputState>,
    connection_summary: String,
    cx: &mut Context<KrustyDesktop>,
) -> AnyElement {
    section(
        "Server connection",
        "Desktop will use the same server/API direction as mobile.",
    )
    .child(labeled_field(
        "Server URL",
        krusty_input(&server_url_input).into_any_element(),
    ))
    .child(
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .id("settings-check-server")
                    .px_2()
                    .py_1()
                    .border_1()
                    .border_color(theme::hairline())
                    .hover(|style| style.bg(theme::surface_hover()))
                    .cursor_pointer()
                    .text_xs()
                    .child("Check server")
                    .on_click(cx.listener(|view, _, _window, cx| {
                        view.refresh_connection(cx);
                    })),
            )
            .child(
                div()
                    .id("settings-start-server")
                    .px_2()
                    .py_1()
                    .border_1()
                    .border_color(theme::accent())
                    .hover(|style| style.bg(theme::surface_hover()))
                    .cursor_pointer()
                    .text_xs()
                    .text_color(theme::accent())
                    .child("Start/reuse")
                    .on_click(cx.listener(|view, _, _window, cx| {
                        view.ensure_server(cx);
                    })),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme::text_muted())
                    .child(connection_summary),
            ),
    )
    .into_any_element()
}

fn theme_settings(
    themes: Vec<ThemeOption>,
    appearance: AppearanceSettings,
    cx: &mut Context<KrustyDesktop>,
) -> AnyElement {
    section(
        "Theme",
        "Square gpui-component controls with Krusty-specific palette wrappers.",
    )
    .child(
        div().flex().flex_col().gap_1().children(
            themes
                .into_iter()
                .map(|option| theme_card(option, appearance.clone(), cx).into_any_element()),
        ),
    )
    .child(
        div()
            .mt_2()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_sm().font_semibold().child("App font"))
            .children(
                AppFont::ALL.map(|font| font_card(font, appearance.clone(), cx).into_any_element()),
            ),
    )
    .into_any_element()
}

fn panel_settings(cx: &mut Context<KrustyDesktop>) -> AnyElement {
    section("Panel system", "Only chat and scratch canvas panels are currently exposed.")
        .child(
            div()
                .flex()
                .gap_2()
                .child(panel_action("settings-focus-next", "Focus next").on_click(cx.listener(|view, _, _, cx| {
                    view.focus_next_panel(cx);
                })))
                .child(panel_action("settings-add-canvas", "Add canvas").on_click(cx.listener(|view, _, _, cx| {
                    view.split_focused(
                        crate::panels::SplitAxis::Horizontal,
                        crate::panels::PanelKind::ScratchCanvas,
                        cx,
                    );
                }))),
        )
        .child(
            div()
                .text_xs()
                .text_color(theme::text_muted())
                .child("List, plan, terminal, and web panels are hidden while the chat-first desktop flow is refined."),
        )
        .into_any_element()
}

fn section(title: &'static str, description: &'static str) -> gpui::Div {
    div()
        .w_full()
        .border_1()
        .border_color(theme::hairline())
        .bg(theme::app_bg())
        .p_3()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_sm().font_semibold().child(title))
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::text_muted())
                        .child(description),
                ),
        )
}

fn labeled_field(label: &'static str, field: AnyElement) -> impl gpui::IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_xs().text_color(theme::text_muted()).child(label))
        .child(field)
}

fn theme_card(
    option: ThemeOption,
    appearance: AppearanceSettings,
    cx: &mut Context<KrustyDesktop>,
) -> impl gpui::IntoElement {
    let selected = appearance.theme_name == option.name;
    let next = appearance.with_theme_name(option.name.clone());
    let card_id = SharedString::from(format!("theme-option-{}", slug(&option.name)));
    let swatch = option.palette;
    let label = option.name.clone();

    div()
        .id(card_id)
        .w_full()
        .border_1()
        .border_color(if selected {
            theme::accent()
        } else {
            theme::hairline()
        })
        .bg(if selected {
            theme::surface_selected()
        } else {
            gpui::transparent_black()
        })
        .px_3()
        .py_2()
        .cursor_pointer()
        .hover(|style| style.bg(theme::surface_hover()))
        .on_click(cx.listener(move |view, _, window, cx| {
            let settings = next.clone();
            theme::set_appearance(settings.clone());
            if let Err(error) = theme::save_appearance(&settings) {
                eprintln!("failed to save Krusty appearance settings: {error:#}");
            }
            theme::apply_component_theme_for_window(window, cx);
            view.set_status(format!("Theme set to {label}."), cx);
        }))
        .flex()
        .items_center()
        .gap_3()
        .child(theme_swatch(swatch))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(div().text_sm().font_semibold().child(option.name))
                        .when(selected, |this| {
                            this.child(div().text_xs().text_color(theme::accent()).child("Active"))
                        }),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(meta_pill(option.mode.name()))
                        .when(option.is_default, |this| this.child(meta_pill("built-in"))),
                ),
        )
}

fn font_card(
    font: AppFont,
    appearance: AppearanceSettings,
    cx: &mut Context<KrustyDesktop>,
) -> impl gpui::IntoElement {
    let selected = appearance.font == font;
    let next = appearance.with_font(font);

    div()
        .id(SharedString::from(format!("theme-font-{}", font.id())))
        .w_full()
        .border_1()
        .border_color(if selected {
            theme::accent()
        } else {
            theme::hairline()
        })
        .bg(if selected {
            theme::surface_selected()
        } else {
            theme::surface()
        })
        .p_3()
        .cursor_pointer()
        .hover(|style| style.bg(theme::surface_hover()))
        .on_click(cx.listener(move |view, _, window, cx| {
            let settings = next.clone();
            theme::set_appearance(settings.clone());
            if let Err(error) = theme::save_appearance(&settings) {
                eprintln!("failed to save Krusty font settings: {error:#}");
            }
            theme::apply_component_theme_for_window(window, cx);
            view.set_status(format!("Font set to {}.", font.label()), cx);
        }))
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(div().text_sm().font_semibold().child(font.label()))
                .when(selected, |this| {
                    this.child(div().text_xs().text_color(theme::accent()).child("Active"))
                }),
        )
        .child(
            div()
                .text_xs()
                .text_color(theme::text_muted())
                .child(font.description()),
        )
        .child(
            div()
                .w_full()
                .border_1()
                .border_color(theme::hairline())
                .bg(theme::app_bg())
                .p_2()
                .text_sm()
                .font_family(font.family())
                .child(font.preview()),
        )
}

fn panel_action(id: &'static str, label: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .px_2()
        .py_1()
        .border_1()
        .border_color(theme::hairline())
        .bg(theme::surface())
        .hover(|style| style.bg(theme::surface_hover()))
        .cursor_pointer()
        .text_xs()
        .child(label)
}

fn theme_swatch(palette: theme::Palette) -> impl gpui::IntoElement {
    div()
        .w(px(58.0))
        .h(px(36.0))
        .border_1()
        .border_color(palette.hairline)
        .bg(palette.app_bg)
        .flex()
        .items_end()
        .p_1()
        .gap_1()
        .child(div().w(px(12.0)).h(px(24.0)).bg(palette.surface))
        .child(div().w(px(12.0)).h(px(17.0)).bg(palette.surface_hover))
        .child(div().w(px(12.0)).h(px(28.0)).bg(palette.accent))
}

fn meta_pill(label: &'static str) -> impl gpui::IntoElement {
    div()
        .border_1()
        .border_color(theme::hairline())
        .px_1p5()
        .py_0p5()
        .text_xs()
        .line_height(px(14.0))
        .text_color(theme::text_muted())
        .child(label)
}

fn slug(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}
