use gpui::{actions, App, KeyBinding};

pub const WORKSPACE_KEY_CONTEXT: &str = "KrustyWorkspace";

actions!(krusty_panels, [SwapFocusedPanel, ToggleFocusedPanelAxis]);

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-j", SwapFocusedPanel, Some(WORKSPACE_KEY_CONTEXT)),
        KeyBinding::new(
            "ctrl-k",
            ToggleFocusedPanelAxis,
            Some(WORKSPACE_KEY_CONTEXT),
        ),
    ]);
}
