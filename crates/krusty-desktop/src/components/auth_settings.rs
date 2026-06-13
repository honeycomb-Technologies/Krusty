use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, Animation, AnimationExt as _, AnyElement, Entity, InteractiveElement as _,
    IntoElement, ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
};
use gpui_component::animation::cubic_bezier;
use gpui_component::input::InputState;
use gpui_component::tag::Tag;
use gpui_component::{Icon, Sizable as _, Size, StyledExt as _};

use crate::api::{ActiveOAuthFlow, ProviderStatus};
use crate::app::{AuthFlow, KrustyDesktop};
use crate::components::button::{krusty_button, KrustyButtonKind};
use crate::components::input::krusty_input;
use crate::components::provider_logo::{
    prepare_providers_for_display, provider_card_size, provider_grid_content_width,
    provider_hover_card, provider_icon_cell, provider_logo, AUTH_DETAIL_HEIGHT, AUTH_PANEL_WIDTH,
    PROVIDER_GRID_GAP, PROVIDER_GRID_HEIGHT,
};
use crate::design::theme;

const REFRESH_ICON: &str = "icons/refresh-cw.svg";

#[derive(Clone)]
pub struct AuthSettingsState {
    pub providers: Vec<ProviderStatus>,
    pub providers_error: Option<String>,
    pub selected_provider: Option<String>,
    pub hover_card_provider: Option<String>,
    pub auth_flow: AuthFlow,
    pub pending: bool,
    pub active_oauth_flow: Option<ActiveOAuthFlow>,
}

fn provider_card_animation() -> Animation {
    Animation::new(Duration::from_millis(180)).with_easing(cubic_bezier(0.32, 0.72, 0.0, 1.0))
}

pub fn auth_settings_content(
    state: AuthSettingsState,
    api_key_input: Entity<InputState>,
    oauth_code_input: Entity<InputState>,
    view: Entity<KrustyDesktop>,
    cx: &mut gpui::App,
) -> AnyElement {
    let providers = prepare_providers_for_display(state.providers);
    let selected_status = state.selected_provider.as_ref().and_then(|provider| {
        providers
            .iter()
            .find(|status| status.id == *provider)
            .cloned()
    });
    let hover_card = state.hover_card_provider.as_ref().and_then(|provider| {
        providers
            .iter()
            .enumerate()
            .find(|(_, status)| status.id == *provider)
            .map(|(index, status)| (index, status.name.clone()))
    });
    let providers_empty = providers.is_empty();
    let reload_view = view.clone();
    let grid_width = px(provider_grid_content_width());
    let grid_box_width = px(AUTH_PANEL_WIDTH);
    let grid_box_height = px(PROVIDER_GRID_HEIGHT);
    let provider_icons = providers
        .into_iter()
        .map(|status| {
            provider_icon_cell(status, state.selected_provider.as_deref(), view.clone())
                .into_any_element()
        })
        .collect::<Vec<_>>();

    div()
        .w_full()
        .flex()
        .flex_col()
        .items_center()
        .gap_3()
        .child(
            div()
                .w(px(AUTH_PANEL_WIDTH))
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(div().text_sm().font_semibold().child("Auth"))
                .child(refresh_button(reload_view)),
        )
        .when_some(state.providers_error.clone(), |this, error| {
            this.child(
                div()
                    .w(px(AUTH_PANEL_WIDTH))
                    .text_xs()
                    .text_color(theme::danger())
                    .child(error),
            )
        })
        .child(
            div()
                .relative()
                .w(grid_box_width)
                .h(grid_box_height)
                .child(
                    div()
                        .id("provider-icon-scroll")
                        .w(grid_box_width)
                        .h(grid_box_height)
                        .border_1()
                        .border_color(theme::hairline())
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .w(grid_width)
                                .flex()
                                .flex_row()
                                .items_center()
                                .justify_between()
                                .gap(px(PROVIDER_GRID_GAP))
                                .children(provider_icons)
                                .when(providers_empty, |this| {
                                    this.child(
                                        div()
                                            .text_xs()
                                            .text_center()
                                            .text_color(theme::text_muted())
                                            .child(
                                                "Start or check the server, then refresh providers.",
                                            ),
                                    )
                                }),
                        ),
                )
                .when_some(hover_card, |this, (index, display_name)| {
                    this.child(provider_hover_card(&display_name, index))
                }),
        )
        .when_some(selected_status, |this, status| {
            this.child(provider_auth_flow(
                status,
                state.auth_flow,
                state.active_oauth_flow,
                api_key_input,
                oauth_code_input,
                state.pending,
                view,
                cx,
            ))
        })
        .into_any_element()
}

fn refresh_button(view: Entity<KrustyDesktop>) -> impl IntoElement {
    div()
        .id("provider-refresh")
        .size(px(30.0))
        .flex()
        .items_center()
        .justify_center()
        .text_color(theme::text_muted())
        .cursor_pointer()
        .hover(|style| style.bg(theme::surface_hover()).text_color(theme::accent()))
        .on_click(move |_, _, cx| {
            view.update(cx, |view, cx| view.refresh_providers(cx));
        })
        .child(Icon::empty().path(REFRESH_ICON).size(px(18.0)))
}

fn provider_auth_flow(
    status: ProviderStatus,
    auth_flow: AuthFlow,
    active_oauth_flow: Option<ActiveOAuthFlow>,
    api_key_input: Entity<InputState>,
    oauth_code_input: Entity<InputState>,
    pending: bool,
    view: Entity<KrustyDesktop>,
    cx: &mut gpui::App,
) -> AnyElement {
    let provider = status.id.clone();
    let display_name = status.name.clone();
    let configured = status.configured;
    let supports_oauth = status.supports_oauth;
    let supports_api_key = provider_supports_api_key(&status);
    let show_api_key = (!supports_oauth || auth_flow == AuthFlow::ApiKey) && supports_api_key;
    let show_auth_progress =
        pending && !configured && supports_oauth && auth_flow == AuthFlow::Choose;
    let show_paste_code = active_oauth_flow
        .as_ref()
        .is_some_and(|flow| flow.provider == provider && flow.paste_code);
    let remove_view = view.clone();

    let logo_size = px(provider_card_size() * 0.72);

    div()
        .w(px(AUTH_PANEL_WIDTH))
        .h(px(AUTH_DETAIL_HEIGHT))
        .border_1()
        .border_color(theme::hairline())
        .bg(theme::surface())
        .relative()
        .overflow_hidden()
        .flex()
        .flex_col()
        .when(show_auth_progress, |this| {
            this.child(auth_verification_progress())
        })
        .child(
            div()
                .flex_1()
                .min_h_0()
                .w_full()
                .p_4()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .child(provider_logo(&provider, &display_name, logo_size))
                .child(
                    div()
                        .text_sm()
                        .font_semibold()
                        .text_center()
                        .child(display_name),
                )
                .child(provider_status_tag(configured, status.has_oauth))
                .when(configured, |this| {
                    this.child(
                        krusty_button("remove-auth", "Remove auth", KrustyButtonKind::Danger, cx)
                            .w(px(220.0))
                            .on_click(move |_, _, cx| {
                                remove_view.update(cx, |view, cx| view.remove_selected_auth(cx));
                            }),
                    )
                })
                .when(
                    !configured && supports_oauth && auth_flow == AuthFlow::Choose,
                    |this| this.child(subscription_auth_picker(&status, view.clone(), cx)),
                )
                .when(show_paste_code, |this| {
                    this.child(oauth_paste_form(
                        oauth_code_input,
                        pending,
                        view.clone(),
                        cx,
                    ))
                })
                .when(!configured && show_api_key, |this| {
                    this.child(api_key_auth_form(api_key_input, pending, view, cx))
                }),
        )
        .with_animation(
            SharedString::from(format!("provider-auth-card-{provider}")),
            provider_card_animation(),
            |this, delta| this.opacity(delta).mt(px(-18.0) * (1.0 - delta)),
        )
        .into_any_element()
}

fn auth_verification_progress() -> impl IntoElement {
    div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .h(px(3.0))
        .overflow_hidden()
        .bg(theme::complement().opacity(0.18))
        .child(
            div()
                .absolute()
                .top_0()
                .left(px(-96.0))
                .h_full()
                .w(px(96.0))
                .bg(theme::complement())
                .with_animation(
                    "auth-verification-progress",
                    Animation::new(Duration::from_millis(1150)).repeat(),
                    |this, delta| this.left(px(-96.0 + (AUTH_PANEL_WIDTH + 96.0) * delta)),
                ),
        )
}

fn provider_status_tag(configured: bool, has_oauth: bool) -> impl IntoElement {
    let label = if has_oauth {
        "OAuth"
    } else if configured {
        "Configured"
    } else {
        "Not configured"
    };
    let tag = if configured || has_oauth {
        Tag::success()
    } else {
        Tag::secondary()
    };

    tag.outline()
        .rounded(px(0.0))
        .with_size(Size::Small)
        .child(label)
}

fn subscription_auth_picker(
    status: &ProviderStatus,
    view: Entity<KrustyDesktop>,
    cx: &mut gpui::App,
) -> AnyElement {
    let sign_in_view = view.clone();
    let device_view = view.clone();
    let api_key_view = view;
    let supports_browser = status.auth_methods.iter().any(|m| m == "oauth_browser");
    let supports_device = status.auth_methods.iter().any(|m| m == "oauth_device");
    let supports_api_key = provider_supports_api_key(status);

    div()
        .w(px(220.0))
        .flex()
        .flex_col()
        .items_center()
        .gap_2()
        .when(supports_browser, |this| {
            this.child(
                krusty_button("oauth-sign-in", "Sign in", KrustyButtonKind::Secondary, cx)
                    .w_full()
                    .on_click(move |_, _, cx| {
                        sign_in_view
                            .update(cx, |view, cx| view.start_oauth_login(Some("browser"), cx));
                    }),
            )
        })
        .when(supports_device, |this| {
            this.child(
                krusty_button(
                    "device-auth",
                    "Device code login",
                    KrustyButtonKind::Secondary,
                    cx,
                )
                .w_full()
                .on_click(move |_, _, cx| {
                    device_view.update(cx, |view, cx| view.start_oauth_login(Some("device"), cx));
                }),
            )
        })
        .when(supports_api_key, |this| {
            this.child(
                krusty_button("api-key-method", "API key", KrustyButtonKind::Ghost, cx)
                    .w_full()
                    .on_click(move |_, _, cx| {
                        api_key_view.update(cx, |view, cx| view.show_api_key_flow(cx));
                    }),
            )
        })
        .into_any_element()
}

fn provider_supports_api_key(status: &ProviderStatus) -> bool {
    status.auth_methods.iter().any(|m| m == "api_key")
}

fn oauth_paste_form(
    oauth_code_input: Entity<InputState>,
    pending: bool,
    view: Entity<KrustyDesktop>,
    cx: &mut gpui::App,
) -> AnyElement {
    let exchange_view = view;

    div()
        .w(px(220.0))
        .flex()
        .flex_col()
        .items_center()
        .gap_3()
        .child(
            div()
                .text_xs()
                .text_center()
                .text_color(theme::text_muted())
                .child("Paste the authorization code from your browser."),
        )
        .child(krusty_input(&oauth_code_input).w(px(220.0)).h(px(38.0)))
        .child(
            div().flex().gap_2().child(
                krusty_button(
                    "exchange-oauth",
                    "Submit code",
                    KrustyButtonKind::Primary,
                    cx,
                )
                .loading(pending)
                .on_click(move |_, _, cx| {
                    exchange_view.update(cx, |view, cx| view.exchange_oauth_code(cx));
                }),
            ),
        )
        .into_any_element()
}

fn api_key_auth_form(
    api_key_input: Entity<InputState>,
    pending: bool,
    view: Entity<KrustyDesktop>,
    cx: &mut gpui::App,
) -> AnyElement {
    let save_view = view;

    div()
        .w(px(220.0))
        .flex()
        .flex_col()
        .items_center()
        .gap_3()
        .child(krusty_input(&api_key_input).w(px(220.0)).h(px(38.0)))
        .child(
            div().flex().gap_2().child(
                krusty_button("submit-key", "Submit", KrustyButtonKind::Primary, cx)
                    .loading(pending)
                    .on_click(move |_, _, cx| {
                        save_view.update(cx, |view, cx| view.save_api_key(cx));
                    }),
            ),
        )
        .into_any_element()
}
