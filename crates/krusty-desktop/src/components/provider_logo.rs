use std::collections::HashMap;

use gpui::{
    div, px, AnyElement, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _,
};
use gpui_component::Icon;

use crate::api::ProviderStatus;
use crate::app::KrustyDesktop;
use crate::design::theme;

pub const AUTH_PANEL_WIDTH: f32 = 420.0;
pub const AUTH_DETAIL_HEIGHT: f32 = 276.0;
pub const PROVIDER_GRID_HEIGHT: f32 = 90.0;
pub const PROVIDER_GRID_COLS: usize = 6;
pub const PROVIDER_GRID_GAP: f32 = 4.0;
pub const PROVIDER_GRID_PADDING: f32 = 8.0;

/// Grid order: xAI is intentionally last (rightmost tile).
const AUTH_PROVIDER_ORDER: &[&str] = &[
    "minimax",
    "anthropic",
    "openai",
    "z_ai",
    "openrouter",
    "grok",
];

pub fn provider_card_size() -> f32 {
    let inner = AUTH_PANEL_WIDTH - PROVIDER_GRID_PADDING * 2.0;
    let gaps = (PROVIDER_GRID_COLS.saturating_sub(1) as f32) * PROVIDER_GRID_GAP;
    ((inner - gaps) / PROVIDER_GRID_COLS as f32).floor()
}

pub fn provider_grid_content_width() -> f32 {
    PROVIDER_GRID_COLS as f32 * provider_card_size()
        + (PROVIDER_GRID_COLS.saturating_sub(1) as f32) * PROVIDER_GRID_GAP
}

#[cfg(test)]
pub fn provider_grid_rows(count: usize) -> usize {
    count.max(1).saturating_add(PROVIDER_GRID_COLS - 1) / PROVIDER_GRID_COLS
}

pub fn prepare_providers_for_display(providers: Vec<ProviderStatus>) -> Vec<ProviderStatus> {
    let mut by_id = providers
        .into_iter()
        .map(|status| (status.id.clone(), normalize_provider_status(status)))
        .collect::<HashMap<_, _>>();

    AUTH_PROVIDER_ORDER
        .iter()
        .map(|id| {
            by_id
                .remove(*id)
                .unwrap_or_else(|| canonical_provider_status(id))
        })
        .collect()
}

fn normalize_provider_status(mut status: ProviderStatus) -> ProviderStatus {
    status.name = provider_display_name(&status.id, &status.name);
    status
}

fn provider_display_name(id: &str, fallback: &str) -> String {
    match id {
        "grok" => "xAI".to_owned(),
        _ => fallback.to_owned(),
    }
}

fn canonical_provider_status(id: &str) -> ProviderStatus {
    let (name, supports_oauth, auth_methods) = match id {
        "minimax" => ("MiniMax", false, vec!["api_key".to_owned()]),
        "anthropic" => (
            "Anthropic",
            true,
            vec!["oauth_browser".to_owned(), "api_key".to_owned()],
        ),
        "openai" => (
            "OpenAI",
            true,
            vec![
                "oauth_browser".to_owned(),
                "oauth_device".to_owned(),
                "api_key".to_owned(),
            ],
        ),
        "z_ai" => ("Z.ai", false, vec!["api_key".to_owned()]),
        "openrouter" => ("OpenRouter", false, vec!["api_key".to_owned()]),
        "grok" => ("xAI", true, vec!["oauth_browser".to_owned()]),
        _ => (id, false, vec!["api_key".to_owned()]),
    };

    ProviderStatus {
        id: id.to_owned(),
        name: name.to_owned(),
        configured: false,
        has_oauth: false,
        supports_oauth,
        auth_methods,
    }
}

pub fn provider_icon_cell(
    status: ProviderStatus,
    selected_provider: Option<&str>,
    view: Entity<KrustyDesktop>,
) -> impl IntoElement {
    let selected = selected_provider == Some(status.id.as_str());
    let click_provider = status.id.clone();
    let hover_provider = status.id.clone();
    let hover_view = view.clone();

    let card_size = provider_card_size();

    div()
        .id(SharedString::from(format!("provider-{}", status.id)))
        .relative()
        .size(px(card_size))
        .overflow_hidden()
        .border_1()
        .border_color(if status.configured {
            theme::success()
        } else if selected {
            theme::accent()
        } else {
            theme::hairline()
        })
        .bg(if selected {
            theme::surface_selected()
        } else {
            gpui::transparent_black()
        })
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|style| style.bg(theme::surface_selected()))
        .on_click(move |_, _, cx| {
            let provider = click_provider.clone();
            view.update(cx, |view, cx| view.select_provider(provider, cx));
        })
        .on_hover(move |hovered, _, cx| {
            let provider = hover_provider.clone();
            if *hovered {
                hover_view.update(cx, |view, cx| view.start_provider_hover(provider, cx));
            } else {
                hover_view.update(cx, |view, cx| view.end_provider_hover(&provider, cx));
            }
        })
        .child(provider_logo(&status.id, &status.name, px(card_size * 0.5)))
}

pub fn provider_logo(provider: &str, display_name: &str, size: gpui::Pixels) -> AnyElement {
    if let Some(path) = provider_logo_path(provider) {
        Icon::empty()
            .path(path)
            .size(size)
            .text_color(theme::text())
            .into_any_element()
    } else {
        div()
            .size(size)
            .border_1()
            .border_color(theme::hairline())
            .flex()
            .items_center()
            .justify_center()
            .text_xs()
            .text_color(theme::text_muted())
            .child(provider_initials(display_name))
            .into_any_element()
    }
}

pub fn provider_hover_card(display_name: &str, index: usize) -> impl IntoElement {
    let col = (index % PROVIDER_GRID_COLS) as f32;
    let row = (index / PROVIDER_GRID_COLS) as f32;
    let card_width = (display_name.chars().count() as f32 * 7.0 + 22.0)
        .clamp(72.0, AUTH_PANEL_WIDTH - PROVIDER_GRID_PADDING * 2.0);
    let grid_width = provider_grid_content_width();
    let grid_offset = ((AUTH_PANEL_WIDTH - grid_width) / 2.0).max(PROVIDER_GRID_PADDING);
    let card_size = provider_card_size();
    let cell_stride = card_size + PROVIDER_GRID_GAP;
    let cell_left = grid_offset + col * cell_stride;
    let cell_center_x = cell_left + card_size / 2.0;
    let left = (cell_center_x - card_width / 2.0).clamp(
        PROVIDER_GRID_PADDING,
        AUTH_PANEL_WIDTH - card_width - PROVIDER_GRID_PADDING,
    );
    let row_top = (PROVIDER_GRID_HEIGHT - card_size) / 2.0 + row * (card_size + PROVIDER_GRID_GAP);
    let top = px(row_top - 34.0);

    div()
        .absolute()
        .top(top)
        .left(px(left))
        .w(px(card_width))
        .occlude()
        .border_1()
        .border_color(theme::hairline().alpha(1.0))
        .bg(theme::surface().alpha(1.0))
        .shadow_md()
        .p_2()
        .text_xs()
        .line_height(px(16.0))
        .text_center()
        .text_color(theme::text())
        .child(display_name.to_owned())
}

fn provider_logo_path(provider: &str) -> Option<&'static str> {
    match provider {
        "anthropic" => Some("assets/provider-logos/anthropic.svg"),
        "grok" | "xai" | "x_ai" => Some("assets/provider-logos/xai.svg"),
        "minimax" => Some("assets/provider-logos/minimax.svg"),
        "openai" => Some("assets/provider-logos/openai.svg"),
        "openrouter" => Some("assets/provider-logos/openrouter.svg"),
        "z_ai" | "zai" => Some("assets/provider-logos/zai.svg"),
        _ => None,
    }
}

fn provider_initials(display_name: &str) -> String {
    display_name
        .split_whitespace()
        .filter_map(|word| word.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase()
}

#[cfg(test)]
mod tests {
    use super::{
        prepare_providers_for_display, provider_card_size, provider_grid_rows, ProviderStatus,
        AUTH_DETAIL_HEIGHT, PROVIDER_GRID_COLS, PROVIDER_GRID_HEIGHT,
    };

    #[test]
    fn provider_grid_fits_six_providers_in_one_row() {
        assert_eq!(provider_grid_rows(6), 1);
        assert!(provider_card_size() >= 48.0);
        assert!(PROVIDER_GRID_HEIGHT < AUTH_DETAIL_HEIGHT);
        assert_eq!(PROVIDER_GRID_COLS, 6);
    }

    #[test]
    fn prepare_providers_inserts_xai_last_when_missing() {
        let providers = vec![
            ProviderStatus {
                id: "minimax".to_owned(),
                name: "MiniMax".to_owned(),
                configured: false,
                has_oauth: false,
                supports_oauth: false,
                auth_methods: vec!["api_key".to_owned()],
            },
            ProviderStatus {
                id: "anthropic".to_owned(),
                name: "Anthropic".to_owned(),
                configured: false,
                has_oauth: false,
                supports_oauth: true,
                auth_methods: vec!["oauth_browser".to_owned()],
            },
        ];

        let prepared = prepare_providers_for_display(providers);
        assert_eq!(prepared.len(), 6);
        assert_eq!(prepared.last().map(|p| p.id.as_str()), Some("grok"));
        assert_eq!(prepared.last().map(|p| p.name.as_str()), Some("xAI"));
    }
}
