use gpui::{px, App, ElementId, SharedString};
use gpui_component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_component::Icon;

use crate::design::theme;

#[derive(Clone, Copy)]
pub enum KrustyButtonKind {
    Primary,
    Secondary,
    Ghost,
    Danger,
}

pub fn krusty_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    kind: KrustyButtonKind,
    cx: &App,
) -> Button {
    base_button(id, kind, cx).label(label)
}

pub fn krusty_icon_button(
    id: impl Into<ElementId>,
    icon: impl Into<Icon>,
    kind: KrustyButtonKind,
    cx: &App,
) -> Button {
    base_button(id, kind, cx).icon(icon)
}

fn base_button(id: impl Into<ElementId>, kind: KrustyButtonKind, cx: &App) -> Button {
    let variant = match kind {
        KrustyButtonKind::Primary => ButtonCustomVariant::new(cx)
            .color(theme::accent())
            .foreground(theme::app_bg())
            .border(theme::accent())
            .hover(theme::text_muted())
            .active(theme::text()),
        KrustyButtonKind::Secondary => ButtonCustomVariant::new(cx)
            .color(theme::surface())
            .foreground(theme::text())
            .border(theme::hairline())
            .hover(theme::surface_hover())
            .active(theme::surface_selected()),
        KrustyButtonKind::Ghost => ButtonCustomVariant::new(cx)
            .color(gpui::transparent_black())
            .foreground(theme::text())
            .border(gpui::transparent_black())
            .hover(theme::surface_hover())
            .active(theme::surface_selected()),
        KrustyButtonKind::Danger => ButtonCustomVariant::new(cx)
            .color(theme::danger().opacity(0.14))
            .foreground(theme::danger())
            .border(theme::danger().opacity(0.42))
            .hover(theme::danger().opacity(0.22))
            .active(theme::danger().opacity(0.30)),
    };

    Button::new(id).custom(variant).rounded(px(0.0))
}
