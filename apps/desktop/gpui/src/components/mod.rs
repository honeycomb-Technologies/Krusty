//! Shell chrome pieces (Codex-like layout).

mod activity_rail;
mod approval_bar;
mod browser_panel;
mod codex_button;
mod composer;
mod computer_panel;
mod extensions_panel;
mod feedback_dialog;
mod files_panel;
mod guardian_dialog;
mod main_column;
mod markdown;
mod pull_requests_panel;
mod scheduled_panel;
mod server_request_bar;
mod settings_panel;
mod sidebar;
mod sites_panel;
mod terminal_panel;
mod work_panel;
// status_bar removed: full-width IDE strip under rail is not Codex chat density.
// Connection chip + status text live in main column title bar.

pub use activity_rail::activity_rail;
pub use approval_bar::approval_bar;
pub use browser_panel::browser_panel;
pub use composer::composer;
pub use computer_panel::computer_panel;
pub use extensions_panel::extensions_panel;
pub use feedback_dialog::feedback_dialog;
pub use files_panel::files_panel;
pub use guardian_dialog::guardian_dialog;
pub use main_column::main_column;
pub use pull_requests_panel::pull_requests_panel;
pub use scheduled_panel::scheduled_panel;
pub use settings_panel::settings_panel;
pub use sidebar::sidebar;
pub use sites_panel::sites_panel;
pub use terminal_panel::terminal_panel;
pub use work_panel::work_panel;
