//! Quiet service-backed inspectors and the wide workspace dock.

use ratatui::{
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui_v2::{
    app::state::{PickerUiState, UiState},
    components::primitive::dock_chrome::paint_dock_panel,
    layout::snapshot::{LayoutRegionId, LayoutSnapshot},
    model::{
        capability::{CapabilityProfile, GlyphMode},
        focus::FocusTarget,
    },
    motion::preference::MotionPreference,
    presentation::theme::{SemanticTheme, ThemeKind},
    services::{ExtensionRow, PlanSnapshot, ProcessRow},
};

fn row<'a>(
    active: bool,
    label: impl Into<Span<'a>>,
    meta: impl Into<Span<'a>>,
    glyph_mode: GlyphMode,
    theme: SemanticTheme,
) -> Line<'a> {
    Line::from(vec![
        Span::styled(
            if active {
                if glyph_mode == GlyphMode::Ascii {
                    "> "
                } else {
                    "› "
                }
            } else {
                "  "
            },
            Style::default().fg(theme.identity),
        ),
        label.into().style(
            Style::default()
                .fg(theme.foreground)
                .add_modifier(if active {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ),
        Span::raw("  "),
        meta.into()
            .style(Style::default().fg(theme.foreground_muted)),
    ])
}

pub fn render_processes(
    frame: &mut Frame,
    area: Rect,
    processes: &[ProcessRow],
    picker: &PickerUiState,
    glyph_mode: GlyphMode,
    theme: SemanticTheme,
) {
    let separator = if glyph_mode == GlyphMode::Ascii {
        " | "
    } else {
        " · "
    };
    let mut lines = Vec::new();
    if processes.is_empty() {
        lines.push(Line::styled(
            "No running or recent background processes.",
            Style::default().fg(theme.foreground_muted),
        ));
    } else {
        let available = usize::from(area.height)
            .saturating_sub(usize::from(picker.error.is_some()))
            .max(1);
        let selected = picker.selected.min(processes.len().saturating_sub(1));
        for (index, process) in processes.iter().enumerate().take(available) {
            lines.push(row(
                index == selected,
                process.command.clone(),
                format!("{}{separator}{}s", process.status, process.elapsed_seconds),
                glyph_mode,
                theme,
            ));
        }
    }
    if let Some(error) = picker.error.as_deref() {
        lines.push(Line::styled(error, Style::default().fg(theme.error)));
    }
    paint(frame, area, lines, theme);
}

pub fn render_plan(
    frame: &mut Frame,
    area: Rect,
    plan: Option<&PlanSnapshot>,
    glyph_mode: GlyphMode,
    theme: SemanticTheme,
) {
    let separator = if glyph_mode == GlyphMode::Ascii {
        " | "
    } else {
        " · "
    };
    let lines = if let Some(plan) = plan {
        vec![
            Line::styled(
                plan.title.as_str(),
                Style::default()
                    .fg(theme.foreground)
                    .add_modifier(Modifier::BOLD),
            ),
            Line::styled(
                format!(
                    "{}{separator}{}/{} steps",
                    plan.status, plan.completed_steps, plan.total_steps
                ),
                Style::default().fg(theme.foreground_muted),
            ),
            Line::default(),
            Line::styled(
                plan.objective.as_str(),
                Style::default().fg(theme.foreground),
            ),
            Line::default(),
            Line::styled(
                plan.current_step
                    .as_deref()
                    .unwrap_or("No step is currently in progress."),
                Style::default().fg(theme.identity),
            ),
        ]
    } else {
        vec![Line::styled(
            "This conversation has no durable Goal or plan yet.",
            Style::default().fg(theme.foreground_muted),
        )]
    };
    paint(frame, area, lines, theme);
}

/// Wide workspace dock: title-less purple twin panels (plan over plugin).
pub fn render_workspace_sidebar(
    frame: &mut Frame,
    layout: &LayoutSnapshot,
    state: &UiState,
    plan: Option<&PlanSnapshot>,
    capability: CapabilityProfile,
    theme: SemanticTheme,
) {
    let Some(inspector) = layout.region(LayoutRegionId::Inspector) else {
        return;
    };
    if inspector.is_empty() {
        return;
    }

    let plan_area = layout.region(LayoutRegionId::PlanDock).unwrap_or(inspector);
    let plugin_area = layout.region(LayoutRegionId::PluginDock);
    let plugin_focused =
        state.dock.plugin_focused || matches!(state.focus, FocusTarget::PluginDock);

    render_plan_dock(frame, plan_area, plan, plugin_focused, capability, theme);

    if let Some(plugin) = plugin_area.filter(|area| area.height > 0) {
        render_plugin_dock(frame, plugin, plugin_focused, capability, theme);
    }
}

fn render_plan_dock(
    frame: &mut Frame,
    area: Rect,
    plan: Option<&PlanSnapshot>,
    compressed: bool,
    capability: CapabilityProfile,
    theme: SemanticTheme,
) {
    if area.is_empty() {
        return;
    }
    let inner = paint_dock_panel(frame, area, false, theme, capability);
    if inner.is_empty() {
        return;
    }
    let separator = if capability.glyph_mode == GlyphMode::Ascii {
        " | "
    } else {
        " · "
    };
    let mut lines = Vec::new();
    if let Some(plan) = plan {
        lines.push(Line::styled(
            plan.title.as_str(),
            Style::default()
                .fg(theme.foreground)
                .add_modifier(Modifier::BOLD),
        ));
        lines.push(Line::styled(
            format!(
                "{}{separator}{}/{}",
                plan.status, plan.completed_steps, plan.total_steps
            ),
            Style::default().fg(theme.foreground_muted),
        ));
        if !compressed || inner.height > 4 {
            lines.push(Line::default());
            lines.push(Line::styled(
                plan.current_step
                    .as_deref()
                    .unwrap_or("No step in progress."),
                Style::default().fg(theme.foreground),
            ));
        }
        if !compressed && inner.height > 8 {
            if let Some(objective) = plan.objective.lines().next() {
                if !objective.is_empty() {
                    lines.push(Line::default());
                    lines.push(Line::styled(
                        objective,
                        Style::default().fg(theme.foreground_muted),
                    ));
                }
            }
        }
    } else {
        lines.push(Line::styled(
            "No active goal",
            Style::default().fg(theme.foreground_muted),
        ));
        if inner.height > 3 {
            lines.push(Line::styled(
                "/plan to start",
                Style::default().fg(theme.foreground_muted),
            ));
        }
    }
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().fg(theme.foreground).bg(theme.surface)),
        inner,
    );
}

fn render_plugin_dock(
    frame: &mut Frame,
    area: Rect,
    focused: bool,
    capability: CapabilityProfile,
    theme: SemanticTheme,
) {
    if area.is_empty() {
        return;
    }
    let inner = paint_dock_panel(frame, area, focused, theme, capability);
    if inner.is_empty() {
        return;
    }

    // Live classic plugin runtime is wired in a follow-up; this well is the
    // reserved surface with empty state and available ids when present.
    let available = crate::tui::plugins::available_plugin_ids();
    let mut lines = Vec::new();
    if available.is_empty() {
        lines.push(Line::styled(
            "No plugin loaded",
            Style::default().fg(theme.foreground_muted),
        ));
        lines.push(Line::default());
        lines.push(Line::styled(
            "/extensions",
            Style::default().fg(theme.foreground_muted),
        ));
    } else {
        lines.push(Line::styled(
            "Plugins ready",
            Style::default().fg(theme.foreground_muted),
        ));
        lines.push(Line::default());
        for id in available
            .into_iter()
            .take(usize::from(inner.height.saturating_sub(3)).max(1))
        {
            lines.push(Line::styled(id, Style::default().fg(theme.foreground)));
        }
    }

    let content_height = u16::try_from(lines.len())
        .unwrap_or(u16::MAX)
        .min(inner.height);
    let content = Rect::new(
        inner.x,
        inner
            .y
            .saturating_add(inner.height.saturating_sub(content_height) / 2),
        inner.width,
        content_height,
    );
    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme.foreground).bg(theme.surface)),
        content,
    );
}

pub fn render_extensions(
    frame: &mut Frame,
    area: Rect,
    extensions: &[ExtensionRow],
    picker: &PickerUiState,
    glyph_mode: GlyphMode,
    theme: SemanticTheme,
) {
    let separator = if glyph_mode == GlyphMode::Ascii {
        " | "
    } else {
        " · "
    };
    let mut lines = Vec::new();
    if extensions.is_empty() {
        lines.push(Line::styled(
            "No MCP servers, skills, plugins, or hooks are configured.",
            Style::default().fg(theme.foreground_muted),
        ));
    } else {
        let available = usize::from(area.height)
            .saturating_sub(usize::from(picker.error.is_some()))
            .max(1);
        let selected = picker.selected.min(extensions.len().saturating_sub(1));
        for (index, extension) in extensions.iter().enumerate().take(available) {
            lines.push(row(
                index == selected,
                format!("{}{separator}{}", extension.category, extension.name),
                extension.status.clone(),
                glyph_mode,
                theme,
            ));
        }
    }
    if let Some(error) = picker.error.as_deref() {
        lines.push(Line::styled(error, Style::default().fg(theme.error)));
    }
    paint(frame, area, lines, theme);
}

pub const THEMES: [ThemeKind; 4] = [
    ThemeKind::MitsuroDark,
    ThemeKind::MitsuroLight,
    ThemeKind::TerminalAdaptive,
    ThemeKind::HighContrast,
];
pub const MOTION: [MotionPreference; 3] = [
    MotionPreference::Full,
    MotionPreference::Reduced,
    MotionPreference::Off,
];

pub fn render_appearance(
    frame: &mut Frame,
    area: Rect,
    picker: &PickerUiState,
    selected_theme: ThemeKind,
    selected_motion: MotionPreference,
    glyph_mode: GlyphMode,
    theme: SemanticTheme,
) {
    let mut lines = vec![Line::styled(
        "theme",
        Style::default().fg(theme.foreground_muted),
    )];
    for (index, value) in THEMES.iter().enumerate() {
        lines.push(row(
            picker.selected == index,
            format!("{value:?}"),
            if *value == selected_theme {
                "selected"
            } else {
                ""
            }
            .to_owned(),
            glyph_mode,
            theme,
        ));
    }
    lines.push(Line::default());
    lines.push(Line::styled(
        "motion",
        Style::default().fg(theme.foreground_muted),
    ));
    for (index, value) in MOTION.iter().enumerate() {
        lines.push(row(
            picker.selected == THEMES.len() + index,
            format!("{value:?}"),
            if *value == selected_motion {
                "selected"
            } else {
                ""
            }
            .to_owned(),
            glyph_mode,
            theme,
        ));
    }
    paint(frame, area, lines, theme);
}

fn paint(frame: &mut Frame, area: Rect, lines: Vec<Line<'_>>, theme: SemanticTheme) {
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().fg(theme.foreground).bg(theme.surface)),
        area,
    );
}
