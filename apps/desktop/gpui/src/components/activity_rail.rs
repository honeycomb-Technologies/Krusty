//! Thin left activity rail — product modes:
//! Chat · Work · Codex · Atlas · Terminal · Files · Computer · Extensions · Settings.
//!
//! Density: ~48px, no selected border, near-invisible on calm empty stage.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _,
};
use gpui_component::tooltip::Tooltip;
use gpui_component::{Icon, IconName, Sizable as _};

use crate::app::{MitsuroApp, ProductMode};
use crate::theme::{self, CodexColors};

/// Rail width — thin product column; blends into underlay on calm stage.
const RAIL_W: f32 = 48.0;
const ICON_HIT: f32 = 34.0;

pub fn activity_rail(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let active = app.active_mode();
    let calm = app.is_calm_stage();

    div()
        .id("activity-rail")
        .flex()
        .flex_col()
        .items_center()
        .w(px(RAIL_W))
        .h_full()
        // On calm stage: blend into underlay (no scaffold border).
        .bg(if calm {
            colors.bg_under
        } else {
            colors.bg_rail
        })
        .when(!calm, |this| {
            this.border_r_1().border_color(colors.border_subtle)
        })
        .py(px(12.0))
        .gap(px(2.0))
        // Mitsuro mark (text initial — not OpenAI logo)
        .child(
            div()
                .mb(px(8.0))
                .w(px(26.0))
                .h(px(26.0))
                .rounded(px(8.0))
                .bg(if calm {
                    theme::hex_alpha(0xffffff, 0.04)
                } else {
                    colors.bg_elevated
                })
                .flex()
                .items_center()
                .justify_center()
                .text_xs()
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(if calm {
                    colors.text_tertiary
                } else {
                    colors.text_secondary
                })
                .child("M"),
        )
        .child(rail_button(
            "rail-chat",
            IconName::Bot,
            "Chat",
            active == ProductMode::Chat,
            calm,
            &colors,
            cx,
            |app, window, cx| app.set_mode(ProductMode::Chat, window, cx),
        ))
        .child(rail_button(
            "rail-work",
            IconName::LayoutDashboard,
            "Work",
            active == ProductMode::Work,
            calm,
            &colors,
            cx,
            |app, window, cx| app.set_mode(ProductMode::Work, window, cx),
        ))
        .child(rail_button(
            "rail-codex",
            IconName::Inbox,
            "Codex",
            active == ProductMode::Codex,
            calm,
            &colors,
            cx,
            |app, window, cx| app.set_mode(ProductMode::Codex, window, cx),
        ))
        .child(rail_button(
            "rail-atlas",
            IconName::Globe,
            "Atlas",
            active == ProductMode::Atlas,
            calm,
            &colors,
            cx,
            |app, window, cx| app.open_atlas(window, cx),
        ))
        .child(rail_button(
            "rail-terminal",
            IconName::SquareTerminal,
            "Terminal",
            active == ProductMode::Terminal,
            calm,
            &colors,
            cx,
            |app, window, cx| app.set_mode(ProductMode::Terminal, window, cx),
        ))
        .child(rail_button(
            "rail-files",
            IconName::FolderOpen,
            "Files",
            active == ProductMode::Files,
            calm,
            &colors,
            cx,
            |app, window, cx| app.set_mode(ProductMode::Files, window, cx),
        ))
        .child(rail_button(
            "rail-computer",
            IconName::Building2,
            "Computer",
            active == ProductMode::Computer,
            calm,
            &colors,
            cx,
            |app, window, cx| app.set_mode(ProductMode::Computer, window, cx),
        ))
        .child(rail_button(
            "rail-extensions",
            IconName::Asterisk,
            "Extensions",
            active == ProductMode::Extensions,
            calm,
            &colors,
            cx,
            |app, window, cx| app.set_mode(ProductMode::Extensions, window, cx),
        ))
        .child(div().flex_1())
        .child(rail_button(
            "rail-settings",
            IconName::Settings,
            "Settings",
            active == ProductMode::Settings,
            calm,
            &colors,
            cx,
            |app, window, cx| app.set_mode(ProductMode::Settings, window, cx),
        ))
}

fn rail_button(
    id: &'static str,
    icon: IconName,
    tooltip: &'static str,
    selected: bool,
    calm: bool,
    colors: &CodexColors,
    cx: &mut Context<MitsuroApp>,
    on_click: impl Fn(&mut MitsuroApp, &mut gpui::Window, &mut Context<MitsuroApp>) + 'static,
) -> impl IntoElement {
    let colors = *colors;
    // Selected: soft fill only — never a border (product density, not scaffold).
    // On calm stage, mute selection fill further so the stage reads full-bleed.
    let selected_bg = if calm {
        theme::hex_alpha(0xffffff, 0.04)
    } else {
        colors.bg_selected
    };
    div()
        .id(id)
        .w(px(ICON_HIT))
        .h(px(ICON_HIT))
        .rounded(px(10.0))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .bg(if selected {
            selected_bg
        } else {
            theme::transparent()
        })
        .hover(|s| s.bg(colors.bg_hover))
        .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
        .on_click(cx.listener(move |app, _, window, cx| {
            on_click(app, window, cx);
        }))
        .child(Icon::new(icon).with_size(px(17.0)).text_color(if selected {
            if calm {
                colors.text_secondary
            } else {
                colors.text
            }
        } else {
            colors.text_tertiary
        }))
}
