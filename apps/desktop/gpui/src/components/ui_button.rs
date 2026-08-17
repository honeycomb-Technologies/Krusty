//! Mitsuro button and icon-button primitives built on `gpui-component`.
//!
//! Feature surfaces choose semantic tone and size; this module owns foreground,
//! border, hover, active, selected, disabled, loading, radius, and hit-target
//! behavior. New feature-local button lookalikes should not be introduced.

use gpui::{px, App, ElementId, SharedString};
use gpui_component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_component::{Disableable as _, Icon, Selectable as _, Sizable as _};

use crate::theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonTone {
    Primary,
    Secondary,
    Ghost,
    Subtle,
    Destructive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonSize {
    Small,
    Medium,
    Large,
}

impl ButtonSize {
    fn height(self) -> f32 {
        let shape = theme::shape();
        match self {
            Self::Small => shape.control_sm,
            Self::Medium => shape.control_md,
            Self::Large => shape.control_lg,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ButtonState {
    pub selected: bool,
    pub disabled: bool,
    pub loading: bool,
}

pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    tone: ButtonTone,
    size: ButtonSize,
    state: ButtonState,
    cx: &App,
) -> Button {
    Button::new(id)
        .label(label)
        .custom(variant(tone, cx))
        .rounded(px(theme::shape().radius_md))
        .with_size(px(size.height()))
        .selected(state.selected)
        .disabled(state.disabled)
        .loading(state.loading)
}

#[allow(clippy::too_many_arguments)]
pub fn icon_button(
    id: impl Into<ElementId>,
    icon: Icon,
    tooltip: impl Into<SharedString>,
    tone: ButtonTone,
    size: ButtonSize,
    state: ButtonState,
    cx: &App,
) -> Button {
    Button::new(id)
        .icon(icon)
        .tooltip(tooltip)
        .custom(variant(tone, cx))
        .rounded(px(theme::shape().radius_md))
        .with_size(px(size.height().max(theme::shape().hit_target_min)))
        .selected(state.selected)
        .disabled(state.disabled)
        .loading(state.loading)
}

fn variant(tone: ButtonTone, cx: &App) -> ButtonCustomVariant {
    let colors = theme::colors();
    match tone {
        ButtonTone::Primary => ButtonCustomVariant::new(cx)
            .color(colors.bg_button_primary)
            .foreground(colors.fg_button_primary)
            .border(colors.bg_button_primary)
            .hover(colors.bg_button_primary_hover)
            .active(colors.bg_button_primary_active),
        ButtonTone::Secondary => ButtonCustomVariant::new(cx)
            .color(colors.bg_button_secondary)
            .foreground(colors.text)
            .border(colors.border)
            .hover(colors.bg_hover)
            .active(colors.bg_selected),
        ButtonTone::Ghost => ButtonCustomVariant::new(cx)
            .color(theme::transparent())
            .foreground(colors.text_secondary)
            .border(theme::transparent())
            .hover(colors.bg_hover)
            .active(colors.bg_selected),
        ButtonTone::Subtle => ButtonCustomVariant::new(cx)
            .color(colors.bg_elevated)
            .foreground(colors.text_secondary)
            .border(colors.border_subtle)
            .hover(colors.bg_hover)
            .active(colors.bg_selected),
        ButtonTone::Destructive => ButtonCustomVariant::new(cx)
            .color(theme::transparent())
            .foreground(colors.status_error)
            .border(theme::transparent())
            .hover(colors.destructive_soft)
            .active(colors.status_error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_button_sizes_respect_the_minimum_hit_target() {
        assert!(ButtonSize::Small.height() <= theme::shape().hit_target_min);
        assert!(ButtonSize::Medium.height() >= theme::shape().hit_target_min);
        assert!(ButtonSize::Large.height() > ButtonSize::Medium.height());
    }
}
