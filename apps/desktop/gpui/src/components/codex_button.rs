//! Codex-styled primary / secondary buttons (elevated + fg fills, not stock theme primary).

use gpui::{px, App, ElementId, SharedString};
use gpui_component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_component::Icon;

use crate::theme;

/// Primary with leading icon.
/// Dark-mode Codex `background-button-primary` = text foreground (white fill, dark label).
pub fn primary_with_icon(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    icon: Icon,
    cx: &App,
) -> Button {
    Button::new(id)
        .label(label)
        .icon(icon)
        .custom(primary_variant(cx))
        .rounded(px(999.0))
}

fn primary_variant(cx: &App) -> ButtonCustomVariant {
    let c = theme::colors();
    ButtonCustomVariant::new(cx)
        .color(c.bg_button_primary)
        .foreground(c.fg_button_primary)
        .border(c.bg_button_primary)
        .hover(c.bg_button_primary_hover)
        .active(c.bg_button_primary_active)
}
