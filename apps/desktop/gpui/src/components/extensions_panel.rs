//! Plugins / Extensions surface — marketplace density matching Codex bar.
//!
//! Layout (bar-aligned):
//! - Title + subtitle
//! - Plugins | Skills tabs
//! - Search
//! - Installed icon strip
//! - Public | Personal chips
//! - Featured 2-col grid + Productivity / Creativity sections
//!
//! Data: `plugin/list` / `mcpServerStatus/list` / `skills/list` when Ready;
//! explicit fixture catalog offline. Empty live catalogs remain honestly empty.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _,
};
use gpui_component::{input::Input, Icon, IconName, Sizable as _};
use mitsuro_desktop_backend::{
    McpServerStatus, PluginMarketplaceEntry, PluginSummary, SkillMetadata,
};

use crate::app::{MitsuroApp, PluginsFilter, PluginsSurfaceTab, SurfaceDataState};
use crate::theme;

/// Full-height Plugins panel (sidebar "Plugins" → ProductMode::Extensions).
pub fn extensions_panel(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let mcp = app.mcp_servers().to_vec();
    let plugins = app.plugins().to_vec();
    let marketplaces = app.plugin_marketplaces().to_vec();
    let skills = app.skills().to_vec();
    let filter = app.plugins_filter();
    let tab = app.plugins_surface_tab();
    let chip = app.connection().chip_label();
    let data_state = app.extensions_state();
    let source = data_state.label();
    let mutations_available = app.plugin_mutations_available();
    let mutating_plugin_id = app.plugin_mutation_id().map(ToOwned::to_owned);
    let marketplace_management_available = app.marketplace_management_available();
    let marketplace_mutation = app.marketplace_mutation_id().map(ToOwned::to_owned);
    let marketplace_remove_confirmation =
        app.marketplace_remove_confirmation().map(ToOwned::to_owned);
    let marketplace_source_input = app.marketplace_source_input().clone();
    let marketplace_ref_input = app.marketplace_ref_input().clone();
    let marketplace_sparse_paths_input = app.marketplace_sparse_paths_input().clone();
    let skill_mutations_available = app.skill_mutations_available();
    let mutating_skill_id = app.skill_mutation_id().map(ToOwned::to_owned);
    let search_input = app.plugins_search_input().clone();
    let query = search_input.read(cx).value().trim().to_ascii_lowercase();
    let expanded_sections = app.expanded_plugin_sections().clone();

    let installed: Vec<PluginSummary> = plugins.iter().filter(|p| p.installed).cloned().collect();

    div()
        .id("extensions-panel")
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .bg(colors.bg_main)
        .child(header(
            chip,
            source,
            mcp.len(),
            plugins.len(),
            skills.len(),
            cx,
        ))
        .child(surface_tabs(tab, cx))
        .child(search_field(&search_input))
        .child(
            div()
                .id("extensions-body")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .w_full()
                .overflow_y_scroll()
                .px(px(28.0))
                .pb(px(32.0))
                .gap(px(18.0))
                .child(match tab {
                    PluginsSurfaceTab::Plugins => plugins_body(
                        &plugins,
                        &marketplaces,
                        &installed,
                        &mcp,
                        filter,
                        data_state,
                        mutations_available,
                        mutating_plugin_id.as_deref(),
                        marketplace_management_available,
                        marketplace_mutation.as_deref(),
                        marketplace_remove_confirmation.as_deref(),
                        &marketplace_source_input,
                        &marketplace_ref_input,
                        &marketplace_sparse_paths_input,
                        &query,
                        &expanded_sections,
                        cx,
                    )
                    .into_any_element(),
                    PluginsSurfaceTab::Skills => skills_body(
                        &skills,
                        source,
                        &query,
                        skill_mutations_available,
                        mutating_skill_id.as_deref(),
                        cx,
                    )
                    .into_any_element(),
                }),
        )
}

fn header(
    chip: &str,
    source: &str,
    mcp_n: usize,
    plugin_n: usize,
    skill_n: usize,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let source_owned = source.to_string();
    div()
        .id("plugins-header")
        .flex()
        .flex_row()
        .items_start()
        .justify_between()
        .px(px(28.0))
        .pt(px(24.0))
        .pb(px(10.0))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(
                    div()
                        .text_xl()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child("Plugins"),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(colors.text_tertiary)
                        .child(format!(
                            "Work with ChatGPT across your favorite tools · {source_owned} · \
                             {mcp_n} MCP · {plugin_n} plugin(s) · {skill_n} skill(s)"
                        )),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                // Secondary refresh (bar uses icon; we keep labeled secondary).
                .child(
                    div()
                        .id("plugins-refresh")
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_center()
                        .h(px(32.0))
                        .px(px(12.0))
                        .rounded(px(999.0))
                        .bg(colors.bg_button_secondary)
                        .border_1()
                        .border_color(colors.border)
                        .cursor_pointer()
                        .hover(|s| s.bg(colors.bg_hover))
                        .on_click(cx.listener(|app, _, window, cx| {
                            app.refresh_extensions(window, cx);
                        }))
                        .child(
                            div()
                                .text_xs()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.text)
                                .child("Refresh"),
                        ),
                )
                .when(
                    chip != "Offline" && chip != "Demo" && chip != "Fixture",
                    |this| {
                        this.child(
                            div()
                                .px(px(10.0))
                                .py(px(4.0))
                                .rounded(px(999.0))
                                .bg(colors.bg_elevated)
                                .border_1()
                                .border_color(colors.border)
                                .text_xs()
                                .text_color(colors.text_secondary)
                                .child(chip.to_string()),
                        )
                    },
                ),
        )
}

fn surface_tabs(active: PluginsSurfaceTab, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    div()
        .id("plugins-surface-tabs")
        .flex()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .px(px(28.0))
        .pb(px(12.0))
        .child(surface_tab(
            "plugins-tab-plugins",
            "Plugins",
            PluginsSurfaceTab::Plugins,
            active,
            cx,
        ))
        .child(surface_tab(
            "plugins-tab-skills",
            "Skills",
            PluginsSurfaceTab::Skills,
            active,
            cx,
        ))
}

fn surface_tab(
    id: &'static str,
    label: &str,
    tab: PluginsSurfaceTab,
    active: PluginsSurfaceTab,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let selected = tab == active;
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .h(px(30.0))
        .px(px(12.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .bg(if selected {
            colors.bg_selected
        } else {
            theme::transparent()
        })
        .hover(|s| s.bg(colors.bg_hover))
        .on_click(cx.listener(move |app, _, _, cx| {
            app.set_plugins_surface_tab(tab, cx);
        }))
        .child(
            div()
                .text_sm()
                .font_weight(if selected {
                    gpui::FontWeight::SEMIBOLD
                } else {
                    gpui::FontWeight::MEDIUM
                })
                .text_color(if selected {
                    colors.text
                } else {
                    colors.text_secondary
                })
                .child(label.to_string()),
        )
}

fn search_field(
    search_input: &gpui::Entity<gpui_component::input::InputState>,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("plugins-search")
        .mx(px(28.0))
        .mb(px(4.0))
        .h(px(36.0))
        .px(px(12.0))
        .rounded(px(10.0))
        .bg(theme::hex_alpha(0xffffff, 0.04))
        .border_1()
        .border_color(colors.border)
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .child(
            Icon::new(IconName::Search)
                .with_size(px(14.0))
                .text_color(colors.text_tertiary),
        )
        .child(
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .text_sm()
                .text_color(colors.text)
                .child(Input::new(search_input).appearance(false).h(px(28.0))),
        )
}

fn plugins_body(
    plugins: &[PluginSummary],
    marketplaces: &[PluginMarketplaceEntry],
    installed: &[PluginSummary],
    mcp: &[McpServerStatus],
    filter: PluginsFilter,
    data_state: SurfaceDataState,
    mutations_available: bool,
    mutating_plugin_id: Option<&str>,
    marketplace_management_available: bool,
    marketplace_mutation: Option<&str>,
    marketplace_remove_confirmation: Option<&str>,
    marketplace_source_input: &gpui::Entity<gpui_component::input::InputState>,
    marketplace_ref_input: &gpui::Entity<gpui_component::input::InputState>,
    marketplace_sparse_paths_input: &gpui::Entity<gpui_component::input::InputState>,
    query: &str,
    expanded_sections: &std::collections::HashSet<String>,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    // Marketplace chrome: hide internal `tools` fixtures (still available via plugin/read).
    let market: Vec<&PluginSummary> = plugins
        .iter()
        .filter(|p| {
            p.category() != "tools"
                && !p.name.starts_with("fixture-")
                && plugin_matches_query(p, query)
        })
        .collect();
    let featured: Vec<&PluginSummary> = market
        .iter()
        .copied()
        .filter(|p| p.category() == "featured")
        .collect();
    let productivity: Vec<&PluginSummary> = market
        .iter()
        .copied()
        .filter(|p| p.category() == "productivity")
        .collect();
    let creativity: Vec<&PluginSummary> = market
        .iter()
        .copied()
        .filter(|p| p.category() == "creativity")
        .collect();
    // Catch-all for unknown non-internal categories (not shown for tools).
    let other: Vec<&PluginSummary> = market
        .iter()
        .copied()
        .filter(|p| {
            let c = p.category();
            c != "featured" && c != "productivity" && c != "creativity"
        })
        .collect();

    // Brand strip only — ~12 geometric brand-mark chips (bar-like order).
    let filtered_installed: Vec<PluginSummary> = installed
        .iter()
        .filter(|plugin| plugin_matches_query(plugin, query))
        .cloned()
        .collect();
    let strip = brand_installed_strip(&filtered_installed);
    let market_empty = market.is_empty() && mcp.is_empty();
    let search_empty = !query.is_empty()
        && match filter {
            PluginsFilter::Mcp => !mcp.iter().any(|server| mcp_matches_query(server, query)),
            PluginsFilter::Personal => strip.is_empty(),
            PluginsFilter::Public => market.is_empty(),
        };

    div()
        .id("plugins-market")
        .flex()
        .flex_col()
        .gap(px(20.0))
        .child(installed_strip(&strip))
        .child(scope_chips(filter, cx))
        .when(search_empty, |this| {
            this.child(
                div()
                    .px(px(14.0))
                    .py(px(18.0))
                    .rounded(px(12.0))
                    .bg(colors_empty_card())
                    .border_1()
                    .border_color(theme::colors().border)
                    .text_sm()
                    .text_color(theme::colors().text_tertiary)
                    .child(format!("No results match “{query}”.")),
            )
        })
        .when(!search_empty && market_empty && data_state != SurfaceDataState::Fixture, |this| {
            this.child(
                div()
                    .px(px(14.0))
                    .py(px(18.0))
                    .rounded(px(12.0))
                    .bg(colors_empty_card())
                    .border_1()
                    .border_color(theme::colors().border)
                    .text_sm()
                    .text_color(theme::colors().text_tertiary)
                    .child(match data_state {
                        SurfaceDataState::Live => "The connected backend returned no plugin or MCP records. Skills are listed separately.",
                        SurfaceDataState::Loading => "Loading extensions from the connected backend.",
                        SurfaceDataState::Unsupported => "Extensions are not exposed by the connected backend.",
                        SurfaceDataState::Error => "The extension catalog could not be loaded from the connected backend.",
                        SurfaceDataState::Fixture => unreachable!(),
                    }),
            )
        })
        .child(if search_empty {
            div().into_any_element()
        } else if filter == PluginsFilter::Mcp {
            let filtered: Vec<McpServerStatus> = mcp
                .iter()
                .filter(|server| mcp_matches_query(server, query))
                .cloned()
                .collect();
            mcp_marketplace(&filtered).into_any_element()
        } else if filter == PluginsFilter::Personal {
            // Personal: brand installed only (no fixture chrome).
            let personal: Vec<PluginSummary> = strip.to_vec();
            personal_catalog(
                &personal,
                marketplaces,
                mutations_available,
                mutating_plugin_id,
                marketplace_management_available,
                marketplace_mutation,
                marketplace_remove_confirmation,
                marketplace_source_input,
                marketplace_ref_input,
                marketplace_sparse_paths_input,
                cx,
            )
            .into_any_element()
        } else {
            // Public marketplace: Featured + categories
            div()
                .flex()
                .flex_col()
                .gap(px(22.0))
                .when(!featured.is_empty(), |this| {
                    this.child(category_section(
                        "Featured",
                        &featured,
                        mutations_available,
                        mutating_plugin_id,
                        !query.is_empty() || expanded_sections.contains("Featured"),
                        cx,
                    ))
                })
                .when(!productivity.is_empty(), |this| {
                    this.child(category_section(
                        "Productivity",
                        &productivity,
                        mutations_available,
                        mutating_plugin_id,
                        !query.is_empty() || expanded_sections.contains("Productivity"),
                        cx,
                    ))
                })
                .when(!creativity.is_empty(), |this| {
                    this.child(category_section(
                        "Creativity",
                        &creativity,
                        mutations_available,
                        mutating_plugin_id,
                        !query.is_empty() || expanded_sections.contains("Creativity"),
                        cx,
                    ))
                })
                .when(!other.is_empty(), |this| {
                    this.child(category_section(
                        "More",
                        &other,
                        mutations_available,
                        mutating_plugin_id,
                        !query.is_empty() || expanded_sections.contains("More"),
                        cx,
                    ))
                })
                .into_any_element()
        })
}

fn colors_empty_card() -> gpui::Hsla {
    theme::colors().bg_elevated
}

/// Prefer bar-like brand order for the Installed chip rail; drop internal fixtures.
fn brand_installed_strip(installed: &[PluginSummary]) -> Vec<PluginSummary> {
    const PREFERRED: &[&str] = &[
        "documents",
        "pdf",
        "spreadsheets",
        "presentations",
        "notion",
        "canva",
        "chrome",
        "gmail",
        "google-drive",
        "google-calendar",
        "dropbox",
        "github",
    ];
    let mut out: Vec<PluginSummary> = Vec::new();
    for name in PREFERRED {
        if let Some(p) = installed
            .iter()
            .find(|p| p.name == *name && p.category() != "tools")
        {
            out.push(p.clone());
        }
    }
    for p in installed {
        if p.category() == "tools" || p.name.starts_with("fixture-") {
            continue;
        }
        if out.iter().any(|x| x.name == p.name) {
            continue;
        }
        out.push(p.clone());
        if out.len() >= 12 {
            break;
        }
    }
    out.truncate(12);
    out
}

fn installed_strip(installed: &[PluginSummary]) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("plugins-installed-strip")
        .flex()
        .flex_col()
        .w_full()
        .gap(px(10.0))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text)
                        .child("Installed"),
                )
                .child(
                    Icon::empty()
                        .path("icons/puzzle.svg")
                        .with_size(px(14.0))
                        .text_color(colors.text_tertiary),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .flex_wrap()
                .children(if installed.is_empty() {
                    vec![div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child("No plugins installed yet.")
                        .into_any_element()]
                } else {
                    installed
                        .iter()
                        .enumerate()
                        .map(|(i, p)| installed_icon(i as u64, p).into_any_element())
                        .collect()
                }),
        )
}

fn installed_icon(index: u64, plugin: &PluginSummary) -> impl IntoElement {
    // Geometric brand-mark chip — unique shape+color, not letter monograms.
    div()
        .id(("installed-icon", index))
        .child(brand_mark_chip(plugin.name.as_str(), 34.0))
}

fn scope_chips(active: PluginsFilter, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    div()
        .id("plugins-scope-chips")
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .child(scope_chip(
            "plugins-scope-public",
            "Public",
            PluginsFilter::Public,
            active,
            cx,
        ))
        .child(scope_chip(
            "plugins-scope-personal",
            "Personal",
            PluginsFilter::Personal,
            active,
            cx,
        ))
}

fn scope_chip(
    id: &'static str,
    label: &str,
    filter: PluginsFilter,
    active: PluginsFilter,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let selected = filter == active;
    let label = label.to_string();
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .h(px(28.0))
        .px(px(12.0))
        .rounded(px(999.0))
        .cursor_pointer()
        .bg(if selected {
            colors.bg_selected
        } else {
            theme::transparent()
        })
        .border_1()
        .border_color(if selected {
            colors.border_heavy
        } else {
            colors.border
        })
        .hover(|s| s.bg(colors.bg_hover))
        .on_click(cx.listener(move |app, _, _, cx| {
            app.set_plugins_filter(filter, cx);
        }))
        .child(
            div()
                .text_xs()
                .font_weight(if selected {
                    gpui::FontWeight::SEMIBOLD
                } else {
                    gpui::FontWeight::NORMAL
                })
                .text_color(if selected {
                    colors.text
                } else {
                    colors.text_secondary
                })
                .child(label),
        )
}

fn personal_catalog(
    installed: &[PluginSummary],
    marketplaces: &[PluginMarketplaceEntry],
    mutations_available: bool,
    mutating_plugin_id: Option<&str>,
    marketplace_management_available: bool,
    marketplace_mutation: Option<&str>,
    marketplace_remove_confirmation: Option<&str>,
    marketplace_source_input: &gpui::Entity<gpui_component::input::InputState>,
    marketplace_ref_input: &gpui::Entity<gpui_component::input::InputState>,
    marketplace_sparse_paths_input: &gpui::Entity<gpui_component::input::InputState>,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let refs: Vec<&PluginSummary> = installed.iter().collect();
    div()
        .id("plugins-personal")
        .flex()
        .flex_col()
        .gap(px(12.0))
        .child(section_heading("Your plugins"))
        .child(if refs.is_empty() {
            div()
                .px(px(14.0))
                .py(px(18.0))
                .rounded(px(12.0))
                .bg(colors.bg_elevated)
                .border_1()
                .border_color(colors.border)
                .text_sm()
                .text_color(colors.text_tertiary)
                .child("Install plugins from Public to see them here.")
                .into_any_element()
        } else {
            card_grid(&refs, 0, mutations_available, mutating_plugin_id, cx).into_any_element()
        })
        .when(
            marketplace_management_available || !marketplaces.is_empty(),
            |this| {
                this.child(marketplace_manager(
                    marketplaces,
                    marketplace_management_available,
                    marketplace_mutation,
                    marketplace_remove_confirmation,
                    marketplace_source_input,
                    marketplace_ref_input,
                    marketplace_sparse_paths_input,
                    cx,
                ))
            },
        )
}

#[allow(clippy::too_many_arguments)]
fn marketplace_manager(
    marketplaces: &[PluginMarketplaceEntry],
    available: bool,
    mutation: Option<&str>,
    remove_confirmation: Option<&str>,
    source_input: &gpui::Entity<gpui_component::input::InputState>,
    ref_input: &gpui::Entity<gpui_component::input::InputState>,
    sparse_paths_input: &gpui::Entity<gpui_component::input::InputState>,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let busy = mutation.is_some();
    let upgrade_label = if mutation == Some("upgrade") {
        "Upgrading…"
    } else {
        "Upgrade all"
    };

    div()
        .id("marketplace-manager")
        .flex()
        .flex_col()
        .gap(px(10.0))
        .pt(px(8.0))
        .border_t_1()
        .border_color(colors.border)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(section_heading("Marketplaces"))
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.text_tertiary)
                                .child("Sources used to discover and update plugins."),
                        ),
                )
                .child(
                    div()
                        .id("marketplaces-upgrade")
                        .h(px(28.0))
                        .px(px(11.0))
                        .flex()
                        .items_center()
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(colors.border)
                        .bg(colors.bg_button_secondary)
                        .when(available && !busy && !marketplaces.is_empty(), |this| {
                            this.cursor_pointer()
                                .hover(|style| style.bg(colors.bg_hover))
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.upgrade_plugin_marketplaces(cx);
                                }))
                        })
                        .when(!available || busy || marketplaces.is_empty(), |this| {
                            this.opacity(0.5)
                        })
                        .text_xs()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text)
                        .child(upgrade_label),
                ),
        )
        .children(marketplaces.iter().enumerate().map(|(index, marketplace)| {
            let name = marketplace.name.clone();
            let remove_key = format!("remove:{name}");
            let removing = mutation == Some(remove_key.as_str());
            let confirming = remove_confirmation == Some(name.as_str());
            let remove_label = if removing {
                "Removing…"
            } else if confirming {
                "Confirm remove"
            } else {
                "Remove"
            };
            let name_for_remove = name.clone();
            div()
                .id(("marketplace-row", index as u64))
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .w_full()
                .min_h(px(48.0))
                .gap(px(12.0))
                .py(px(7.0))
                .border_b_1()
                .border_color(theme::hex_alpha(0xffffff, 0.055))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .gap(px(1.0))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(colors.text)
                                .w_full()
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .child(name),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.text_tertiary)
                                .w_full()
                                .whitespace_nowrap()
                                .overflow_hidden()
                                .child(
                                    marketplace
                                        .path
                                        .clone()
                                        .unwrap_or_else(|| "Managed source".to_owned()),
                                ),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .when(confirming && !busy, |this| {
                            this.child(
                                div()
                                    .id(("marketplace-cancel-remove", index as u64))
                                    .h(px(26.0))
                                    .px(px(9.0))
                                    .flex()
                                    .items_center()
                                    .rounded(px(7.0))
                                    .cursor_pointer()
                                    .hover(|style| style.bg(colors.bg_hover))
                                    .on_click(cx.listener(|app, _, _, cx| {
                                        app.cancel_marketplace_removal(cx);
                                    }))
                                    .text_xs()
                                    .text_color(colors.text_secondary)
                                    .child("Cancel"),
                            )
                        })
                        .child(
                            div()
                                .id(("marketplace-remove", index as u64))
                                .h(px(26.0))
                                .px(px(9.0))
                                .flex()
                                .items_center()
                                .rounded(px(7.0))
                                .border_1()
                                .border_color(if confirming {
                                    colors.status_error
                                } else {
                                    colors.border
                                })
                                .when(available && !busy, |this| {
                                    this.cursor_pointer()
                                        .hover(|style| style.bg(colors.bg_hover))
                                        .on_click(cx.listener(move |app, _, _, cx| {
                                            app.remove_plugin_marketplace(
                                                name_for_remove.clone(),
                                                cx,
                                            );
                                        }))
                                })
                                .when(!available || busy, |this| this.opacity(0.5))
                                .text_xs()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(if confirming {
                                    colors.status_error
                                } else {
                                    colors.text_secondary
                                })
                                .child(remove_label),
                        ),
                )
                .into_any_element()
        }))
        .when(available, |this| {
            this.child(
                div()
                    .id("marketplace-add-form")
                    .flex()
                    .flex_col()
                    .gap(px(7.0))
                    .pt(px(4.0))
                    .child(marketplace_input("marketplace-source", source_input))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap(px(8.0))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(marketplace_input("marketplace-ref", ref_input)),
                            )
                            .child(div().flex_1().min_w_0().child(marketplace_input(
                                "marketplace-sparse-paths",
                                sparse_paths_input,
                            ))),
                    )
                    .child(
                        div().flex().flex_row().justify_end().child(
                            div()
                                .id("marketplace-add")
                                .h(px(28.0))
                                .px(px(12.0))
                                .flex()
                                .items_center()
                                .rounded(px(8.0))
                                .bg(colors.accent)
                                .when(!busy, |this| {
                                    this.cursor_pointer()
                                        .hover(|style| style.opacity(0.9))
                                        .on_click(cx.listener(|app, _, window, cx| {
                                            app.add_plugin_marketplace(window, cx);
                                        }))
                                })
                                .when(busy, |this| this.opacity(0.5))
                                .text_xs()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.fg_button_primary)
                                .child(if mutation == Some("add") {
                                    "Adding…"
                                } else {
                                    "Add marketplace"
                                }),
                        ),
                    ),
            )
        })
}

fn marketplace_input(
    id: &'static str,
    input: &gpui::Entity<gpui_component::input::InputState>,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id(id)
        .h(px(34.0))
        .px(px(10.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(colors.border)
        .bg(theme::hex_alpha(0xffffff, 0.03))
        .flex()
        .items_center()
        .text_sm()
        .text_color(colors.text)
        .child(Input::new(input).appearance(false).h(px(28.0)))
}

/// Max plugin cards shown per marketplace section (bar shows ~6 then "See N more").
const SECTION_VISIBLE_CAP: usize = 6;

fn category_section(
    title: &str,
    plugins: &[&PluginSummary],
    mutations_available: bool,
    mutating_plugin_id: Option<&str>,
    expanded: bool,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let (section_id, base): (&'static str, u64) = match title {
        "Featured" => ("plugins-section-featured", 0),
        "Productivity" => ("plugins-section-productivity", 100),
        "Creativity" => ("plugins-section-creativity", 200),
        _ => ("plugins-section-more", 300),
    };
    let visible: Vec<&PluginSummary> = if expanded {
        plugins.to_vec()
    } else {
        plugins.iter().copied().take(SECTION_VISIBLE_CAP).collect()
    };
    let overflow: Vec<&PluginSummary> = if expanded {
        Vec::new()
    } else {
        plugins.iter().copied().skip(SECTION_VISIBLE_CAP).collect()
    };
    div()
        .id(section_id)
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(section_heading(title))
        .child(card_grid(
            &visible,
            base,
            mutations_available,
            mutating_plugin_id,
            cx,
        ))
        .when(!overflow.is_empty(), |this| {
            this.child(see_more_row(title, base, &overflow, cx))
        })
}

/// Expand a marketplace section using the exact number of hidden server records.
fn see_more_row(
    section: &str,
    index_base: u64,
    overflow: &[&PluginSummary],
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let names: Vec<&str> = overflow
        .iter()
        .map(|plugin| plugin.display_name())
        .collect();
    let label = see_more_label(&names);
    let section_for_action = section.to_owned();
    // Mini geometric brand chips for first few overflow entries.
    let chips: Vec<_> = overflow
        .iter()
        .take(3)
        .enumerate()
        .map(|(i, p)| {
            div()
                .id(("see-more-chip", index_base + i as u64))
                .child(brand_mark_chip(p.name.as_str(), 18.0))
                .into_any_element()
        })
        .collect();
    div()
        .id(("plugins-see-more", index_base))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .pt(px(4.0))
        .pb(px(2.0))
        .cursor_pointer()
        .hover(|s| s.opacity(0.85))
        .on_click(cx.listener(move |app, _, _, cx| {
            app.expand_plugin_section(section_for_action.clone(), cx);
        }))
        .children(chips)
        .child(
            div()
                .text_xs()
                .text_color(colors.text_secondary)
                .child(label),
        )
}

fn see_more_label(names: &[&str]) -> String {
    match names {
        [] => "See more".to_owned(),
        [only] => format!("See {only}"),
        [first, second] => format!("See {first} and {second}"),
        [first, second, rest @ ..] => {
            format!("See {first}, {second}, and {} more", rest.len())
        }
    }
}

fn section_heading(title: &str) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .text_sm()
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(colors.text)
        .child(title.to_string())
}

fn card_grid(
    plugins: &[&PluginSummary],
    index_base: u64,
    mutations_available: bool,
    mutating_plugin_id: Option<&str>,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let rows: Vec<Vec<&PluginSummary>> = plugins.chunks(2).map(|c| c.to_vec()).collect();
    div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .children(rows.into_iter().enumerate().map(|(ri, pair)| {
            let left = pair[0];
            let right = pair.get(1).copied();
            let li = index_base + (ri as u64) * 2;
            let ri_idx = li + 1;
            div()
                .id(("plugin-row", index_base + ri as u64))
                .flex()
                .flex_row()
                .gap(px(12.0))
                .w_full()
                .child(div().flex_1().min_w_0().child(plugin_card(
                    li,
                    left,
                    mutations_available,
                    mutating_plugin_id,
                    cx,
                )))
                .child(if let Some(p) = right {
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(plugin_card(
                            ri_idx,
                            p,
                            mutations_available,
                            mutating_plugin_id,
                            cx,
                        ))
                        .into_any_element()
                } else {
                    div().flex_1().min_w_0().into_any_element()
                })
        }))
}

fn plugin_card(
    index: u64,
    plugin: &PluginSummary,
    mutations_available: bool,
    mutating_plugin_id: Option<&str>,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let title = plugin.display_name().to_string();
    let desc = plugin.short_description().unwrap_or("").to_string();
    let installed = plugin.installed;
    let plugin_mutable = plugin.availability
        == mitsuro_desktop_backend::PluginAvailability::Available
        && plugin.install_policy == mitsuro_desktop_backend::PluginInstallPolicy::Available;
    let busy = mutating_plugin_id.is_some();
    let this_mutating = mutating_plugin_id == Some(plugin.id.as_str());
    let plugin_for_action = plugin.clone();
    let action_label = if this_mutating {
        if installed {
            "Removing…"
        } else {
            "Installing…"
        }
    } else if !mutations_available {
        "Read-only"
    } else if !plugin_mutable {
        if installed {
            "Managed"
        } else {
            "Unavailable"
        }
    } else if installed {
        "Remove"
    } else {
        "Install"
    };
    let enabled = mutations_available && plugin_mutable && !busy;

    // Flat list-row (bar): geometric brand chip · name/desc · Install — no elevated card chrome.
    div()
        .id(("plugin-card", index))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(10.0))
        .px(px(4.0))
        .py(px(8.0))
        .rounded(px(8.0))
        .hover(|s| s.bg(theme::hex_alpha(0xffffff, 0.03)))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .flex_shrink_0()
                        .child(brand_mark_chip(plugin.name.as_str(), 32.0)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(1.0))
                        .min_w_0()
                        .flex_1()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.text)
                                .child(title),
                        )
                        .when(!desc.is_empty(), |this| {
                            this.child(
                                div()
                                    .text_xs()
                                    .text_color(colors.text_tertiary)
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .child(desc),
                            )
                        }),
                ),
        )
        .child(
            div()
                .id(("plugin-mutation", index))
                .flex()
                .flex_row()
                .items_center()
                .justify_center()
                .h(px(28.0))
                .px(px(12.0))
                .rounded(px(999.0))
                .flex_shrink_0()
                .bg(colors.bg_button_secondary)
                .border_1()
                .border_color(colors.border)
                .when(enabled, |this| {
                    this.cursor_pointer()
                        .hover(|s| s.bg(colors.bg_hover))
                        .on_click(cx.listener(move |app, _, _, cx| {
                            app.mutate_plugin(plugin_for_action.clone(), cx);
                        }))
                })
                .when(!enabled, |this| this.opacity(0.55))
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(colors.text)
                        .child(action_label),
                ),
        )
}

/// Geometric multi-color brand *language* for a plugin.
///
/// These are intentional geometric abstractions (not trademark bitmaps / letter monograms).
/// Multi-color segments read closer to product logos while remaining original shapes.
#[derive(Clone, Copy)]
enum MarkGeom {
    /// Docs: horizontal lines on brand plate.
    DocLines,
    /// PDF: page rect + folded corner.
    PageFold,
    /// Sheets: multi-cell green grid with light cells.
    Grid2x2,
    /// Slides: yellow rounded landscape rect.
    Landscape,
    /// Notion: black plate + white hollow frame.
    Frame,
    /// Soft circle / blob (Canva etc.).
    CircleDot,
    /// Chrome-like: four colored dots around a center.
    QuadDots,
    /// Gmail-like: red envelope with multi-color fold accents.
    Envelope,
    /// Drive-like: triangular multi-color band stack (decreasing width).
    TriangleBands,
    /// Dropbox / OneDrive: open box / stacked bands.
    StackBands,
    /// Calendar: blue header strip + date cell.
    CalBlock,
    /// SharePoint / Teams: nested square.
    NestedSquare,
    /// GitHub-like circle face.
    CircleFace,
    /// Linear: three vertical bars.
    Bars3,
    /// ClickUp: multi-color chevron / check accents.
    Chevron,
    /// Asana: three multi-color dots.
    Dots3,
    /// Figma: multi-color soft pills.
    CrossPills,
    /// Outlook: blue O-like ring (not a letter monogram alone).
    ORing,
    /// Concentric rings.
    Rings,
    /// Center diamond.
    CenterDiamond,
    /// Diagonal slash.
    Slash,
    /// Wave of three multi-color pills (Slack-ish).
    WavePills,
    /// Hash fallback: tint + inset circle.
    InsetDot,
}

fn mark_geom(name: &str) -> MarkGeom {
    match name {
        "documents" | "coda" | "confluence" | "evernote" | "obsidian" => MarkGeom::DocLines,
        "pdf" | "box" => MarkGeom::PageFold,
        "spreadsheets" | "airtable" | "monday" => MarkGeom::Grid2x2,
        "presentations" | "miro" | "gamma" => MarkGeom::Landscape,
        "notion" | "product-design" => MarkGeom::Frame,
        "canva" | "spotify" | "whatsapp" => MarkGeom::CircleDot,
        "chrome" => MarkGeom::QuadDots,
        "gmail" | "hubspot" => MarkGeom::Envelope,
        "outlook" => MarkGeom::ORing,
        "google-drive" => MarkGeom::TriangleBands,
        "onedrive" | "dropbox" => MarkGeom::StackBands,
        "google-calendar" | "todoist" => MarkGeom::CalBlock,
        "sharepoint" | "teams" | "jira" => MarkGeom::NestedSquare,
        "github" | "gitlab" | "bitbucket" => MarkGeom::CircleFace,
        "linear" | "trello" => MarkGeom::Bars3,
        "clickup" => MarkGeom::Chevron,
        "asana" => MarkGeom::Dots3,
        "figma" | "adobe" | "descript" => MarkGeom::CrossPills,
        "slack" | "discord" | "zoom" => MarkGeom::WavePills,
        "linkedin" | "youtube" | "reddit" => MarkGeom::Slash,
        "sentry" | "datadog" | "pagerduty" => MarkGeom::Rings,
        "zapier" | "make" | "salesforce" => MarkGeom::CenterDiamond,
        "x-twitter" => MarkGeom::Slash,
        _ => {
            // Stable hash → varied geometry so unknown plugins still look like marks.
            let mut h: u32 = 0;
            for b in name.bytes() {
                h = h.wrapping_mul(31).wrapping_add(b as u32);
            }
            match h % 8 {
                0 => MarkGeom::DocLines,
                1 => MarkGeom::Grid2x2,
                2 => MarkGeom::CircleDot,
                3 => MarkGeom::Bars3,
                4 => MarkGeom::Envelope,
                5 => MarkGeom::NestedSquare,
                6 => MarkGeom::Rings,
                _ => MarkGeom::InsetDot,
            }
        }
    }
}

/// Brand plate fill (light plates for multi-color marks so segments read clearly).
fn logo_tint(name: &str) -> u32 {
    match name {
        // Light plates for multi-color language
        "chrome" => 0xf1f3f4,
        "google-drive" => 0xf8f9fa,
        "gmail" => 0xea4335,
        "spreadsheets" => 0x0f9d58,
        "presentations" => 0xf4b400,
        "documents" => 0x4285f4,
        "pdf" => 0xe5252a,
        "outlook" => 0x0078d4,
        "github" => 0x24292f,
        "sharepoint" => 0x038387,
        "notion" => 0x191919,
        "google-calendar" => 0xffffff, // white plate; blue header strip inside
        "linear" => 0x5e6ad2,
        "clickup" => 0x7b68ee,
        "dropbox" => 0x0061ff,
        "asana" => 0xf8f9fa, // light plate for multi-color dots
        "canva" => 0x00c4cc,
        "figma" => 0x1e1e1e,
        "gamma" => 0x8b5cf6,
        "descript" => 0x6d28d9,
        "adobe" => 0xeb1000,
        "product-design" => 0x9333ea,
        "teams" => 0x6264a7,
        "onedrive" => 0x094ab2,
        "slack" => 0x4a154b,
        "zoom" => 0x2d8cff,
        "discord" => 0x5865f2,
        "whatsapp" => 0x25d366,
        "linkedin" => 0x0a66c2,
        "youtube" => 0xff0000,
        "spotify" => 0x1db954,
        "reddit" => 0xff4500,
        "x-twitter" => 0x111111,
        "jira" => 0x0052cc,
        "confluence" => 0x172b4d,
        "trello" => 0x0079bf,
        "monday" => 0xff3d57,
        "airtable" => 0x18bfff,
        "todoist" => 0xe44332,
        "evernote" => 0x00a82d,
        "coda" => 0xf46a54,
        "box" => 0x0061d5,
        "miro" => 0xffd02f,
        "hubspot" => 0xff7a59,
        "salesforce" => 0x00a1e0,
        "zapier" => 0xff4a00,
        "obsidian" => 0x7c3aed,
        "sentry" => 0x362d59,
        "datadog" => 0x632ca6,
        "fixture-review" => 0xf59e0b,
        "fixture-mcp-bridge" => 0xf97316,
        _ => {
            const PALETTE: &[u32] = &[
                0x4285f4, 0x34a853, 0xea4335, 0xfbbc04, 0xa855f7, 0x06b6d4, 0xf97316, 0xec4899,
                0x14b8a6, 0x6366f1,
            ];
            let mut h: u32 = 0;
            for b in name.bytes() {
                h = h.wrapping_mul(31).wrapping_add(b as u32);
            }
            PALETTE[(h as usize) % PALETTE.len()]
        }
    }
}

/// Ink color for monochrome mark strokes on a solid chip.
fn mark_ink(tint: u32) -> u32 {
    let r = ((tint >> 16) & 0xff) as f32;
    let g = ((tint >> 8) & 0xff) as f32;
    let b = (tint & 0xff) as f32;
    let lum = 0.299 * r + 0.587 * g + 0.114 * b;
    if lum > 165.0 {
        0x1a1a1a
    } else {
        0xffffff
    }
}

/// Unique multi-color geometric brand-mark chip. No trademark bitmaps / letter monograms.
fn brand_mark_chip(name: &str, size: f32) -> impl IntoElement {
    let tint = logo_tint(name);
    let ink = mark_ink(tint);
    let geom = mark_geom(name);
    let radius = (size * 0.26).clamp(4.0, 10.0);
    let s = size;
    // Subtle rim so light plates don't disappear on dark marketplace bg.
    let rim = if mark_ink(tint) == 0x1a1a1a {
        theme::hex_alpha(0xffffff, 0.10)
    } else {
        theme::hex_alpha(0x000000, 0.0)
    };

    div()
        .w(px(s))
        .h(px(s))
        .rounded(px(radius))
        .bg(theme::hex(tint))
        .border_1()
        .border_color(rim)
        .relative()
        .flex_shrink_0()
        .overflow_hidden()
        .child(mark_inner(name, geom, s, ink))
}

fn mark_inner(name: &str, geom: MarkGeom, s: f32, ink: u32) -> gpui::AnyElement {
    let p = (s * 0.18).max(2.0);
    let ink_c = theme::hex(ink);
    let ink_soft = theme::hex_alpha(ink, 0.82);

    match geom {
        MarkGeom::DocLines => {
            // Docs: three white lines (shortening) on blue plate.
            let bar_h = (s * 0.09).max(1.5);
            let bar_w = s - p * 2.0;
            let gap = (s * 0.12).max(2.0);
            let top = p + s * 0.10;
            div()
                .absolute()
                .inset_0()
                .child(bar_at(p, top, bar_w, bar_h, ink_c))
                .child(bar_at(p, top + gap + bar_h, bar_w * 0.90, bar_h, ink_soft))
                .child(bar_at(
                    p,
                    top + (gap + bar_h) * 2.0,
                    bar_w * 0.68,
                    bar_h,
                    ink_soft,
                ))
                .into_any_element()
        }
        MarkGeom::PageFold => {
            let body = s - p * 2.0;
            let fold = (s * 0.30).max(4.0);
            div()
                .absolute()
                .inset_0()
                .child(
                    div()
                        .absolute()
                        .top(px(p))
                        .left(px(p))
                        .w(px(body))
                        .h(px(body))
                        .rounded(px(2.0))
                        .bg(ink_c),
                )
                .child(
                    div()
                        .absolute()
                        .top(px(p))
                        .right(px(p))
                        .w(px(fold))
                        .h(px(fold))
                        .bg(theme::hex_alpha(0x000000, 0.18)),
                )
                .into_any_element()
        }
        MarkGeom::Grid2x2 => {
            // Sheets: green plate + light grid cells (spreadsheet language).
            let gap = (s * 0.06).max(1.5);
            let cell = ((s - p * 2.0 - gap) / 2.0).max(3.0);
            let light = theme::hex_alpha(0xffffff, 0.92);
            let light_soft = theme::hex_alpha(0xffffff, 0.72);
            div()
                .absolute()
                .inset_0()
                .child(cell_at(p, p, cell, cell, light))
                .child(cell_at(p + cell + gap, p, cell, cell, light_soft))
                .child(cell_at(p, p + cell + gap, cell, cell, light_soft))
                .child(cell_at(p + cell + gap, p + cell + gap, cell, cell, light))
                .into_any_element()
        }
        MarkGeom::Landscape => {
            // Slides: yellow rounded rect with soft white bar.
            let w = s - p * 2.0;
            let h = (s * 0.46).max(5.0);
            let y = (s - h) / 2.0;
            div()
                .absolute()
                .inset_0()
                .child(
                    div()
                        .absolute()
                        .top(px(y))
                        .left(px(p))
                        .w(px(w))
                        .h(px(h))
                        .rounded(px((h * 0.18).max(2.0)))
                        .bg(ink_c),
                )
                .child(
                    div()
                        .absolute()
                        .top(px(y + h * 0.38))
                        .left(px(p + w * 0.18))
                        .w(px(w * 0.64))
                        .h(px(h * 0.22))
                        .rounded(px(1.5))
                        .bg(theme::hex_alpha(logo_tint(name), 0.45)),
                )
                .into_any_element()
        }
        MarkGeom::Frame => {
            // Notion: white hollow frame on black plate.
            let inset = p * 0.95;
            div()
                .absolute()
                .inset_0()
                .child(
                    div()
                        .absolute()
                        .top(px(inset))
                        .left(px(inset))
                        .right(px(inset))
                        .bottom(px(inset))
                        .rounded(px(3.0))
                        .border_2()
                        .border_color(ink_c)
                        .bg(theme::transparent()),
                )
                // subtle inner corner tick (frame language, not a letter)
                .child(
                    div()
                        .absolute()
                        .top(px(inset + s * 0.16))
                        .left(px(inset + s * 0.16))
                        .w(px(s * 0.12))
                        .h(px(s * 0.08))
                        .rounded(px(1.0))
                        .bg(ink_soft),
                )
                .into_any_element()
        }
        MarkGeom::CircleDot => {
            let d = s * 0.52;
            div()
                .absolute()
                .inset_0()
                .child(
                    div()
                        .absolute()
                        .top(px((s - d) / 2.0))
                        .left(px((s - d) / 2.0))
                        .w(px(d))
                        .h(px(d))
                        .rounded_full()
                        .bg(ink_c),
                )
                .child(
                    div()
                        .absolute()
                        .top(px(s * 0.24))
                        .left(px(s * 0.22))
                        .w(px(s * 0.20))
                        .h(px(s * 0.20))
                        .rounded_full()
                        .bg(theme::hex_alpha(0xffffff, 0.40)),
                )
                .into_any_element()
        }
        MarkGeom::QuadDots => {
            // Chrome-like: four RGBY dots in a circle around a blue center.
            let d = (s * 0.24).max(3.5);
            let c = s / 2.0 - d / 2.0;
            let off = s * 0.24;
            div()
                .absolute()
                .inset_0()
                .child(dot_at(c, c - off, d, 0xea4335)) // red (top)
                .child(dot_at(c + off, c, d, 0xfbbc04)) // yellow (right)
                .child(dot_at(c, c + off, d, 0x34a853)) // green (bottom)
                .child(dot_at(c - off, c, d, 0x4285f4)) // blue (left)
                .child(
                    div()
                        .absolute()
                        .top(px(c + d * 0.12))
                        .left(px(c + d * 0.12))
                        .w(px(d * 0.76))
                        .h(px(d * 0.76))
                        .rounded_full()
                        .bg(theme::hex(0x4285f4)),
                )
                .into_any_element()
        }
        MarkGeom::Envelope => {
            // Gmail-like red envelope: white body + multi-color fold accents.
            let body_inset = p * 0.85;
            let body_w = s - body_inset * 2.0;
            let body_h = s * 0.48;
            let body_y = s * 0.28;
            let bar_h = (s * 0.07).max(1.5);
            // Multi-color flap accents (red/blue/green/yellow language — not a letter M).
            div()
                .absolute()
                .inset_0()
                // white envelope body
                .child(
                    div()
                        .absolute()
                        .top(px(body_y))
                        .left(px(body_inset))
                        .w(px(body_w))
                        .h(px(body_h))
                        .rounded(px(2.0))
                        .bg(theme::hex(0xffffff)),
                )
                // fold peak (center V approximation with three accent bars)
                .child(bar_at(
                    body_inset + body_w * 0.08,
                    body_y + body_h * 0.18,
                    body_w * 0.38,
                    bar_h,
                    theme::hex(0xea4335),
                ))
                .child(bar_at(
                    body_inset + body_w * 0.54,
                    body_y + body_h * 0.18,
                    body_w * 0.38,
                    bar_h,
                    theme::hex(0x4285f4),
                ))
                .child(bar_at(
                    body_inset + body_w * 0.30,
                    body_y + body_h * 0.42,
                    body_w * 0.40,
                    bar_h,
                    theme::hex(0x34a853),
                ))
                .child(bar_at(
                    body_inset + body_w * 0.42,
                    body_y + body_h * 0.62,
                    body_w * 0.16,
                    bar_h,
                    theme::hex(0xfbbc04),
                ))
                .into_any_element()
        }
        MarkGeom::TriangleBands => {
            // Drive-like: three multi-color bands tapering to a triangle silhouette.
            // Top (narrow blue), mid (green), base (yellow) — geometric, not trademark triangle.
            let colors = [0x4285f4_u32, 0x34a853, 0xfbbc04];
            let band_h = ((s - p * 2.0 - 4.0) / 3.0).max(2.5);
            let full_w = s - p * 2.0;
            let widths = [full_w * 0.42, full_w * 0.68, full_w];
            div()
                .absolute()
                .inset_0()
                .children((0..3).map(|i| {
                    let w = widths[i];
                    let x = p + (full_w - w) / 2.0;
                    let y = p + (band_h + 2.0) * i as f32;
                    bar_at(x, y, w, band_h, theme::hex(colors[i])).into_any_element()
                }))
                .into_any_element()
        }
        MarkGeom::StackBands => {
            let band_h = ((s - p * 2.0 - 4.0) / 3.0).max(2.5);
            let colors = match name {
                "dropbox" => [0xffffff_u32, 0xb3d4ff, 0x7eb6ff],
                "onedrive" => [0xffffff, 0xa0c4f0, 0x5b9bd5],
                _ => [ink, ink, ink],
            };
            div()
                .absolute()
                .inset_0()
                .child(bar_at(p, p, s - p * 2.0, band_h, theme::hex(colors[0])))
                .child(bar_at(
                    p,
                    p + band_h + 2.0,
                    s - p * 2.0,
                    band_h,
                    theme::hex(colors[1]),
                ))
                .child(bar_at(
                    p,
                    p + (band_h + 2.0) * 2.0,
                    s - p * 2.0,
                    band_h,
                    theme::hex(colors[2]),
                ))
                .into_any_element()
        }
        MarkGeom::CalBlock => {
            // Calendar: white body + blue header + dark date block (grid language).
            let inset = p * 0.9;
            let strip_h = (s * 0.24).max(3.5);
            let body_top = inset + strip_h;
            let body_h = s - body_top - inset;
            let header = match name {
                "google-calendar" => 0x1a73e8_u32,
                _ => ink,
            };
            div()
                .absolute()
                .inset_0()
                // outer card
                .child(
                    div()
                        .absolute()
                        .top(px(inset))
                        .left(px(inset))
                        .w(px(s - inset * 2.0))
                        .h(px(s - inset * 2.0))
                        .rounded(px(3.0))
                        .bg(theme::hex(0xf1f3f4))
                        .border_1()
                        .border_color(theme::hex_alpha(0x000000, 0.08)),
                )
                // blue header strip
                .child(
                    div()
                        .absolute()
                        .top(px(inset))
                        .left(px(inset))
                        .w(px(s - inset * 2.0))
                        .h(px(strip_h))
                        .rounded(px(3.0))
                        .bg(theme::hex(header)),
                )
                // date cell (reads as "day" without rendering a digit glyph)
                .child(
                    div()
                        .absolute()
                        .top(px(body_top + body_h * 0.18))
                        .left(px(inset + (s - inset * 2.0) * 0.22))
                        .w(px((s - inset * 2.0) * 0.56))
                        .h(px(body_h * 0.55))
                        .rounded(px(2.0))
                        .bg(theme::hex(0x202124)),
                )
                .into_any_element()
        }
        MarkGeom::NestedSquare => {
            let outer = s - p * 2.0;
            let inner = outer * 0.45;
            div()
                .absolute()
                .inset_0()
                .child(
                    div()
                        .absolute()
                        .top(px(p))
                        .left(px(p))
                        .w(px(outer))
                        .h(px(outer))
                        .rounded(px(3.0))
                        .border_2()
                        .border_color(ink_c)
                        .bg(theme::transparent()),
                )
                .child(
                    div()
                        .absolute()
                        .top(px((s - inner) / 2.0))
                        .left(px((s - inner) / 2.0))
                        .w(px(inner))
                        .h(px(inner))
                        .rounded(px(2.0))
                        .bg(ink_c),
                )
                .into_any_element()
        }
        MarkGeom::CircleFace => {
            let d = s * 0.55;
            div()
                .absolute()
                .inset_0()
                .child(
                    div()
                        .absolute()
                        .top(px((s - d) / 2.0 + s * 0.04))
                        .left(px((s - d) / 2.0))
                        .w(px(d))
                        .h(px(d))
                        .rounded_full()
                        .bg(ink_c),
                )
                .child(dot_at(s * 0.22, s * 0.18, s * 0.16, ink))
                .child(dot_at(s * 0.62, s * 0.18, s * 0.16, ink))
                .into_any_element()
        }
        MarkGeom::Bars3 => {
            let bar_w = (s * 0.14).max(2.0);
            let bar_h = s - p * 2.0;
            let gap = (s - p * 2.0 - bar_w * 3.0) / 2.0;
            div()
                .absolute()
                .inset_0()
                .child(bar_at(p, p, bar_w, bar_h * 0.7, ink_c))
                .child(bar_at(
                    p + bar_w + gap,
                    p + bar_h * 0.15,
                    bar_w,
                    bar_h * 0.85,
                    ink_c,
                ))
                .child(bar_at(
                    p + (bar_w + gap) * 2.0,
                    p + bar_h * 0.3,
                    bar_w,
                    bar_h * 0.7,
                    ink_soft,
                ))
                .into_any_element()
        }
        MarkGeom::Chevron => {
            // ClickUp-like multi-color check / chevron accents.
            let bar_h = (s * 0.12).max(2.0);
            let colors = [0xff2d55_u32, 0x7b68ee, 0x00d4ff];
            div()
                .absolute()
                .inset_0()
                .child(bar_at(
                    p,
                    p + s * 0.22,
                    s - p * 2.0,
                    bar_h,
                    theme::hex(colors[0]),
                ))
                .child(bar_at(
                    p + s * 0.08,
                    p + s * 0.42,
                    (s - p * 2.0) * 0.84,
                    bar_h,
                    theme::hex(colors[1]),
                ))
                .child(bar_at(
                    p + s * 0.16,
                    p + s * 0.62,
                    (s - p * 2.0) * 0.68,
                    bar_h,
                    theme::hex(colors[2]),
                ))
                .into_any_element()
        }
        MarkGeom::Dots3 => {
            // Asana-like: three multi-color dots in a triangle.
            let d = (s * 0.20).max(3.0);
            let colors = [0xf06a6a_u32, 0xf9bf34, 0x4573d2];
            div()
                .absolute()
                .inset_0()
                .child(dot_at(s * 0.22, s * 0.28, d, colors[0]))
                .child(dot_at(s * 0.58, s * 0.28, d, colors[1]))
                .child(dot_at(s * 0.40, s * 0.56, d, colors[2]))
                .into_any_element()
        }
        MarkGeom::CrossPills => {
            // Figma-like multi-color soft pills (not the trademarked logo).
            let pw = s * 0.28;
            let ph = s * 0.42;
            let colors = match name {
                "figma" => [0xf24e1e_u32, 0xff7262, 0xa259ff, 0x1abcfe],
                "adobe" => [0xffffff, 0xffffff, 0xffffff, 0xffffff],
                "descript" => [0xff6b6b, 0xffd166, 0xc084fc, 0x60a5fa],
                _ => [0xf472b6, 0xa78bfa, 0x60a5fa, 0x34d399],
            };
            div()
                .absolute()
                .inset_0()
                .child(
                    div()
                        .absolute()
                        .top(px(s * 0.12))
                        .left(px(s * 0.18))
                        .w(px(pw))
                        .h(px(ph))
                        .rounded(px(pw / 2.0))
                        .bg(theme::hex(colors[0])),
                )
                .child(
                    div()
                        .absolute()
                        .top(px(s * 0.12))
                        .left(px(s * 0.52))
                        .w(px(pw))
                        .h(px(ph))
                        .rounded(px(pw / 2.0))
                        .bg(theme::hex(colors[1])),
                )
                .child(
                    div()
                        .absolute()
                        .top(px(s * 0.42))
                        .left(px(s * 0.18))
                        .w(px(pw))
                        .h(px(ph * 0.85))
                        .rounded(px(pw / 2.0))
                        .bg(theme::hex(colors[2])),
                )
                .child(
                    div()
                        .absolute()
                        .top(px(s * 0.48))
                        .left(px(s * 0.52))
                        .w(px(pw))
                        .h(px(ph * 0.7))
                        .rounded(px(pw / 2.0))
                        .bg(theme::hex(colors[3])),
                )
                .into_any_element()
        }
        MarkGeom::ORing => {
            // Outlook: thick blue O-like ring on blue plate (ring geometry, not a letter alone).
            let outer = s * 0.62;
            let hole = outer * 0.42;
            div()
                .absolute()
                .inset_0()
                .child(
                    div()
                        .absolute()
                        .top(px((s - outer) / 2.0))
                        .left(px((s - outer) / 2.0))
                        .w(px(outer))
                        .h(px(outer))
                        .rounded_full()
                        .bg(theme::hex(0xffffff)),
                )
                .child(
                    div()
                        .absolute()
                        .top(px((s - hole) / 2.0))
                        .left(px((s - hole) / 2.0))
                        .w(px(hole))
                        .h(px(hole))
                        .rounded_full()
                        .bg(theme::hex(logo_tint(name))),
                )
                // soft envelope flap accent under the ring (mail language)
                .child(
                    div()
                        .absolute()
                        .top(px(s * 0.72))
                        .left(px(s * 0.28))
                        .w(px(s * 0.44))
                        .h(px(s * 0.08))
                        .rounded(px(1.5))
                        .bg(theme::hex_alpha(0xffffff, 0.55)),
                )
                .into_any_element()
        }
        MarkGeom::Rings => {
            let outer = s - p * 2.0;
            let mid = outer * 0.62;
            let inner = outer * 0.28;
            div()
                .absolute()
                .inset_0()
                .child(
                    div()
                        .absolute()
                        .top(px(p))
                        .left(px(p))
                        .w(px(outer))
                        .h(px(outer))
                        .rounded_full()
                        .border_2()
                        .border_color(ink_c)
                        .bg(theme::transparent()),
                )
                .child(
                    div()
                        .absolute()
                        .top(px((s - mid) / 2.0))
                        .left(px((s - mid) / 2.0))
                        .w(px(mid))
                        .h(px(mid))
                        .rounded_full()
                        .border_1()
                        .border_color(ink_soft)
                        .bg(theme::transparent()),
                )
                .child(
                    div()
                        .absolute()
                        .top(px((s - inner) / 2.0))
                        .left(px((s - inner) / 2.0))
                        .w(px(inner))
                        .h(px(inner))
                        .rounded_full()
                        .bg(ink_c),
                )
                .into_any_element()
        }
        MarkGeom::CenterDiamond => {
            let outer = s * 0.55;
            let inner = s * 0.28;
            div()
                .absolute()
                .inset_0()
                .child(
                    div()
                        .absolute()
                        .top(px((s - outer) / 2.0))
                        .left(px((s - outer) / 2.0))
                        .w(px(outer))
                        .h(px(outer))
                        .rounded(px(3.0))
                        .bg(ink_soft),
                )
                .child(
                    div()
                        .absolute()
                        .top(px((s - inner) / 2.0))
                        .left(px((s - inner) / 2.0))
                        .w(px(inner))
                        .h(px(inner))
                        .rounded(px(2.0))
                        .bg(ink_c),
                )
                .into_any_element()
        }
        MarkGeom::Slash => {
            let bar_w = (s * 0.16).max(2.5);
            let bar_h = s - p * 2.0;
            div()
                .absolute()
                .inset_0()
                .child(bar_at(s * 0.52, p, bar_w, bar_h, ink_c))
                .child(bar_at(
                    s * 0.28,
                    p + s * 0.12,
                    bar_w * 0.85,
                    bar_h * 0.75,
                    ink_soft,
                ))
                .into_any_element()
        }
        MarkGeom::WavePills => {
            // Slack-ish multi-color wave of rounded pills.
            let pw = s * 0.20;
            let ph = s * 0.38;
            let colors = match name {
                "slack" => [0xe01e5a_u32, 0x36c5f0, 0x2eb67d],
                "discord" => [0xffffff, 0xb5baf0, 0x7289da],
                "zoom" => [0xffffff, 0xa8d4ff, 0xffffff],
                _ => [ink, ink, ink],
            };
            div()
                .absolute()
                .inset_0()
                .child(
                    div()
                        .absolute()
                        .top(px(s * 0.32))
                        .left(px(p))
                        .w(px(pw))
                        .h(px(ph))
                        .rounded(px(pw / 2.0))
                        .bg(theme::hex(colors[0])),
                )
                .child(
                    div()
                        .absolute()
                        .top(px(s * 0.22))
                        .left(px(s * 0.40))
                        .w(px(pw))
                        .h(px(ph))
                        .rounded(px(pw / 2.0))
                        .bg(theme::hex(colors[1])),
                )
                .child(
                    div()
                        .absolute()
                        .top(px(s * 0.32))
                        .left(px(s - p - pw))
                        .w(px(pw))
                        .h(px(ph))
                        .rounded(px(pw / 2.0))
                        .bg(theme::hex(colors[2])),
                )
                .into_any_element()
        }
        MarkGeom::InsetDot => {
            let d = s * 0.38;
            div()
                .absolute()
                .inset_0()
                .child(
                    div()
                        .absolute()
                        .top(px((s - d) / 2.0))
                        .left(px((s - d) / 2.0))
                        .w(px(d))
                        .h(px(d))
                        .rounded_full()
                        .bg(ink_c),
                )
                .into_any_element()
        }
    }
}

fn bar_at(x: f32, y: f32, w: f32, h: f32, color: gpui::Hsla) -> impl IntoElement {
    div()
        .absolute()
        .top(px(y))
        .left(px(x))
        .w(px(w))
        .h(px(h))
        .rounded(px(1.5))
        .bg(color)
}

fn cell_at(x: f32, y: f32, w: f32, h: f32, color: gpui::Hsla) -> impl IntoElement {
    div()
        .absolute()
        .top(px(y))
        .left(px(x))
        .w(px(w))
        .h(px(h))
        .rounded(px(1.5))
        .bg(color)
}

fn dot_at(x: f32, y: f32, d: f32, color: u32) -> impl IntoElement {
    div()
        .absolute()
        .top(px(y))
        .left(px(x))
        .w(px(d))
        .h(px(d))
        .rounded_full()
        .bg(theme::hex(color))
}

fn skills_body(
    skills: &[SkillMetadata],
    source: &str,
    query: &str,
    mutations_available: bool,
    mutating_skill_id: Option<&str>,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let filtered: Vec<&SkillMetadata> = skills
        .iter()
        .filter(|skill| skill_matches_query(skill, query))
        .collect();
    let empty_msg = if source == "app-server" {
        "app-server · skills/list returned empty."
    } else {
        "No skills loaded."
    };
    div()
        .id("skills-market")
        .flex()
        .flex_col()
        .gap(px(12.0))
        .child(section_heading("Skills"))
        .child(
            div()
                .text_xs()
                .text_color(colors.text_tertiary)
                .child(if mutations_available {
                    format!(
                        "Reusable instructions Codex can load into a turn · {source} · Select a status to enable or disable"
                    )
                } else {
                    format!(
                        "Reusable instructions Codex can load into a turn · {source} · Read-only on the active backend"
                    )
                }),
        )
        .children(if filtered.is_empty() {
            vec![div()
                .px(px(14.0))
                .py(px(18.0))
                .rounded(px(12.0))
                .bg(colors.bg_elevated)
                .border_1()
                .border_color(colors.border)
                .text_sm()
                .text_color(colors.text_tertiary)
                .child(if query.is_empty() {
                    empty_msg.to_owned()
                } else {
                    format!("No skills match “{query}”.")
                })
                .into_any_element()]
        } else {
            filtered
                .into_iter()
                .enumerate()
                .map(|(i, skill)| {
                    skill_card(i as u64, skill, mutations_available, mutating_skill_id, cx)
                        .into_any_element()
                })
                .collect()
        })
}

fn plugin_matches_query(plugin: &PluginSummary, query: &str) -> bool {
    query.is_empty()
        || plugin.name.to_ascii_lowercase().contains(query)
        || plugin.display_name().to_ascii_lowercase().contains(query)
        || plugin.category().to_ascii_lowercase().contains(query)
        || plugin
            .short_description()
            .is_some_and(|description| description.to_ascii_lowercase().contains(query))
}

fn skill_matches_query(skill: &SkillMetadata, query: &str) -> bool {
    query.is_empty()
        || skill.name.to_ascii_lowercase().contains(query)
        || skill.description.to_ascii_lowercase().contains(query)
        || skill
            .short_description
            .as_ref()
            .is_some_and(|description| description.to_ascii_lowercase().contains(query))
        || skill.scope.to_ascii_lowercase().contains(query)
}

fn mcp_matches_query(server: &McpServerStatus, query: &str) -> bool {
    query.is_empty()
        || server.name.to_ascii_lowercase().contains(query)
        || server.server_info.as_ref().is_some_and(|info| {
            info.title
                .as_ref()
                .is_some_and(|title| title.to_ascii_lowercase().contains(query))
        })
        || server
            .tools
            .keys()
            .any(|name| name.to_ascii_lowercase().contains(query))
}

fn skill_card(
    index: u64,
    skill: &SkillMetadata,
    mutations_available: bool,
    mutating_skill_id: Option<&str>,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let title = skill
        .short_description
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| skill.name.clone());
    let desc = skill.description.clone();
    let scope = skill.scope.clone();
    let enabled = skill.enabled;
    let mutation_id = if skill.path.trim().is_empty() {
        skill.name.as_str()
    } else {
        skill.path.as_str()
    };
    let busy = mutating_skill_id == Some(mutation_id);
    let skill_for_action = skill.clone();

    div()
        .id(("skill-card", index))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(12.0))
        .px(px(14.0))
        .py(px(12.0))
        .rounded(px(12.0))
        .bg(theme::hex_alpha(0xffffff, 0.03))
        .border_1()
        .border_color(colors.border)
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .w(px(36.0))
                        .h(px(36.0))
                        .rounded(px(10.0))
                        .bg(theme::hex_alpha(0xa855f7, 0.14))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            Icon::empty()
                                .path("icons/sparkles.svg")
                                .with_size(px(16.0))
                                .text_color(theme::hex(0xa855f7)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .min_w_0()
                        .flex_1()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.text)
                                .child(title),
                        )
                        .when(!desc.is_empty(), |this| {
                            this.child(div().text_xs().text_color(colors.text_tertiary).child(desc))
                        })
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.text_tertiary)
                                .child(scope),
                        ),
                ),
        )
        .child(
            div()
                .id(("skill-toggle", index))
                .px(px(10.0))
                .py(px(4.0))
                .rounded(px(999.0))
                .bg(colors.bg_elevated)
                .border_1()
                .border_color(colors.border)
                .text_xs()
                .text_color(if enabled {
                    colors.status_ready
                } else {
                    colors.text_tertiary
                })
                .when(mutations_available && !busy, |this| {
                    this.cursor_pointer()
                        .hover(|style| style.bg(colors.bg_hover))
                        .on_click(cx.listener(move |app, _, _, cx| {
                            app.mutate_skill(skill_for_action.clone(), cx);
                        }))
                })
                .when(busy, |this| this.opacity(0.55))
                .child(if busy {
                    "Updating…"
                } else if enabled {
                    "Enabled"
                } else {
                    "Disabled"
                }),
        )
}

fn mcp_marketplace(servers: &[McpServerStatus]) -> impl IntoElement {
    div()
        .id("mcp-market")
        .flex()
        .flex_col()
        .gap(px(10.0))
        .child(section_heading("MCP servers"))
        .child(mcp_cards(servers))
}

fn mcp_cards(servers: &[McpServerStatus]) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("mcp-cards")
        .flex()
        .flex_col()
        .gap(px(8.0))
        .children(if servers.is_empty() {
            vec![div()
                .px(px(14.0))
                .py(px(12.0))
                .rounded(px(12.0))
                .bg(colors.bg_elevated)
                .border_1()
                .border_color(colors.border)
                .text_sm()
                .text_color(colors.text_tertiary)
                .child("No MCP servers loaded.")
                .into_any_element()]
        } else {
            servers
                .iter()
                .enumerate()
                .map(|(i, s)| mcp_card(i as u64, s).into_any_element())
                .collect()
        })
}

fn mcp_card(index: u64, server: &McpServerStatus) -> impl IntoElement {
    let colors = theme::colors();
    let title = server.display_title().to_string();
    let name = server.name.clone();
    let status = server.status_label();
    let auth = server.auth_status.as_str().to_string();
    let version = server
        .server_info
        .as_ref()
        .map(|i| i.version.clone())
        .unwrap_or_else(|| "—".into());
    let desc = server
        .server_info
        .as_ref()
        .and_then(|i| i.description.clone())
        .unwrap_or_default();
    let tools = server.tools.len();

    div()
        .id(("mcp-card", index))
        .flex()
        .flex_row()
        .items_start()
        .justify_between()
        .gap(px(12.0))
        .px(px(14.0))
        .py(px(12.0))
        .rounded(px(12.0))
        .bg(theme::hex_alpha(0xffffff, 0.03))
        .border_1()
        .border_color(colors.border)
        .child(
            div()
                .flex()
                .flex_row()
                .items_start()
                .gap(px(10.0))
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .w(px(36.0))
                        .h(px(36.0))
                        .rounded(px(10.0))
                        .bg(theme::hex_alpha(0x339cff, 0.12))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            Icon::new(IconName::Globe)
                                .with_size(px(16.0))
                                .text_color(colors.accent),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(3.0))
                        .min_w_0()
                        .flex_1()
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(8.0))
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(colors.text)
                                        .child(title),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .font_family("monospace")
                                        .text_color(colors.text_tertiary)
                                        .child(name),
                                ),
                        )
                        .when(!desc.is_empty(), |this| {
                            this.child(
                                div()
                                    .text_xs()
                                    .text_color(colors.text_secondary)
                                    .child(desc),
                            )
                        })
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.text_tertiary)
                                .child(format!("auth {auth} · v{version} · {tools} tool(s)")),
                        ),
                ),
        )
        .child(
            div()
                .px(px(8.0))
                .py(px(3.0))
                .rounded(px(6.0))
                .bg(colors.bg_sidebar)
                .border_1()
                .border_color(colors.border)
                .text_xs()
                .text_color(colors.text_secondary)
                .child(status),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_matches_real_plugin_skill_and_mcp_fields() {
        let plugin = mitsuro_desktop_backend::fixture_demo_plugins()
            .marketplaces
            .into_iter()
            .flat_map(|marketplace| marketplace.plugins)
            .find(|plugin| plugin.name == "documents")
            .expect("documents fixture plugin");
        assert!(plugin_matches_query(&plugin, "document"));
        assert!(!plugin_matches_query(&plugin, "definitely absent"));

        let skill = mitsuro_desktop_backend::fixture_demo_skills()
            .data
            .into_iter()
            .flat_map(|entry| entry.skills)
            .next()
            .expect("fixture skill");
        assert!(skill_matches_query(
            &skill,
            &skill.name.to_ascii_lowercase()
        ));

        let mcp = mitsuro_desktop_backend::fixture_demo_mcp_servers()
            .data
            .into_iter()
            .next()
            .expect("fixture MCP server");
        assert!(mcp_matches_query(&mcp, &mcp.name.to_ascii_lowercase()));
    }

    #[test]
    fn overflow_label_uses_only_the_exact_hidden_record_count() {
        assert_eq!(
            see_more_label(&["One", "Two", "Three", "Four", "Five"]),
            "See One, Two, and 3 more"
        );
    }
}
