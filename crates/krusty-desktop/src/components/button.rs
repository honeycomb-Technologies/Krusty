use gpui::{px, App, ElementId, SharedString};
use gpui_component::button::{Button, ButtonCustomVariant, ButtonVariants as _};

use crate::design::theme;

#[derive(Clone, Copy)]
pub enum KrustyButtonKind {
    Primary,
    Secondary,
}

pub fn krusty_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    kind: KrustyButtonKind,
    cx: &App,
) -> Button {
    base_button(id, kind, cx).label(label)
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
    };

    Button::new(id).custom(variant).rounded(px(0.0))
}
