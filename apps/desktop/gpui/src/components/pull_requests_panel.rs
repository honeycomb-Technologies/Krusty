//! Pull requests destination — two-pane list + detail matching bar PR surface.
//!
//! Left: filter chips (All / Reviewing / Authored), search, fixture PR rows.
//! All view groups rows under Authored / Reviewing section headers (bar residual).
//! Right: empty "Select pull request to view" or selected PR detail.
//!
//! Honesty: app-server has **no** `pullRequest/*` methods. Rows always come from
//! [`FIXTURE_PRS`]. When MCP GitHub is present in `mcpServerStatus/list`, surface that
//! under "GitHub via Codex connections" — never invent a live GitHub API.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, px, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _,
};
use gpui_component::{Icon, IconName, Sizable as _};

use crate::app::{MitsuroApp, PrFilter, UiConnection};
use crate::theme;

#[derive(Clone, Copy)]
struct FixturePr {
    title: &'static str,
    repo: &'static str,
    number: u32,
    status: PrStatus,
    author: &'static str,
    updated: &'static str,
    /// Whether the current user is a requested reviewer.
    reviewing: bool,
    /// Whether the current user authored the PR.
    authored: bool,
    additions: u32,
    deletions: u32,
    files_changed: u32,
    body: &'static str,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PrStatus {
    Open,
    Draft,
    Merged,
    Closed,
}

impl PrStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::Draft => "Draft",
            Self::Merged => "Merged",
            Self::Closed => "Closed",
        }
    }
}

impl FixturePr {
    fn matches_filter(self, filter: PrFilter) -> bool {
        match filter {
            PrFilter::All => true,
            PrFilter::Reviewing => self.reviewing,
            PrFilter::Authored => self.authored,
        }
    }
}

const FIXTURE_PRS: &[FixturePr] = &[
    FixturePr {
        title: "Harden Bullring website launch checklist",
        repo: "codex/prelaunch",
        number: 108,
        status: PrStatus::Open,
        author: "Burgess",
        updated: "3w",
        reviewing: false,
        authored: true,
        additions: 2108,
        deletions: 430,
        files_changed: 24,
        body: "Locks down the pre-launch surface: CSP headers, health checks, \
               and a dry-run deploy path for the Bullring marketing site.",
    },
    FixturePr {
        title: "Polish home chrome density to match Codex bar",
        repo: "honeycomb-Technologies/Mitsuro",
        number: 27,
        status: PrStatus::Open,
        author: "demo",
        updated: "2h",
        reviewing: true,
        authored: false,
        additions: 412,
        deletions: 88,
        files_changed: 11,
        body: "Tighten sidebar spacing, footer avatar chip, and main-column \
               padding so home matches the Codex bar reference.",
    },
    FixturePr {
        title: "Wire plugin marketplace install surface",
        repo: "honeycomb-Technologies/Mitsuro",
        number: 24,
        status: PrStatus::Merged,
        author: "demo",
        updated: "1d",
        reviewing: false,
        authored: true,
        additions: 980,
        deletions: 120,
        files_changed: 18,
        body: "Install / uninstall actions for local plugin fixtures plus \
               Featured / Productivity category chips.",
    },
    FixturePr {
        title: "Add scheduled tasks empty destination",
        repo: "mitsuro/desktop-fixtures",
        number: 9,
        status: PrStatus::Closed,
        author: "fixture",
        updated: "3d",
        reviewing: true,
        authored: false,
        additions: 220,
        deletions: 40,
        files_changed: 6,
        body: "Empty state + suggestion cards for Scheduled before live \
               task wiring lands.",
    },
    FixturePr {
        title: "Two-pane pull requests detail surface",
        repo: "honeycomb-Technologies/Mitsuro",
        number: 37,
        status: PrStatus::Open,
        author: "Burgess",
        updated: "1h",
        reviewing: false,
        authored: true,
        additions: 540,
        deletions: 95,
        files_changed: 4,
        body: "Left list with All / Reviewing / Authored filters; right pane \
               shows selection detail or empty prompt.",
    },
    FixturePr {
        title: "Sites empty state vertical centering",
        repo: "honeycomb-Technologies/Mitsuro",
        number: 31,
        status: PrStatus::Draft,
        author: "fixture",
        updated: "5d",
        reviewing: true,
        authored: true,
        additions: 64,
        deletions: 12,
        files_changed: 2,
        body: "Center the Sites empty CTA under the header/search chrome \
               without changing card density when fixtures are shown.",
    },
    FixturePr {
        title: "Review: settings personalization section map",
        repo: "mitsuro/desktop-fixtures",
        number: 14,
        status: PrStatus::Open,
        author: "reviewer",
        updated: "6h",
        reviewing: true,
        authored: false,
        additions: 310,
        deletions: 55,
        files_changed: 9,
        body: "Maps bar Personalization copy and control density into the \
               settings tree for offline demos.",
    },
];

/// Sparse default under All: only these Authored PRs (bar-pr-real shows #108).
const SPARSE_AUTHORED_CAP: usize = 1;

/// Full-height Pull requests panel (sidebar nav destination).
pub fn pull_requests_panel(app: &MitsuroApp, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let filter = app.pr_filter();
    let selected = app.selected_pr();
    let dense = app.pr_list_dense();
    let items: Vec<FixturePr> = FIXTURE_PRS
        .iter()
        .copied()
        .filter(|p| p.matches_filter(filter))
        .collect();
    let selected_pr = selected.and_then(|n| items.iter().copied().find(|p| p.number == n));
    let github_mcp = app
        .mcp_github_server()
        .map(|s| s.display_title().to_string());
    let live = matches!(app.connection(), UiConnection::Ready { .. });

    div()
        .id("pull-requests-panel")
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .bg(colors.bg_main)
        .child(
            div()
                .id("prs-body")
                .flex()
                .flex_row()
                .flex_1()
                .min_h_0()
                .w_full()
                .child(list_pane(
                    filter,
                    dense,
                    &items,
                    selected,
                    live,
                    github_mcp.as_deref(),
                    cx,
                ))
                .child(detail_pane(selected_pr.as_ref(), cx)),
        )
}

// ── Left list pane ───────────────────────────────────────────────────────────

fn list_pane(
    filter: PrFilter,
    dense: bool,
    items: &[FixturePr],
    selected: Option<u32>,
    live: bool,
    github_mcp: Option<&str>,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("prs-list-pane")
        .flex()
        .flex_col()
        .w(px(360.0))
        .min_w(px(320.0))
        .max_w(px(380.0))
        .h_full()
        .border_r_1()
        .border_color(colors.border)
        .bg(colors.bg_main)
        .child(list_header(filter, live, github_mcp, cx))
        .child(search_row())
        .child(pr_list(filter, dense, items, selected, cx))
}

fn list_header(
    filter: PrFilter,
    live: bool,
    github_mcp: Option<&str>,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    // App-server has no pullRequest/* — always fixture rows with explicit badges.
    let colors = theme::colors();
    let gh_label = github_mcp.map(|n| format!("GitHub MCP · {n}"));
    div()
        .id("prs-list-header")
        .flex()
        .flex_col()
        .gap(px(8.0))
        .px(px(14.0))
        .pt(px(14.0))
        .pb(px(8.0))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .child(
                    div()
                        .id("prs-filters")
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(2.0))
                        .child(filter_chip(
                            "pr-filter-all",
                            "All",
                            PrFilter::All,
                            filter,
                            cx,
                        ))
                        .child(filter_chip(
                            "pr-filter-reviewing",
                            "Reviewing",
                            PrFilter::Reviewing,
                            filter,
                            cx,
                        ))
                        .child(filter_chip(
                            "pr-filter-authored",
                            "Authored",
                            PrFilter::Authored,
                            filter,
                            cx,
                        )),
                )
                .child(
                    div()
                        .id("prs-fixture-badge")
                        .px(px(8.0))
                        .py(px(3.0))
                        .rounded(px(999.0))
                        .bg(theme::hex_alpha(0xf59e0b, 0.14))
                        .border_1()
                        .border_color(theme::hex_alpha(0xf59e0b, 0.35))
                        .text_xs()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(theme::hex(0xfbbf24))
                        .child("Fixture demo"),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .flex_wrap()
                .gap(px(6.0))
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child(if live {
                            "GitHub via Codex connections · no pullRequest/*"
                        } else {
                            "Offline · FIXTURE_PRS catalog"
                        }),
                )
                .when_some(gh_label, |this, label| {
                    this.child(
                        div()
                            .id("prs-github-mcp-badge")
                            .px(px(8.0))
                            .py(px(2.0))
                            .rounded(px(999.0))
                            .bg(theme::hex_alpha(0x3b82f6, 0.12))
                            .border_1()
                            .border_color(theme::hex_alpha(0x3b82f6, 0.3))
                            .text_xs()
                            .text_color(theme::hex(0x93c5fd))
                            .child(label),
                    )
                }),
        )
}

fn filter_chip(
    id: &'static str,
    label: &'static str,
    filter: PrFilter,
    active: PrFilter,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let selected = filter == active;
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .h(px(28.0))
        .px(px(10.0))
        .rounded(px(8.0))
        .cursor_pointer()
        .bg(if selected {
            colors.bg_selected
        } else {
            theme::transparent()
        })
        .hover(|s| s.bg(colors.bg_hover))
        .on_click(cx.listener(move |app, _, _, cx| {
            app.set_pr_filter(filter, cx);
        }))
        .child(
            div()
                .text_sm()
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

fn search_row() -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("prs-search")
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .px(px(14.0))
        .pb(px(10.0))
        .child(
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .h(px(32.0))
                .px(px(12.0))
                .rounded(px(999.0))
                .bg(colors.bg_elevated)
                .border_1()
                .border_color(colors.border)
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(
                    Icon::new(IconName::Search)
                        .with_size(px(13.0))
                        .text_color(colors.text_tertiary),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(colors.text_tertiary)
                        .child("Search pull requests"),
                ),
        )
        .child(
            div()
                .id("prs-filter-btn")
                .w(px(32.0))
                .h(px(32.0))
                .rounded(px(999.0))
                .bg(colors.bg_elevated)
                .border_1()
                .border_color(colors.border)
                .flex()
                .items_center()
                .justify_center()
                .child(
                    Icon::empty()
                        .path("icons/settings-2.svg")
                        .with_size(px(13.0))
                        .text_color(colors.text_tertiary),
                ),
        )
}

fn group_header(id: &'static str, label: &'static str) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id(id)
        .px(px(10.0))
        .pt(px(10.0))
        .pb(px(4.0))
        .text_xs()
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(colors.text_tertiary)
        .child(label)
}

fn pr_list(
    filter: PrFilter,
    dense: bool,
    items: &[FixturePr],
    selected: Option<u32>,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let mut children: Vec<gpui::AnyElement> = Vec::new();

    if items.is_empty() {
        children.push(
            div()
                .px(px(12.0))
                .py(px(24.0))
                .text_sm()
                .text_color(colors.text_tertiary)
                .text_center()
                .child("No pull requests in this filter")
                .into_any_element(),
        );
    } else if filter == PrFilter::All {
        // Bar residual: under All, group with Authored then Reviewing section headers.
        // Sparse default (like bar-pr-real): only Authored, 1 live-like row — no "Show N more".
        // Dense (`MITSURO_PR_DENSE=1` only): full Authored + Reviewing catalog.
        let authored_all: Vec<&FixturePr> = items.iter().filter(|p| p.authored).collect();
        let reviewing: Vec<&FixturePr> = items
            .iter()
            .filter(|p| p.reviewing && !p.authored)
            .collect();
        let authored: Vec<&FixturePr> = if dense {
            authored_all.clone()
        } else {
            authored_all
                .iter()
                .copied()
                .take(SPARSE_AUTHORED_CAP)
                .collect()
        };
        let mut idx: u64 = 0;
        if !authored.is_empty() {
            children.push(group_header("prs-group-authored", "Authored").into_any_element());
            for pr in authored {
                let is_sel = selected == Some(pr.number);
                children.push(pr_row(idx, pr, is_sel, cx).into_any_element());
                idx += 1;
            }
        }
        if dense {
            if !reviewing.is_empty() {
                children.push(group_header("prs-group-reviewing", "Reviewing").into_any_element());
                for pr in reviewing {
                    let is_sel = selected == Some(pr.number);
                    children.push(pr_row(idx, pr, is_sel, cx).into_any_element());
                    idx += 1;
                }
            }
            // Catch-all (neither authored nor reviewing) — rare for fixtures.
            let other: Vec<&FixturePr> = items
                .iter()
                .filter(|p| !p.authored && !p.reviewing)
                .collect();
            if !other.is_empty() {
                children.push(group_header("prs-group-other", "Other").into_any_element());
                for pr in other {
                    let is_sel = selected == Some(pr.number);
                    children.push(pr_row(idx, pr, is_sel, cx).into_any_element());
                    idx += 1;
                }
            }
        }
        // Sparse: intentionally no "Show N more" chrome (bar pure 1-row empty list).
    } else {
        let label = match filter {
            PrFilter::Reviewing => "Reviewing",
            PrFilter::Authored => "Authored",
            PrFilter::All => "All",
        };
        children.push(group_header("prs-group-single", label).into_any_element());
        for (i, pr) in items.iter().enumerate() {
            let is_sel = selected == Some(pr.number);
            children.push(pr_row(i as u64, pr, is_sel, cx).into_any_element());
        }
    }

    div()
        .id("prs-list")
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .px(px(8.0))
        .pb(px(12.0))
        .children(children)
}

/// Thousands-grouped count for bar-like +2,108 / −430.
fn format_count(n: u32) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    let rem = bytes.len() % 3;
    if rem > 0 {
        out.push_str(&s[..rem]);
    }
    let mut i = rem;
    while i < bytes.len() {
        if !out.is_empty() {
            out.push(',');
        }
        out.push_str(&s[i..i + 3]);
        i += 3;
    }
    out
}

/// Soft ellipsis for multi-line bar titles.
fn truncate_title(title: &str, max_chars: usize) -> String {
    let count = title.chars().count();
    if count <= max_chars {
        title.to_string()
    } else {
        let mut t: String = title.chars().take(max_chars.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

fn pr_row(
    index: u64,
    pr: &FixturePr,
    selected: bool,
    cx: &mut Context<MitsuroApp>,
) -> impl IntoElement {
    let colors = theme::colors();
    let number = pr.number;
    let icon_color = match pr.status {
        PrStatus::Open => colors.status_ready,
        PrStatus::Draft => colors.text_tertiary,
        PrStatus::Merged => theme::hex(0xc084fc),
        PrStatus::Closed => colors.status_error,
    };
    let title = truncate_title(pr.title, 32);
    // Bar meta: author · repo… · +2,108 −430 (no #number noise).
    let author_short = truncate_title(pr.author, 10);
    let repo_short = truncate_title(pr.repo, 16);
    let add = format_count(pr.additions);
    let del = format_count(pr.deletions);

    div()
        .id(("pr-row", index))
        .flex()
        .flex_col()
        .gap(px(4.0))
        .px(px(10.0))
        .py(px(10.0))
        .rounded(px(10.0))
        .cursor_pointer()
        .bg(if selected {
            colors.bg_selected
        } else {
            theme::transparent()
        })
        .hover(|s| s.bg(colors.bg_hover))
        .on_click(cx.listener(move |app, _, _, cx| {
            app.set_selected_pr(Some(number), cx);
        }))
        .child(
            div()
                .flex()
                .flex_row()
                .items_start()
                .justify_between()
                .gap(px(8.0))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_start()
                        .gap(px(8.0))
                        .min_w_0()
                        .flex_1()
                        .child(
                            Icon::empty()
                                .path("icons/git-pull-request.svg")
                                .with_size(px(14.0))
                                .text_color(icon_color)
                                .mt(px(2.0)),
                        )
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(colors.text)
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .child(title),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .flex_shrink_0()
                        .child(pr.updated),
                ),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .pl(px(22.0))
                .child(
                    div()
                        .text_xs()
                        .text_color(colors.text_tertiary)
                        .child(format!("{author_short}  {repo_short}")),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .child(
                            div()
                                .text_xs()
                                .font_family("monospace")
                                .text_color(colors.diff_add)
                                .child(format!("+{add}")),
                        )
                        .child(
                            div()
                                .text_xs()
                                .font_family("monospace")
                                .text_color(colors.diff_del)
                                .child(format!("-{del}")),
                        ),
                ),
        )
}

// ── Right detail pane ────────────────────────────────────────────────────────

fn detail_pane(pr: Option<&FixturePr>, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("prs-detail-pane")
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .h_full()
        .bg(colors.bg_main)
        .child(match pr {
            Some(pr) => detail_content(pr, cx).into_any_element(),
            None => empty_detail().into_any_element(),
        })
}

fn empty_detail() -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id("prs-detail-empty")
        .flex()
        .flex_1()
        .min_h_0()
        .items_center()
        .justify_center()
        .child(
            div()
                .text_sm()
                .text_color(colors.text_tertiary)
                .child("Select pull request to view"),
        )
}

fn detail_content(pr: &FixturePr, cx: &mut Context<MitsuroApp>) -> impl IntoElement {
    let colors = theme::colors();
    let (badge_bg, badge_fg) = match pr.status {
        PrStatus::Open => (theme::hex_alpha(0x04b84c, 0.14), colors.status_ready),
        PrStatus::Draft => (theme::hex_alpha(0xffffff, 0.08), colors.text_secondary),
        PrStatus::Merged => (theme::hex_alpha(0xa855f7, 0.16), theme::hex(0xc084fc)),
        PrStatus::Closed => (theme::hex_alpha(0xfa423e, 0.12), colors.status_error),
    };
    let number = pr.number;
    let title = pr.title;
    let repo = pr.repo;
    let author = pr.author;
    let updated = pr.updated;
    let status_label = pr.status.label();
    let body = pr.body;
    let files = pr.files_changed;
    let adds = pr.additions;
    let dels = pr.deletions;

    div()
        .id("prs-detail-content")
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .px(px(32.0))
        .py(px(28.0))
        .gap(px(18.0))
        // Title row
        .child(
            div()
                .flex()
                .flex_row()
                .items_start()
                .justify_between()
                .gap(px(16.0))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.0))
                        .min_w_0()
                        .flex_1()
                        .child(
                            div()
                                .text_xl()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(colors.text)
                                .child(title),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(8.0))
                                .child(
                                    div()
                                        .px(px(8.0))
                                        .py(px(3.0))
                                        .rounded(px(6.0))
                                        .bg(badge_bg)
                                        .text_xs()
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(badge_fg)
                                        .child(status_label),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .font_family("monospace")
                                        .text_color(colors.text_tertiary)
                                        .child(format!("{repo}#{number}")),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(colors.text_tertiary)
                                        .child(format!("· {author} · {updated}")),
                                ),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(action_btn(
                            "prs-review",
                            "Review",
                            true,
                            cx,
                            move |app, _, cx| {
                                app.set_status_line(
                                    format!("Pull requests · review #{number}"),
                                    cx,
                                );
                            },
                        ))
                        .child(action_btn(
                            "prs-open",
                            "Open",
                            false,
                            cx,
                            move |app, _, cx| {
                                app.set_status_line(
                                    format!("Pull requests · open #{number} (fixture)"),
                                    cx,
                                );
                            },
                        )),
                ),
        )
        // Meta strip
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(16.0))
                .px(px(14.0))
                .py(px(12.0))
                .rounded(px(10.0))
                .bg(colors.bg_elevated)
                .border_1()
                .border_color(colors.border)
                .child(meta_stat(
                    "Files",
                    format!("{files} changed"),
                    colors.text_secondary,
                ))
                .child(meta_divider())
                .child(meta_stat("Additions", format!("+{adds}"), colors.diff_add))
                .child(meta_divider())
                .child(meta_stat("Deletions", format!("−{dels}"), colors.diff_del)),
        )
        // Body
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text_tertiary)
                        .child("DESCRIPTION"),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(colors.text_secondary)
                        .child(body),
                ),
        )
        // Checks strip
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text_tertiary)
                        .child("CHECKS"),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.0))
                        .px(px(12.0))
                        .py(px(10.0))
                        .rounded(px(10.0))
                        .bg(colors.bg_elevated)
                        .border_1()
                        .border_color(colors.border)
                        .child(check_row("CI", "Passing", colors.status_ready))
                        .child(check_row("Lint", "Passing", colors.status_ready))
                        .child(check_row(
                            "Review",
                            if pr.reviewing { "Requested" } else { "None" },
                            if pr.reviewing {
                                colors.status_connecting
                            } else {
                                colors.text_tertiary
                            },
                        )),
                ),
        )
        // Files changed list (fixture stubs)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(colors.text_tertiary)
                        .child(format!("FILES · {files}")),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .px(px(10.0))
                        .py(px(8.0))
                        .rounded(px(10.0))
                        .bg(colors.bg_elevated)
                        .border_1()
                        .border_color(colors.border)
                        .children(
                            fixture_files_for(pr)
                                .into_iter()
                                .map(|(path, a, d)| file_row(path, a, d).into_any_element()),
                        ),
                ),
        )
        .child(
            div()
                .mt(px(4.0))
                .text_xs()
                .text_color(colors.text_tertiary)
                .child("Mitsuro · pull request detail"),
        )
}

fn check_row(name: &'static str, status: &'static str, color: gpui::Hsla) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(div().size(px(7.0)).rounded_full().bg(color))
                .child(
                    div()
                        .text_sm()
                        .text_color(colors.text_secondary)
                        .child(name),
                ),
        )
        .child(
            div()
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(color)
                .child(status),
        )
}

fn file_row(path: &'static str, adds: u32, dels: u32) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(12.0))
        .px(px(6.0))
        .py(px(5.0))
        .rounded(px(6.0))
        .child(
            div()
                .text_xs()
                .font_family("monospace")
                .text_color(colors.text_secondary)
                .min_w_0()
                .child(path),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .flex_shrink_0()
                .child(
                    div()
                        .text_xs()
                        .font_family("monospace")
                        .text_color(colors.diff_add)
                        .child(format!("+{adds}")),
                )
                .child(
                    div()
                        .text_xs()
                        .font_family("monospace")
                        .text_color(colors.diff_del)
                        .child(format!("−{dels}")),
                ),
        )
}

/// Stable fixture file stubs per PR number (rich detail pane).
fn fixture_files_for(pr: &FixturePr) -> Vec<(&'static str, u32, u32)> {
    match pr.number {
        108 => vec![
            ("apps/site/headers.ts", 420, 12),
            ("apps/site/health.ts", 180, 4),
            ("deploy/bullring.yml", 96, 22),
            ("docs/prelaunch.md", 140, 18),
        ],
        27 => vec![
            ("src/components/sidebar.rs", 180, 40),
            ("src/components/main_column.rs", 120, 28),
            ("src/theme.rs", 40, 8),
        ],
        24 => vec![
            ("src/components/extensions_panel.rs", 640, 80),
            ("src/app.rs", 120, 20),
            ("assets/icons/plugin.svg", 12, 0),
        ],
        37 => vec![
            ("src/components/pull_requests_panel.rs", 480, 60),
            ("src/app.rs", 40, 20),
            ("src/components/mod.rs", 8, 4),
        ],
        _ => vec![
            ("src/lib.rs", pr.additions / 2, pr.deletions / 2),
            ("README.md", pr.additions / 4, pr.deletions / 4),
            ("Cargo.toml", 12, 2),
        ],
    }
}

fn meta_stat(label: &'static str, value: String, value_color: gpui::Hsla) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(
            div()
                .text_xs()
                .text_color(colors.text_tertiary)
                .child(label),
        )
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(value_color)
                .child(value),
        )
}

fn meta_divider() -> impl IntoElement {
    let colors = theme::colors();
    div().w(px(1.0)).h(px(28.0)).bg(colors.border_heavy)
}

fn action_btn(
    id: &'static str,
    label: &'static str,
    primary: bool,
    cx: &mut Context<MitsuroApp>,
    on_click: impl Fn(&mut MitsuroApp, &mut gpui::Window, &mut Context<MitsuroApp>) + 'static,
) -> impl IntoElement {
    let colors = theme::colors();
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .justify_center()
        .h(px(32.0))
        .px(px(14.0))
        .rounded(px(999.0))
        .cursor_pointer()
        .bg(if primary {
            colors.bg_button_primary
        } else {
            colors.bg_button_secondary
        })
        .border_1()
        .border_color(if primary {
            theme::transparent()
        } else {
            colors.border_heavy
        })
        .hover(|s| {
            if primary {
                s.bg(colors.bg_button_primary_hover)
            } else {
                s.bg(colors.bg_hover)
            }
        })
        .on_click(cx.listener(move |app, _, window, cx| {
            on_click(app, window, cx);
        }))
        .child(
            div()
                .text_xs()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(if primary {
                    colors.fg_button_primary
                } else {
                    colors.text_secondary
                })
                .child(label),
        )
}
