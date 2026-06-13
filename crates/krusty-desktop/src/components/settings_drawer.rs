use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, Animation, AnimationExt as _, AnyElement, App, Context, Entity,
    InteractiveElement as _, IntoElement as _, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window,
};
use gpui_component::animation::cubic_bezier;
use gpui_component::group_box::GroupBoxVariant;
use gpui_component::input::InputState;
use gpui_component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::setting::{SettingGroup, SettingItem, SettingPage, Settings};
use gpui_component::{Sizable as _, Size, StyledExt as _};

use crate::app::KrustyDesktop;
use crate::components::button::{krusty_button, KrustyButtonKind};
use crate::components::input::krusty_input;
use crate::components::{auth_settings, brand};
use crate::design::theme::{self, AppFont, AppearanceSettings, ThemeOption};

pub const DRAWER_ANIMATION_DURATION: Duration = Duration::from_millis(220);
const DRAWER_WIDTH: f32 = 620.0;

fn drawer_animation() -> Animation {
    Animation::new(DRAWER_ANIMATION_DURATION).with_easing(cubic_bezier(0.32, 0.72, 0.0, 1.0))
}

pub fn settings_backdrop(opening: bool, cx: &mut Context<KrustyDesktop>) -> impl gpui::IntoElement {
    let animation_id = if opening {
        "settings-backdrop-open"
    } else {
        "settings-backdrop-close"
    };

    div()
        .id("settings-backdrop")
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .bottom_0()
        .bg(gpui::black())
        .opacity(if opening { 0.52 } else { 0.0 })
        .on_click(cx.listener(|view, _, _window, cx| {
            view.close_settings(cx);
        }))
        .with_animation(animation_id, drawer_animation(), move |this, delta| {
            let opacity = if opening { delta } else { 1.0 - delta } * 0.52;
            this.opacity(opacity)
        })
}

pub fn settings_drawer(
    opening: bool,
    server_url_input: Entity<InputState>,
    api_key_input: Entity<InputState>,
    appearance: AppearanceSettings,
    connection_summary: String,
    auth_state: auth_settings::AuthSettingsState,
    cx: &mut Context<KrustyDesktop>,
) -> impl gpui::IntoElement {
    let themes = theme::available_themes(cx);
    let view = cx.entity();
    let animation_id = if opening {
        "settings-drawer-open"
    } else {
        "settings-drawer-close"
    };

    div()
        .absolute()
        .top_0()
        .right_0()
        .bottom_0()
        .w(px(DRAWER_WIDTH))
        .occlude()
        .border_l_1()
        .border_color(theme::hairline())
        .bg(theme::surface())
        .shadow_lg()
        .child(settings_component(
            server_url_input,
            api_key_input,
            appearance,
            connection_summary,
            auth_state,
            themes,
            view,
        ))
        .with_animation(animation_id, drawer_animation(), move |this, delta| {
            let closed_offset = px(-DRAWER_WIDTH);
            let offset = if opening {
                closed_offset * (1.0 - delta)
            } else {
                closed_offset * delta
            };
            this.right(offset)
        })
}

fn settings_component(
    server_url_input: Entity<InputState>,
    api_key_input: Entity<InputState>,
    appearance: AppearanceSettings,
    connection_summary: String,
    auth_state: auth_settings::AuthSettingsState,
    themes: Vec<ThemeOption>,
    view: Entity<KrustyDesktop>,
) -> impl gpui::IntoElement {
    let connection_view = view.clone();
    let auth_view = view.clone();
    let theme_view = view.clone();
    let panel_view = view.clone();
    let about_view = view;

    Settings::new("krusty-settings")
        .sidebar_width(px(152.0))
        .with_size(Size::Small)
        .with_group_variant(GroupBoxVariant::Normal)
        .pages(vec![
            SettingPage::new("Connection")
                .default_open(true)
                .resettable(false)
                .group(
                    SettingGroup::new().item(SettingItem::render(move |_, _, _| {
                        connection_settings(
                            server_url_input.clone(),
                            connection_summary.clone(),
                            connection_view.clone(),
                        )
                    })),
                ),
            SettingPage::new("Providers")
                .resettable(false)
                .group(
                    SettingGroup::new().item(SettingItem::render(move |_, _, cx| {
                        auth_settings::auth_settings_content(
                            auth_state.clone(),
                            api_key_input.clone(),
                            auth_view.read(cx).oauth_code_input(),
                            auth_view.clone(),
                            cx,
                        )
                    })),
                ),
            SettingPage::new("Theme").resettable(false).group(
                SettingGroup::new()
                    .title("Theme")
                    .item(SettingItem::render(move |_, _, cx| {
                        theme_settings(themes.clone(), appearance.clone(), theme_view.clone(), cx)
                    })),
            ),
            SettingPage::new("Panels").resettable(false).group(
                SettingGroup::new()
                    .title("Panels")
                    .item(SettingItem::render(move |_, _, _| {
                        panel_settings(panel_view.clone())
                    })),
            ),
            SettingPage::new("Brand").resettable(false).group(
                SettingGroup::new()
                    .title("Krusty")
                    .item(SettingItem::render(move |_, _, _| {
                        brand_settings(about_view.clone())
                    })),
            ),
        ])
}

fn connection_settings(
    server_url_input: Entity<InputState>,
    connection_summary: String,
    view: Entity<KrustyDesktop>,
) -> AnyElement {
    let check_view = view.clone();
    let start_view = view;

    section(
        "Server connection",
        "Desktop uses the same server/API boundary as the web and mobile clients.",
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
                settings_action("settings-check-server", "Check server").on_click(
                    move |_, _, cx| {
                        check_view.update(cx, |view, cx| view.refresh_connection(cx));
                    },
                ),
            )
            .child(
                settings_action("settings-start-server", "Start/reuse")
                    .border_color(theme::accent())
                    .text_color(theme::accent())
                    .on_click(move |_, _, cx| {
                        start_view.update(cx, |view, cx| view.ensure_server(cx));
                    }),
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
    view: Entity<KrustyDesktop>,
    cx: &mut App,
) -> AnyElement {
    let font_view = view.clone();

    section("Theme", "")
        .child(
            div()
                .id("theme-scroll-window")
                .w_full()
                .h(px(248.0))
                .border_1()
                .border_color(theme::hairline())
                .bg(theme::surface())
                .p_1()
                .overflow_y_scrollbar()
                .block_mouse_except_scroll()
                .flex()
                .flex_col()
                .gap_1()
                .children(themes.into_iter().map(|option| {
                    theme_card(option, appearance.clone(), view.clone()).into_any_element()
                })),
        )
        .child(
            div()
                .mt_1()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(div().text_sm().font_semibold().child("App font"))
                .child(font_dropdown(appearance, font_view, cx)),
        )
        .into_any_element()
}

fn panel_settings(view: Entity<KrustyDesktop>) -> AnyElement {
    let focus_view = view.clone();
    let swap_view = view.clone();
    let axis_view = view.clone();
    let chat_view = view.clone();
    let canvas_view = view;

    section(
        "Panel system",
        "Hyprland-style controls for the focused tiled panel.",
    )
    .child(
        div()
            .flex()
            .flex_wrap()
            .gap_2()
            .child(settings_action("settings-focus-next", "Focus next").on_click(
                move |_, _, cx| {
                    focus_view.update(cx, |view, cx| view.focus_next_panel(cx));
                },
            ))
            .child(settings_action("settings-swap-panel", "Swap panel · Ctrl-J").on_click(
                move |_, _, cx| {
                    swap_view.update(cx, |view, cx| view.swap_focused_panel(cx));
                },
            ))
            .child(settings_action("settings-toggle-axis", "Toggle axis · Ctrl-K").on_click(
                move |_, _, cx| {
                    axis_view.update(cx, |view, cx| view.toggle_focused_panel_axis(cx));
                },
            ))
            .child(settings_action("settings-add-chat", "Add chat").on_click(move |_, _, cx| {
                chat_view.update(cx, |view, cx| {
                    view.split_focused(
                        crate::panels::SplitAxis::Horizontal,
                        crate::panels::PanelKind::Chat,
                        cx,
                    );
                });
            }))
            .child(settings_action("settings-add-canvas", "Add canvas").on_click(
                move |_, _, cx| {
                    canvas_view.update(cx, |view, cx| {
                        view.split_focused(
                            crate::panels::SplitAxis::Horizontal,
                            crate::panels::PanelKind::ScratchCanvas,
                            cx,
                        );
                    });
                },
            )),
    )
    .child(
        div()
            .text_xs()
            .text_color(theme::text_muted())
            .child("Ctrl-J swaps the focused panel with its adjacent tile. Ctrl-K flips the nearest split axis."),
    )
    .into_any_element()
}

fn brand_settings(view: Entity<KrustyDesktop>) -> AnyElement {
    section(
        "Official branding",
        "Krusty uses the same K mark from the Expo app icon and splash animation in the native GPUI shell.",
    )
    .child(
        div()
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .size(px(48.0))
                    .border_1()
                    .border_color(theme::hairline())
                    .bg(theme::app_bg())
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(brand::mark(38.0)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_sm().font_semibold().child("Krusty Desktop"))
                    .child(div().text_xs().text_color(theme::text_muted()).child(
                        "Server-backed native GPUI app with project tabs and tiled panels.",
                    )),
            ),
    )
    .child(
        settings_action("settings-brand-home", "Return to landing").on_click(move |_, _, cx| {
            view.update(cx, |view, cx| view.open_landing(cx));
        }),
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
                .when(!description.is_empty(), |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(theme::text_muted())
                            .child(description),
                    )
                }),
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
    view: Entity<KrustyDesktop>,
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
        .px_2()
        .py_1()
        .cursor_pointer()
        .hover(|style| style.bg(theme::surface_hover()))
        .on_click(move |_, window: &mut Window, cx: &mut App| {
            let settings = next.clone();
            theme::set_appearance(settings.clone());
            if let Err(error) = theme::save_appearance(&settings) {
                eprintln!("failed to save Krusty appearance settings: {error:#}");
            }
            theme::apply_component_theme_for_window(window, cx);
            view.update(cx, |view, cx| {
                view.set_status(format!("Theme set to {label}."), cx);
            });
        })
        .flex()
        .items_center()
        .gap_2()
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
                        .child(div().text_xs().font_semibold().child(option.name))
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

fn font_dropdown(
    appearance: AppearanceSettings,
    view: Entity<KrustyDesktop>,
    cx: &mut App,
) -> AnyElement {
    krusty_button(
        "settings-font-menu",
        appearance.font.label(),
        KrustyButtonKind::Secondary,
        cx,
    )
    .w(px(172.0))
    .dropdown_menu(move |menu, _window, _cx| {
        let mut menu = menu
            .scrollable(true)
            .max_h(px(220.0))
            .min_w(px(230.0))
            .label("Bundled fonts");

        for font in AppFont::ALL {
            let next = appearance.with_font(font);
            let font_view = view.clone();
            let checked = appearance.font == font;
            menu = menu.item(
                PopupMenuItem::element(move |_, _| {
                    div()
                        .font_family(font.family())
                        .text_sm()
                        .child(font.label())
                })
                .checked(checked)
                .on_click(move |_, window, cx| {
                    let settings = next.clone();
                    theme::set_appearance(settings.clone());
                    if let Err(error) = theme::save_appearance(&settings) {
                        eprintln!("failed to save Krusty font settings: {error:#}");
                    }
                    theme::apply_component_theme_for_window(window, cx);
                    font_view.update(cx, |view, cx| {
                        view.set_status(format!("Font set to {}.", font.label()), cx);
                    });
                }),
            );
        }

        menu
    })
    .into_any_element()
}

fn settings_action(id: &'static str, label: &'static str) -> gpui::Stateful<gpui::Div> {
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
        .w(px(42.0))
        .h(px(24.0))
        .border_1()
        .border_color(palette.hairline)
        .bg(palette.app_bg)
        .flex()
        .items_end()
        .p_0p5()
        .gap_0p5()
        .child(div().w(px(8.0)).h(px(15.0)).bg(palette.surface))
        .child(div().w(px(8.0)).h(px(11.0)).bg(palette.surface_hover))
        .child(div().w(px(8.0)).h(px(18.0)).bg(palette.accent))
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
