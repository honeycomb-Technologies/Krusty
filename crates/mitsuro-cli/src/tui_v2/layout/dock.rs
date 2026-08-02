//! Workspace dock split: plan band above, plugin well below.

use ratatui::layout::Rect;

/// Minimum rows for the plan panel (including border).
pub const PLAN_MIN_HEIGHT: u16 = 6;
/// Compressed plan height while the plugin well is focused.
pub const PLAN_COMPRESSED_HEIGHT: u16 = 5;
/// Minimum rows for the plugin well (including border).
pub const PLUGIN_MIN_HEIGHT: u16 = 8;
/// Default fraction of dock height reserved for plan (before mins).
pub const DEFAULT_PLAN_RATIO: f32 = 0.42;
/// One blank cell between the two title-less purple frames.
pub const DOCK_GAP: u16 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DockSplit {
    pub plan: Rect,
    pub gap: Option<Rect>,
    pub plugin: Rect,
}

/// Split the inspector column into plan + plugin twin panels.
pub fn split_dock(inspector: Rect, plan_ratio: f32, plugin_focused: bool) -> Option<DockSplit> {
    if inspector.width < 8 || inspector.height < PLAN_MIN_HEIGHT.saturating_add(2) {
        return None;
    }

    let gap = if inspector.height
        > PLAN_MIN_HEIGHT
            .saturating_add(PLUGIN_MIN_HEIGHT)
            .saturating_add(DOCK_GAP)
    {
        DOCK_GAP
    } else {
        0
    };
    let usable = inspector.height.saturating_sub(gap);
    if usable < PLAN_MIN_HEIGHT {
        return None;
    }

    let ratio = plan_ratio.clamp(0.2, 0.8);
    let mut plan_height = ((f32::from(usable) * ratio).round() as u16)
        .clamp(PLAN_MIN_HEIGHT, usable.saturating_sub(1));

    if plugin_focused {
        plan_height = PLAN_COMPRESSED_HEIGHT.min(plan_height).max(4);
    }

    // Prefer a real plugin well when height allows.
    if usable > plan_height.saturating_add(PLUGIN_MIN_HEIGHT.saturating_sub(1)) {
        let plugin_height = usable.saturating_sub(plan_height);
        if plugin_height < 3 {
            // Collapse to plan-only dock.
            return Some(DockSplit {
                plan: inspector,
                gap: None,
                plugin: Rect::new(inspector.x, inspector.bottom(), inspector.width, 0),
            });
        }
        let plan = Rect::new(inspector.x, inspector.y, inspector.width, plan_height);
        let gap_rect = if gap > 0 {
            Some(Rect::new(inspector.x, plan.bottom(), inspector.width, gap))
        } else {
            None
        };
        let plugin_y = plan.bottom().saturating_add(gap);
        let plugin = Rect::new(
            inspector.x,
            plugin_y,
            inspector.width,
            inspector.bottom().saturating_sub(plugin_y),
        );
        return Some(DockSplit {
            plan,
            gap: gap_rect,
            plugin,
        });
    }

    // Short dock: plan only.
    Some(DockSplit {
        plan: inspector,
        gap: None,
        plugin: Rect::new(inspector.x, inspector.bottom(), inspector.width, 0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twin_panels_respect_mins_and_gap() {
        let split = split_dock(Rect::new(100, 2, 40, 36), DEFAULT_PLAN_RATIO, false).expect("dock");
        assert!(split.plan.height >= PLAN_MIN_HEIGHT);
        assert!(split.plugin.height >= PLUGIN_MIN_HEIGHT || split.plugin.height == 0);
        if let Some(gap) = split.gap {
            assert_eq!(gap.height, DOCK_GAP);
            assert_eq!(split.plan.bottom(), gap.y);
            assert_eq!(gap.bottom(), split.plugin.y);
        }
        assert_eq!(split.plugin.bottom(), 2 + 36);
    }

    #[test]
    fn plugin_focus_compresses_plan_band() {
        let open = split_dock(Rect::new(0, 0, 40, 36), 0.5, false).expect("open");
        let focused = split_dock(Rect::new(0, 0, 40, 36), 0.5, true).expect("focused");
        assert!(focused.plan.height <= open.plan.height);
        assert!(focused.plan.height <= PLAN_COMPRESSED_HEIGHT);
        assert!(focused.plugin.height >= open.plugin.height);
    }
}
